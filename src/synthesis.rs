use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fs::{self, File};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use libloading::Library;
use ndarray::{Array2, Array3, ArrayD, Axis, Ix2, Ix3};
use ndarray_npy::NpzReader;
use ort::session::Session;
use ort::value::{Tensor, TensorElementType};
use ort::{self, ep};

use crate::kokoro_vocab;

const ESPEAK_AUDIO_OUTPUT_SYNCHRONOUS: c_int = 2;
const ESPEAK_ERROR_OK: c_int = 0;
const ESPEAK_CHARS_UTF8: c_int = 1;
const ESPEAK_PHONEMES_IPA: c_int = 0x02;
const ESPEAK_INITIALIZE_DONT_EXIT: c_int = 0x8000;
const ORT_DYLIB_PATH_ENV: &str = "ORT_DYLIB_PATH";

static ONNXRUNTIME_INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Result of a successful TTS synthesis.
#[derive(Debug)]
pub struct SynthResult {
    /// Raw PCM16 LE samples (mono).
    pub pcm16: Vec<i16>,
    /// Sample rate reported by the backend.
    pub sample_rate: u32,
}

/// Kokoro voice metadata inferred from the requested voice identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedVoice {
    pub voice_id: String,
    pub lang_code: &'static str,
    pub british: bool,
    pub espeak_voice: Option<&'static str>,
}

/// Result of a successful phonemization pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhonemeResult {
    pub phonemes: String,
    pub lang_code: &'static str,
}

/// Exact Kokoro runtime assets resolved from a request.
#[derive(Debug, PartialEq, Eq)]
pub struct ResolvedModelAssets {
    /// `kokoro-*.onnx` model file.
    pub model_path: PathBuf,
    /// `voices*.bin` voices bundle.
    pub voices_path: PathBuf,
    /// Voice metadata inferred from the Kokoro voice ID.
    pub voice: ResolvedVoice,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RuntimeKey {
    model_path: PathBuf,
    voices_path: PathBuf,
}

impl RuntimeKey {
    fn from_assets(assets: &ResolvedModelAssets) -> Self {
        Self {
            model_path: assets.model_path.clone(),
            voices_path: assets.voices_path.clone(),
        }
    }
}

trait Phonemizer: Send {
    fn phonemize(&mut self, text: &str, voice: &ResolvedVoice) -> Result<PhonemeResult, String>;
}

trait RuntimeFactory: Send {
    fn load(&self, assets: &ResolvedModelAssets) -> Result<Box<dyn KokoroRuntime>, String>;
}

trait KokoroRuntime: Send {
    fn synthesize_chunk(
        &mut self,
        voice_id: &str,
        token_ids: &[i64],
        speed: f32,
    ) -> Result<Vec<f32>, String>;
}

/// Execution provider selected at startup. CPU is the default and release floor;
/// DirectML is an explicit, Windows-only opt-in (see `AGENTS.md §10.2`).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ExecutionProvider {
    Cpu,
    DirectMl,
}

/// Process-owned synthesis state.
pub struct SynthesisCache {
    phonemizer: Box<dyn Phonemizer>,
    runtime_factory: Box<dyn RuntimeFactory>,
    runtimes: HashMap<RuntimeKey, Box<dyn KokoroRuntime>>,
}

impl SynthesisCache {
    pub fn new(espeak_data_dir: PathBuf, execution_provider: ExecutionProvider) -> Self {
        Self::with_components(
            Box::new(EspeakPhonemizer::new(espeak_data_dir)),
            Box::new(OrtRuntimeFactory { execution_provider }),
        )
    }

    fn with_components(
        phonemizer: Box<dyn Phonemizer>,
        runtime_factory: Box<dyn RuntimeFactory>,
    ) -> Self {
        Self {
            phonemizer,
            runtime_factory,
            runtimes: HashMap::new(),
        }
    }

    pub fn synthesize(
        &mut self,
        text: &str,
        assets: &ResolvedModelAssets,
        speed: f32,
    ) -> Result<SynthResult, String> {
        let phonemize_start = Instant::now();
        let phonemes = self.phonemizer.phonemize(text, &assets.voice)?;
        let phonemize_elapsed = phonemize_start.elapsed();
        let phoneme_chunks = split_phonemes_for_inference(&phonemes.phonemes);
        if phoneme_chunks.is_empty() {
            return Err(format!(
                "No Kokoro-compatible phonemes remained for voice '{}'",
                assets.voice.voice_id
            ));
        }

        tracing::debug!(
            event = "phonemization_succeeded",
            voice = assets.voice.voice_id.as_str(),
            lang_code = phonemes.lang_code,
            phoneme_len = phonemes.phonemes.chars().count(),
            chunk_count = phoneme_chunks.len()
        );
        tracing::info!(
            event = "phonemization_done",
            voice = assets.voice.voice_id.as_str(),
            duration_ms = phonemize_elapsed.as_millis() as u64
        );

        let voice_id = assets.voice.voice_id.clone();
        let chunk_count = phoneme_chunks.len();
        let runtime = self.runtime_mut(assets)?;
        let mut samples = Vec::new();

        let inference_start = Instant::now();
        for chunk in phoneme_chunks {
            let token_ids = tokenize_phonemes(&chunk)?;
            let audio_chunk = runtime.synthesize_chunk(&voice_id, &token_ids, speed)?;
            samples.extend(audio_chunk);
        }
        let inference_elapsed = inference_start.elapsed();
        tracing::info!(
            event = "inference_done",
            voice = voice_id.as_str(),
            chunk_count = chunk_count,
            duration_ms = inference_elapsed.as_millis() as u64
        );

        Ok(SynthResult {
            pcm16: float_audio_to_pcm16(&samples),
            sample_rate: kokoro_vocab::SAMPLE_RATE,
        })
    }

    fn runtime_mut(
        &mut self,
        assets: &ResolvedModelAssets,
    ) -> Result<&mut (dyn KokoroRuntime + '_), String> {
        let key = RuntimeKey::from_assets(assets);
        if !self.runtimes.contains_key(&key) {
            tracing::debug!(
                event = "kokoro_runtime_cache_miss",
                model_path = assets.model_path.display().to_string(),
                voices_path = assets.voices_path.display().to_string()
            );
            let load_start = Instant::now();
            let runtime = self.runtime_factory.load(assets)?;
            let load_elapsed = load_start.elapsed();
            self.runtimes.insert(key.clone(), runtime);
            tracing::info!(
                event = "model_loaded",
                duration_ms = load_elapsed.as_millis() as u64
            );
        } else {
            tracing::debug!(
                event = "kokoro_runtime_cache_hit",
                model_path = assets.model_path.display().to_string(),
                voices_path = assets.voices_path.display().to_string()
            );
        }

        match self.runtimes.get_mut(&key) {
            Some(runtime) => Ok(runtime.as_mut()),
            None => Err("Kokoro runtime cache returned no runtime after insertion".to_string()),
        }
    }
}

struct EspeakPhonemizer {
    runtime_dir: PathBuf,
    api: Option<EspeakApi>,
}

impl EspeakPhonemizer {
    fn new(runtime_dir: PathBuf) -> Self {
        Self {
            runtime_dir,
            api: None,
        }
    }

    fn api(&mut self) -> Result<&mut EspeakApi, String> {
        if self.api.is_none() {
            self.api = Some(EspeakApi::load(&self.runtime_dir)?);
        }

        self.api
            .as_mut()
            .ok_or_else(|| "eSpeak runtime did not initialize".to_string())
    }
}

impl Phonemizer for EspeakPhonemizer {
    fn phonemize(&mut self, text: &str, voice: &ResolvedVoice) -> Result<PhonemeResult, String> {
        let Some(espeak_voice) = voice.espeak_voice else {
            return Err(format!(
                "phonemization is not implemented yet for lang_code='{}'",
                voice.lang_code
            ));
        };

        let raw = self.api()?.text_to_ipa(text, espeak_voice)?;
        let normalized = if voice.lang_code == "a" || voice.lang_code == "b" {
            normalize_english_phonemes(&raw, voice.british)
        } else {
            normalize_non_english_phonemes(&raw)
        };
        let phonemes = filter_phonemes_to_vocab(&normalized);
        if phonemes.trim().is_empty() {
            return Err(format!(
                "eSpeak produced no Kokoro-compatible phonemes for voice '{}'",
                voice.voice_id
            ));
        }

        Ok(PhonemeResult {
            phonemes,
            lang_code: voice.lang_code,
        })
    }
}

struct EspeakApi {
    _library: Library,
    set_voice_by_name: unsafe extern "C" fn(*const c_char) -> c_int,
    text_to_phonemes: unsafe extern "C" fn(*mut *const c_void, c_int, c_int) -> *const c_char,
    terminate: unsafe extern "C" fn() -> c_int,
}

impl EspeakApi {
    fn load(runtime_dir: &Path) -> Result<Self, String> {
        let library_path = resolve_espeak_library_path(runtime_dir)?;
        let data_root = CString::new(runtime_dir.to_string_lossy().as_bytes()).map_err(|_| {
            format!(
                "Cannot use eSpeak runtime '{}': path contains an interior NUL byte",
                runtime_dir.display()
            )
        })?;

        unsafe {
            let library = Library::new(&library_path).map_err(|error| {
                format!(
                    "Cannot load eSpeak shared library '{}': {}",
                    library_path.display(),
                    error
                )
            })?;

            let initialize: unsafe extern "C" fn(c_int, c_int, *const c_char, c_int) -> c_int =
                *library.get(b"espeak_Initialize\0").map_err(|error| {
                    format!(
                        "Cannot load symbol 'espeak_Initialize' from '{}': {}",
                        library_path.display(),
                        error
                    )
                })?;
            let set_voice_by_name: unsafe extern "C" fn(*const c_char) -> c_int =
                *library.get(b"espeak_SetVoiceByName\0").map_err(|error| {
                    format!(
                        "Cannot load symbol 'espeak_SetVoiceByName' from '{}': {}",
                        library_path.display(),
                        error
                    )
                })?;
            let text_to_phonemes: unsafe extern "C" fn(
                *mut *const c_void,
                c_int,
                c_int,
            ) -> *const c_char = *library.get(b"espeak_TextToPhonemes\0").map_err(|error| {
                format!(
                    "Cannot load symbol 'espeak_TextToPhonemes' from '{}': {}",
                    library_path.display(),
                    error
                )
            })?;
            let terminate: unsafe extern "C" fn() -> c_int =
                *library.get(b"espeak_Terminate\0").map_err(|error| {
                    format!(
                        "Cannot load symbol 'espeak_Terminate' from '{}': {}",
                        library_path.display(),
                        error
                    )
                })?;

            let sample_rate = initialize(
                ESPEAK_AUDIO_OUTPUT_SYNCHRONOUS,
                0,
                data_root.as_ptr(),
                ESPEAK_INITIALIZE_DONT_EXIT,
            );
            if sample_rate < 0 {
                return Err(format!(
                    "Cannot initialize eSpeak runtime '{}' using '{}'",
                    runtime_dir.display(),
                    library_path.display()
                ));
            }

            Ok(Self {
                _library: library,
                set_voice_by_name,
                text_to_phonemes,
                terminate,
            })
        }
    }

    fn text_to_ipa(&mut self, text: &str, voice_name: &str) -> Result<String, String> {
        let voice_name = CString::new(voice_name).map_err(|_| {
            format!(
                "Cannot select eSpeak voice '{}': voice name contains an interior NUL byte",
                voice_name
            )
        })?;
        let text = CString::new(text).map_err(|_| {
            "Cannot phonemize request text: text contains an interior NUL byte".to_string()
        })?;

        unsafe {
            let voice_result = (self.set_voice_by_name)(voice_name.as_ptr());
            if voice_result != ESPEAK_ERROR_OK {
                return Err(format!(
                    "Cannot select eSpeak voice '{}' (error={voice_result})",
                    voice_name.to_string_lossy()
                ));
            }

            let mut text_ptr = text.as_ptr() as *const c_void;
            let mut segments = Vec::new();

            loop {
                let previous_ptr = text_ptr;
                let chunk_ptr = (self.text_to_phonemes)(
                    &mut text_ptr as *mut *const c_void,
                    ESPEAK_CHARS_UTF8,
                    ESPEAK_PHONEMES_IPA,
                );

                if chunk_ptr.is_null() {
                    break;
                }

                let chunk = CStr::from_ptr(chunk_ptr).to_string_lossy().into_owned();
                if !chunk.is_empty() {
                    segments.push(chunk);
                }

                if text_ptr.is_null() {
                    break;
                }

                if text_ptr == previous_ptr {
                    return Err("eSpeak phonemizer did not advance the input pointer".to_string());
                }
            }

            Ok(segments.join(" "))
        }
    }
}

impl Drop for EspeakApi {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.terminate)();
        }
    }
}

struct OrtRuntimeFactory {
    execution_provider: ExecutionProvider,
}

impl RuntimeFactory for OrtRuntimeFactory {
    fn load(&self, assets: &ResolvedModelAssets) -> Result<Box<dyn KokoroRuntime>, String> {
        let runtime = OrtKokoroRuntime::load(assets, self.execution_provider)?;
        runtime.ensure_voice_present(&assets.voice.voice_id)?;
        Ok(Box::new(runtime))
    }
}

struct OrtKokoroRuntime {
    model_path: PathBuf,
    voices_path: PathBuf,
    session: Session,
    input_config: RuntimeInputConfig,
    voice_styles: HashMap<String, Array2<f32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpeedInputKind {
    Float32,
    Int32,
    Int64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StyleInputKind {
    Vector,
    BatchVector,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeInputConfig {
    token_input_name: String,
    style_input_name: String,
    speed_input_name: String,
    output_name: String,
    speed_input_kind: SpeedInputKind,
    style_input_kind: StyleInputKind,
}

impl RuntimeInputConfig {
    fn from_session(session: &Session) -> Result<Self, String> {
        let mut tokens_name: Option<String> = None;
        let mut input_ids_name: Option<String> = None;
        let mut style_input_name: Option<String> = None;
        let mut speed_input_name: Option<String> = None;
        let mut speed_input_kind: Option<SpeedInputKind> = None;
        let mut style_input_kind: Option<StyleInputKind> = None;

        for input in session.inputs() {
            match input.name() {
                "tokens" => {
                    tokens_name = Some(input.name().to_string());
                }
                "input_ids" => {
                    input_ids_name = Some(input.name().to_string());
                }
                "style" => {
                    style_input_name = Some(input.name().to_string());
                    style_input_kind = Some(detect_style_input_kind(input)?);
                }
                "speed" => {
                    speed_input_name = Some(input.name().to_string());
                    speed_input_kind = Some(detect_speed_input_kind(input)?);
                }
                _ => {}
            }
        }

        let token_input_name = input_ids_name.or(tokens_name).ok_or_else(|| {
            let inputs = session
                .inputs()
                .iter()
                .map(|input| input.name().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Unsupported Kokoro model inputs: expected 'input_ids' or 'tokens', found [{}]",
                inputs
            )
        })?;
        let style_input_name = style_input_name
            .ok_or_else(|| "Unsupported Kokoro model: missing 'style' input".to_string())?;
        let speed_input_name = speed_input_name
            .ok_or_else(|| "Unsupported Kokoro model: missing 'speed' input".to_string())?;
        let speed_input_kind = speed_input_kind.ok_or_else(|| {
            "Unsupported Kokoro model: cannot determine 'speed' input type".to_string()
        })?;
        let style_input_kind = style_input_kind.ok_or_else(|| {
            "Unsupported Kokoro model: cannot determine 'style' input shape".to_string()
        })?;
        let output_name = session
            .outputs()
            .first()
            .map(|output| output.name().to_string())
            .ok_or_else(|| "Unsupported Kokoro model: model has no outputs".to_string())?;

        Ok(Self {
            token_input_name,
            style_input_name,
            speed_input_name,
            output_name,
            speed_input_kind,
            style_input_kind,
        })
    }
}

impl OrtKokoroRuntime {
    fn load(
        assets: &ResolvedModelAssets,
        execution_provider: ExecutionProvider,
    ) -> Result<Self, String> {
        ensure_onnxruntime_loaded()?;

        // NO silent fallback to CPU on DirectML registration failure: per
        // `AGENTS.md §10.2`, DirectML errors surface to the host through the
        // normal "Synthesis failed:" envelope. Do not add a fallback here.
        let providers = match execution_provider {
            ExecutionProvider::Cpu => vec![ep::CPU::default().build()],
            ExecutionProvider::DirectMl => {
                #[cfg(target_os = "windows")]
                {
                    vec![ep::DirectML::default().with_device_id(0).build()]
                }
                #[cfg(not(target_os = "windows"))]
                {
                    unreachable!("directml rejected at startup on non-Windows");
                }
            }
        };
        let ep_label = match execution_provider {
            ExecutionProvider::Cpu => "CPU",
            ExecutionProvider::DirectMl => "DirectML",
        };

        let session = Session::builder()
            .map_err(|error| format!("Cannot create ONNX Runtime session builder: {error}"))?
            .with_execution_providers(providers)
            .map_err(|error| {
                format!("Cannot configure Kokoro {ep_label} execution provider: {error}")
            })?
            .commit_from_file(&assets.model_path)
            .map_err(|error| {
                format!(
                    "Cannot load Kokoro model '{}': {}",
                    assets.model_path.display(),
                    error
                )
            })?;

        let input_config = RuntimeInputConfig::from_session(&session)?;
        let voice_styles = load_voice_styles(&assets.voices_path)?;

        tracing::debug!(
            event = "kokoro_runtime_loaded",
            model_path = assets.model_path.display().to_string(),
            voices_path = assets.voices_path.display().to_string(),
            token_input = input_config.token_input_name.as_str(),
            style_input = input_config.style_input_name.as_str(),
            speed_input = input_config.speed_input_name.as_str(),
            output_name = input_config.output_name.as_str()
        );

        Ok(Self {
            model_path: assets.model_path.clone(),
            voices_path: assets.voices_path.clone(),
            session,
            input_config,
            voice_styles,
        })
    }

    fn ensure_voice_present(&self, voice_id: &str) -> Result<(), String> {
        if self.voice_styles.contains_key(voice_id) {
            Ok(())
        } else {
            Err(format!(
                "Kokoro voice '{}' is not present in voices bundle '{}'",
                voice_id,
                self.voices_path.display()
            ))
        }
    }

    fn style_row_for_tokens(&self, voice_id: &str, token_count: usize) -> Result<Vec<f32>, String> {
        let style_matrix = self.voice_styles.get(voice_id).ok_or_else(|| {
            format!(
                "Kokoro voice '{}' is not present in voices bundle '{}'",
                voice_id,
                self.voices_path.display()
            )
        })?;

        if token_count >= style_matrix.nrows() {
            return Err(format!(
                "Kokoro voice '{}' in '{}' does not provide a style row for token_count={token_count} (available rows: {})",
                voice_id,
                self.voices_path.display(),
                style_matrix.nrows()
            ));
        }

        Ok(style_matrix.row(token_count).to_vec())
    }
}

impl KokoroRuntime for OrtKokoroRuntime {
    fn synthesize_chunk(
        &mut self,
        voice_id: &str,
        token_ids: &[i64],
        speed: f32,
    ) -> Result<Vec<f32>, String> {
        if token_ids.is_empty() {
            return Err(format!(
                "Cannot run Kokoro inference for voice '{}': token sequence is empty",
                voice_id
            ));
        }

        let style_row = self.style_row_for_tokens(voice_id, token_ids.len())?;
        let mut padded_tokens = Vec::with_capacity(token_ids.len() + 2);
        padded_tokens.push(0_i64);
        padded_tokens.extend_from_slice(token_ids);
        padded_tokens.push(0_i64);

        let token_tensor =
            Tensor::from_array((vec![1_i64, padded_tokens.len() as i64], padded_tokens))
                .map_err(|error| format!("Cannot build Kokoro token tensor: {error}"))?;

        let style_tensor = match self.input_config.style_input_kind {
            StyleInputKind::Vector => {
                Tensor::from_array((vec![style_row.len() as i64], style_row.clone()))
            }
            StyleInputKind::BatchVector => {
                Tensor::from_array((vec![1_i64, style_row.len() as i64], style_row.clone()))
            }
        }
        .map_err(|error| format!("Cannot build Kokoro style tensor: {error}"))?;

        let outputs = match self.input_config.speed_input_kind {
            SpeedInputKind::Float32 => {
                let speed_tensor = Tensor::from_array((vec![1_i64], vec![speed]))
                    .map_err(|error| format!("Cannot build Kokoro speed tensor: {error}"))?;
                self.session.run(ort::inputs![
                    self.input_config.token_input_name.as_str() => token_tensor,
                    self.input_config.style_input_name.as_str() => style_tensor,
                    self.input_config.speed_input_name.as_str() => speed_tensor,
                ])
            }
            SpeedInputKind::Int32 => {
                let speed_tensor = Tensor::from_array((vec![1_i64], vec![speed.round() as i32]))
                    .map_err(|error| format!("Cannot build Kokoro speed tensor: {error}"))?;
                self.session.run(ort::inputs![
                    self.input_config.token_input_name.as_str() => token_tensor,
                    self.input_config.style_input_name.as_str() => style_tensor,
                    self.input_config.speed_input_name.as_str() => speed_tensor,
                ])
            }
            SpeedInputKind::Int64 => {
                let speed_tensor = Tensor::from_array((vec![1_i64], vec![speed.round() as i64]))
                    .map_err(|error| format!("Cannot build Kokoro speed tensor: {error}"))?;
                self.session.run(ort::inputs![
                    self.input_config.token_input_name.as_str() => token_tensor,
                    self.input_config.style_input_name.as_str() => style_tensor,
                    self.input_config.speed_input_name.as_str() => speed_tensor,
                ])
            }
        }
        .map_err(|error| {
            format!(
                "Kokoro ONNX inference failed for voice '{}' using model '{}': {}",
                voice_id,
                self.model_path.display(),
                error
            )
        })?;

        let output = outputs
            .get(self.input_config.output_name.as_str())
            .unwrap_or(&outputs[0]);
        let (_, audio) = output.try_extract_tensor::<f32>().map_err(|error| {
            format!(
                "Cannot extract Kokoro audio output '{}' from model '{}': {}",
                self.input_config.output_name,
                self.model_path.display(),
                error
            )
        })?;
        if audio.is_empty() {
            return Err(format!(
                "Kokoro model '{}' returned no audio samples for voice '{}'",
                self.model_path.display(),
                voice_id
            ));
        }

        Ok(audio.to_vec())
    }
}

/// Validate the process-scoped eSpeak runtime directory.
pub fn validate_espeak_data_dir(data_dir: &Path) -> Result<(), String> {
    if !data_dir.is_absolute() {
        return Err(format!(
            "Invalid eSpeak data dir '{}': path must be absolute",
            data_dir.display()
        ));
    }

    let metadata = std::fs::metadata(data_dir).map_err(|error| {
        format!(
            "Cannot use eSpeak data dir '{}': {}",
            data_dir.display(),
            error
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "Cannot use eSpeak data dir '{}': path is not a directory",
            data_dir.display()
        ));
    }

    let espeak_ng_data_dir = data_dir.join("espeak-ng-data");
    let metadata = std::fs::metadata(&espeak_ng_data_dir).map_err(|error| {
        format!(
            "Invalid eSpeak data dir '{}': missing 'espeak-ng-data' directory ({})",
            data_dir.display(),
            error
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "Invalid eSpeak data dir '{}': '{}' is not a directory",
            data_dir.display(),
            espeak_ng_data_dir.display()
        ));
    }

    Ok(())
}

/// Validate the request-scoped Kokoro asset bundle directory.
pub fn validate_model_dir(model_dir: &Path) -> Result<(), String> {
    if !model_dir.is_absolute() {
        return Err(format!(
            "Invalid model_dir '{}': path must be absolute",
            model_dir.display()
        ));
    }

    let metadata = match std::fs::metadata(model_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "Cannot use model_dir '{}': path does not exist",
                model_dir.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "Cannot use model_dir '{}': {}",
                model_dir.display(),
                error
            ));
        }
    };

    if !metadata.is_dir() {
        return Err(format!(
            "Cannot use model_dir '{}': path is not a directory",
            model_dir.display()
        ));
    }

    Ok(())
}

/// Resolve the exact Kokoro runtime assets for the requested bundle.
pub fn resolve_model_assets(
    model_dir: &Path,
    voice_id: &str,
) -> Result<ResolvedModelAssets, String> {
    validate_model_dir(model_dir)?;

    let voice = resolve_voice_profile(voice_id)?;
    let model_path = find_single_matching_file(
        model_dir,
        |name| name.ends_with(".onnx"),
        "Kokoro model (*.onnx)",
    )?;
    let voices_path = find_single_matching_file(
        model_dir,
        |name| name.starts_with("voices") && name.ends_with(".bin"),
        "Kokoro voices bundle (voices*.bin)",
    )?;

    Ok(ResolvedModelAssets {
        model_path,
        voices_path,
        voice,
    })
}

fn resolve_voice_profile(voice_id: &str) -> Result<ResolvedVoice, String> {
    let trimmed = voice_id.trim();
    if trimmed.is_empty() {
        return Err("Invalid voice: voice must not be empty or whitespace".to_string());
    }

    let (lang_code, british, espeak_voice) = if trimmed.starts_with("af_")
        || trimmed.starts_with("am_")
    {
        ("a", false, Some("en-us"))
    } else if trimmed.starts_with("bf_") || trimmed.starts_with("bm_") {
        ("b", true, Some("en"))
    } else if trimmed.starts_with("jf_") || trimmed.starts_with("jm_") {
        ("j", false, None)
    } else if trimmed.starts_with("zf_") || trimmed.starts_with("zm_") {
        ("z", false, None)
    } else if trimmed.starts_with("ef_") || trimmed.starts_with("em_") {
        ("e", false, Some("es"))
    } else if trimmed.starts_with("ff_") {
        ("f", false, Some("fr"))
    } else if trimmed.starts_with("hf_") || trimmed.starts_with("hm_") {
        ("h", false, Some("hi"))
    } else if trimmed.starts_with("if_") || trimmed.starts_with("im_") {
        ("i", false, Some("it"))
    } else if trimmed.starts_with("pf_") || trimmed.starts_with("pm_") {
        ("p", false, Some("pt-br"))
    } else {
        return Err(format!(
            "Unsupported Kokoro voice '{}': expected a known Kokoro voice prefix such as af_, bf_, jf_, zf_, ef_, ff_, hf_, if_, or pf_",
            trimmed
        ));
    };

    Ok(ResolvedVoice {
        voice_id: trimmed.to_string(),
        lang_code,
        british,
        espeak_voice,
    })
}

fn find_single_matching_file<F>(
    model_dir: &Path,
    predicate: F,
    label: &str,
) -> Result<PathBuf, String>
where
    F: Fn(&str) -> bool,
{
    let mut matches: Vec<(String, PathBuf)> = Vec::new();
    let entries = fs::read_dir(model_dir)
        .map_err(|error| format!("Cannot list model_dir '{}': {}", model_dir.display(), error))?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("Cannot list model_dir '{}': {}", model_dir.display(), error)
        })?;

        let file_type = entry.file_type().map_err(|error| {
            format!(
                "Cannot inspect path '{}': {}",
                entry.path().display(),
                error
            )
        })?;

        if !file_type.is_file() {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        if predicate(&file_name) {
            matches.push((file_name, entry.path()));
        }
    }

    matches.sort_by(|left, right| left.0.cmp(&right.0));

    match matches.len() {
        1 => Ok(matches.remove(0).1),
        0 => Err(format!(
            "Cannot use model_dir '{}': expected exactly one {}",
            model_dir.display(),
            label
        )),
        _ => {
            let file_names = matches
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "Cannot use model_dir '{}': expected exactly one {}, found {}",
                model_dir.display(),
                label,
                file_names
            ))
        }
    }
}

fn load_voice_styles(voices_path: &Path) -> Result<HashMap<String, Array2<f32>>, String> {
    let file = File::open(voices_path).map_err(|error| {
        format!(
            "Cannot open Kokoro voices bundle '{}': {}",
            voices_path.display(),
            error
        )
    })?;
    let mut npz = NpzReader::new(file).map_err(|error| {
        format!(
            "Cannot read Kokoro voices bundle '{}': {}",
            voices_path.display(),
            error
        )
    })?;

    let names = npz.names().map_err(|error| {
        format!(
            "Cannot list voices in bundle '{}': {}",
            voices_path.display(),
            error
        )
    })?;
    if names.is_empty() {
        return Err(format!(
            "Cannot use Kokoro voices bundle '{}': bundle contains no voices",
            voices_path.display()
        ));
    }

    let mut styles = HashMap::new();
    for name in names {
        let array: ArrayD<f32> = npz.by_name(&name).map_err(|error| {
            format!(
                "Cannot read voice '{}' from bundle '{}': {}",
                name,
                voices_path.display(),
                error
            )
        })?;
        let matrix = voice_style_matrix_from_array(array, &name, voices_path)?;
        if matrix.nrows() == 0 || matrix.ncols() == 0 {
            return Err(format!(
                "Cannot use voice '{}' from bundle '{}': style matrix must not be empty",
                name,
                voices_path.display()
            ));
        }
        styles.insert(name, matrix);
    }

    Ok(styles)
}

fn voice_style_matrix_from_array(
    array: ArrayD<f32>,
    voice_name: &str,
    voices_path: &Path,
) -> Result<Array2<f32>, String> {
    if let Ok(matrix) = array.clone().into_dimensionality::<Ix2>() {
        return Ok(matrix);
    }

    if let Ok(tensor) = array.into_dimensionality::<Ix3>() {
        return squeeze_voice_style_tensor(tensor, voice_name, voices_path);
    }

    Err(format!(
        "Cannot use voice '{}' from bundle '{}': expected a 2D style matrix or a 3D tensor with singleton axis 1",
        voice_name,
        voices_path.display()
    ))
}

fn squeeze_voice_style_tensor(
    tensor: Array3<f32>,
    voice_name: &str,
    voices_path: &Path,
) -> Result<Array2<f32>, String> {
    if tensor.shape()[1] != 1 {
        return Err(format!(
            "Cannot use voice '{}' from bundle '{}': expected singleton axis 1 in 3D style tensor, found shape {:?}",
            voice_name,
            voices_path.display(),
            tensor.shape()
        ));
    }

    Ok(tensor.index_axis_move(Axis(1), 0))
}

fn detect_style_input_kind(input: &ort::value::Outlet) -> Result<StyleInputKind, String> {
    let rank = input
        .dtype()
        .tensor_shape()
        .ok_or_else(|| {
            format!(
                "Unsupported Kokoro model input '{}': style input is not a tensor",
                input.name()
            )
        })?
        .len();

    match rank {
        1 => Ok(StyleInputKind::Vector),
        2 => Ok(StyleInputKind::BatchVector),
        _ => Err(format!(
            "Unsupported Kokoro model input '{}': expected style tensor rank 1 or 2, found {}",
            input.name(),
            rank
        )),
    }
}

fn detect_speed_input_kind(input: &ort::value::Outlet) -> Result<SpeedInputKind, String> {
    match input.dtype().tensor_type() {
        Some(TensorElementType::Float32) => Ok(SpeedInputKind::Float32),
        Some(TensorElementType::Int32) => Ok(SpeedInputKind::Int32),
        Some(TensorElementType::Int64) => Ok(SpeedInputKind::Int64),
        Some(other) => Err(format!(
            "Unsupported Kokoro model input '{}': expected speed tensor type f32, i32, or i64, found {:?}",
            input.name(),
            other
        )),
        None => Err(format!(
            "Unsupported Kokoro model input '{}': speed input is not a tensor",
            input.name()
        )),
    }
}

fn ensure_onnxruntime_loaded() -> Result<(), String> {
    let result = ONNXRUNTIME_INIT.get_or_init(|| {
        let library_path = resolve_onnxruntime_library_path()?;
        ort::init_from(&library_path)
            .map_err(|error| {
                format!(
                    "Cannot initialize ONNX Runtime from '{}': {}",
                    library_path.display(),
                    error
                )
            })?
            .with_execution_providers([ep::CPU::default().build()])
            .commit();
        Ok(())
    });

    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(error.clone()),
    }
}

fn resolve_onnxruntime_library_path() -> Result<PathBuf, String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("Cannot locate current executable: {}", error))?;
    resolve_onnxruntime_library_path_for(std::env::var_os(ORT_DYLIB_PATH_ENV), &current_exe)
}

fn resolve_onnxruntime_library_path_for(
    env_value: Option<std::ffi::OsString>,
    current_exe: &Path,
) -> Result<PathBuf, String> {
    match env_value {
        Some(value) if !value.is_empty() && !value.to_string_lossy().trim().is_empty() => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(format!(
                    "Cannot initialize ONNX Runtime: {} must be an absolute path",
                    ORT_DYLIB_PATH_ENV
                ));
            }
            if !path.is_file() {
                return Err(format!(
                    "Cannot initialize ONNX Runtime: '{}' does not exist",
                    path.display()
                ));
            }
            Ok(path)
        }
        _ => {
            let Some(exe_dir) = current_exe.parent() else {
                return Err(format!(
                    "Cannot initialize ONNX Runtime: executable '{}' has no parent directory",
                    current_exe.display()
                ));
            };

            let library_path = exe_dir.join(platform_onnxruntime_library_name());
            if library_path.is_file() {
                Ok(library_path)
            } else {
                Err(format!(
                    "Cannot initialize ONNX Runtime: missing shared library '{}' beside the executable; alternatively set {} to an absolute path",
                    library_path.display(),
                    ORT_DYLIB_PATH_ENV
                ))
            }
        }
    }
}

fn platform_onnxruntime_library_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

fn normalize_english_phonemes(raw: &str, british: bool) -> String {
    let mut phonemes = raw.to_string();

    for (from, to) in english_espeak_replacements() {
        phonemes = phonemes.replace(from, to);
    }

    phonemes = convert_syllabic_consonants(&phonemes);

    if british {
        phonemes = phonemes.replace("A^ə", "ɛː");
        phonemes = phonemes.replace("e^ə", "ɛː");
        phonemes = phonemes.replace("iə", "ɪə");
        phonemes = phonemes.replace("ə^ʊ", "Q");
    } else {
        phonemes = phonemes.replace("o^ʊ", "O");
        phonemes = phonemes.replace("ɜːɹ", "ɜɹ");
        phonemes = phonemes.replace("ɜː", "ɜɹ");
        phonemes = phonemes.replace("ɪə", "iə");
        phonemes = phonemes.replace('ː', "");
    }

    phonemes.trim().to_string()
}

fn normalize_non_english_phonemes(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    let mut previous_was_space = true;

    for ch in raw.chars() {
        let mapped = match ch {
            '\r' | '\n' | '\t' => ' ',
            'g' => 'ɡ',
            _ => ch,
        };

        if mapped.is_whitespace() {
            if !previous_was_space {
                normalized.push(' ');
                previous_was_space = true;
            }
        } else {
            normalized.push(mapped);
            previous_was_space = false;
        }
    }

    normalized.trim().to_string()
}

fn filter_phonemes_to_vocab(text: &str) -> String {
    let vocab = kokoro_vocab::vocab();
    let mut filtered = String::with_capacity(text.len());
    let mut previous_was_space = true;

    for ch in text.chars() {
        let mapped = match ch {
            '\r' | '\n' | '\t' => ' ',
            'g' => 'ɡ',
            _ => ch,
        };

        if !vocab.contains_key(&mapped) {
            continue;
        }

        if mapped == ' ' {
            if previous_was_space {
                continue;
            }
            filtered.push(mapped);
            previous_was_space = true;
            continue;
        }

        filtered.push(mapped);
        previous_was_space = false;
    }

    filtered.trim().to_string()
}

fn tokenize_phonemes(phonemes: &str) -> Result<Vec<i64>, String> {
    let vocab = kokoro_vocab::vocab();
    let mut token_ids = Vec::with_capacity(phonemes.chars().count());

    for ch in phonemes.chars() {
        let token = vocab.get(&ch).ok_or_else(|| {
            format!(
                "No Kokoro token exists for phoneme '{}' (U+{:04X})",
                ch, ch as u32
            )
        })?;
        token_ids.push(*token);
    }

    if token_ids.is_empty() {
        return Err("Kokoro tokenization produced no tokens".to_string());
    }

    Ok(token_ids)
}

fn split_phonemes_for_inference(phonemes: &str) -> Vec<String> {
    let chars: Vec<char> = phonemes.chars().collect();
    let mut start = 0usize;
    let mut chunks = Vec::new();

    while start < chars.len() {
        while start < chars.len() && chars[start].is_whitespace() {
            start += 1;
        }
        if start >= chars.len() {
            break;
        }

        let mut end = (start + kokoro_vocab::MAX_PHONEME_LENGTH).min(chars.len());
        if end < chars.len() {
            if let Some(split) = find_chunk_boundary(&chars, start, end) {
                end = split;
            }
        }

        let chunk = chars[start..end]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        start = end;
    }

    chunks
}

fn find_chunk_boundary(chars: &[char], start: usize, end: usize) -> Option<usize> {
    for index in (start..end).rev() {
        if is_preferred_chunk_boundary(chars[index]) {
            return Some(index + 1);
        }
    }

    for index in (start..end).rev() {
        if chars[index].is_whitespace() {
            return Some(index + 1);
        }
    }

    None
}

fn is_preferred_chunk_boundary(ch: char) -> bool {
    matches!(ch, '.' | ',' | '!' | '?' | ';' | ':' | '—' | '…')
}

fn float_audio_to_pcm16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|sample| {
            let clamped = sample.clamp(-1.0, 1.0);
            if clamped <= -1.0 {
                i16::MIN
            } else if clamped >= 1.0 {
                i16::MAX
            } else {
                (clamped * i16::MAX as f32).round() as i16
            }
        })
        .collect()
}

fn english_espeak_replacements() -> &'static [(&'static str, &'static str)] {
    &[
        ("ʔˌn̩", "tᵊn"),
        ("a^ɪ", "I"),
        ("a^ʊ", "W"),
        ("d^ʒ", "ʤ"),
        ("e^ɪ", "A"),
        ("ɔ^ɪ", "Y"),
        ("ə^l", "ᵊl"),
        ("ʲO", "jO"),
        ("ʲQ", "jQ"),
        ("ɚ", "əɹ"),
        ("ʔn", "tᵊn"),
        ("\u{0303}", ""),
        ("e", "A"),
        ("r", "ɹ"),
        ("t^ʃ", "ʧ"),
        ("x", "k"),
        ("ç", "k"),
        ("ɐ", "ə"),
        ("ɬ", "l"),
        ("ʔ", "t"),
        ("ʲ", ""),
    ]
}

fn convert_syllabic_consonants(text: &str) -> String {
    let mut converted = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if matches!(chars.peek(), Some('\u{0329}')) && !ch.is_whitespace() {
            converted.push('ᵊ');
            converted.push(ch);
            let _ = chars.next();
            continue;
        }

        if ch != '\u{0329}' {
            converted.push(ch);
        }
    }

    converted
}

fn resolve_espeak_library_path(runtime_dir: &Path) -> Result<PathBuf, String> {
    let library_path = runtime_dir.join(platform_espeak_library_name());
    if library_path.is_file() {
        Ok(library_path)
    } else {
        Err(format!(
            "Cannot use eSpeak runtime '{}': missing shared library '{}'",
            runtime_dir.display(),
            library_path.display()
        ))
    }
}

fn platform_espeak_library_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "espeak-ng.dll"
    } else if cfg!(target_os = "macos") {
        "libespeak-ng.dylib"
    } else {
        "libespeak-ng.so"
    }
}

#[cfg(test)]
mod tests {
    use super::{
        filter_phonemes_to_vocab, float_audio_to_pcm16, load_voice_styles,
        normalize_english_phonemes, platform_espeak_library_name,
        platform_onnxruntime_library_name, resolve_espeak_library_path, resolve_model_assets,
        resolve_onnxruntime_library_path_for, split_phonemes_for_inference, tokenize_phonemes,
        validate_espeak_data_dir, validate_model_dir, KokoroRuntime, PhonemeResult,
        ExecutionProvider, ResolvedModelAssets, ResolvedVoice, RuntimeFactory, RuntimeKey,
        SynthesisCache, ORT_DYLIB_PATH_ENV,
    };
    use crate::kokoro_vocab;
    use crate::live_test_support::LiveTestAssets;
    use ndarray::{arr2, Array3};
    use ndarray_npy::NpzWriter;
    use std::fs::{self, File};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    type MockSynthCall = (String, Vec<i64>, f32);
    type SharedSynthCalls = Arc<Mutex<Vec<MockSynthCall>>>;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();

            let path = std::env::temp_dir().join(format!(
                "lingopilot-tts-kokoro-{prefix}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("temp dir should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    struct MockPhonemizer {
        calls: Arc<Mutex<Vec<String>>>,
        result: Result<PhonemeResult, String>,
    }

    impl super::Phonemizer for MockPhonemizer {
        fn phonemize(
            &mut self,
            _text: &str,
            voice: &ResolvedVoice,
        ) -> Result<PhonemeResult, String> {
            self.calls
                .lock()
                .expect("calls should be lockable")
                .push(voice.voice_id.clone());
            self.result.clone()
        }
    }

    struct MockRuntime {
        synth_calls: SharedSynthCalls,
        output: Vec<f32>,
    }

    impl KokoroRuntime for MockRuntime {
        fn synthesize_chunk(
            &mut self,
            voice_id: &str,
            token_ids: &[i64],
            speed: f32,
        ) -> Result<Vec<f32>, String> {
            self.synth_calls
                .lock()
                .expect("calls should be lockable")
                .push((voice_id.to_string(), token_ids.to_vec(), speed));
            Ok(self.output.clone())
        }
    }

    struct MockRuntimeFactory {
        loads: Arc<Mutex<Vec<RuntimeKey>>>,
        synth_calls: SharedSynthCalls,
        output: Vec<f32>,
    }

    impl RuntimeFactory for MockRuntimeFactory {
        fn load(&self, assets: &ResolvedModelAssets) -> Result<Box<dyn KokoroRuntime>, String> {
            self.loads
                .lock()
                .expect("loads should be lockable")
                .push(RuntimeKey::from_assets(assets));
            Ok(Box::new(MockRuntime {
                synth_calls: Arc::clone(&self.synth_calls),
                output: self.output.clone(),
            }))
        }
    }

    fn create_espeak_runtime(dir: &Path) {
        fs::create_dir(dir.join("espeak-ng-data")).expect("runtime data dir should be created");
    }

    fn create_onnxruntime_dll(dir: &Path) -> PathBuf {
        let dll_path = dir.join(platform_onnxruntime_library_name());
        fs::write(&dll_path, b"onnxruntime").expect("onnxruntime dll should be created");
        dll_path
    }

    fn create_kokoro_bundle(dir: &Path) {
        fs::write(dir.join("kokoro-v1.0.onnx"), b"model").expect("model should be created");
        fs::write(dir.join("voices-v1.0.bin"), b"voices").expect("voices bundle should be created");
    }

    fn create_voice_npz(path: &Path) {
        let file = File::create(path).expect("npz file should be created");
        let mut npz = NpzWriter::new(file);
        let style = arr2(&[
            [0.1_f32, 0.2_f32, 0.3_f32],
            [1.1_f32, 1.2_f32, 1.3_f32],
            [2.1_f32, 2.2_f32, 2.3_f32],
        ]);
        npz.add_array("af_heart", &style)
            .expect("voice should be written");
        npz.add_array("bf_emma", &style)
            .expect("voice should be written");
        npz.finish().expect("npz should finish");
    }

    fn create_voice_npz_with_singleton_middle_axis(path: &Path) {
        let file = File::create(path).expect("npz file should be created");
        let mut npz = NpzWriter::new(file);
        let style = Array3::from_shape_vec(
            (3, 1, 3),
            vec![
                0.1_f32, 0.2_f32, 0.3_f32, 1.1_f32, 1.2_f32, 1.3_f32, 2.1_f32, 2.2_f32, 2.3_f32,
            ],
        )
        .expect("3D style tensor should be created");
        npz.add_array("af_alloy", &style)
            .expect("voice should be written");
        npz.finish().expect("npz should finish");
    }

    fn unique_missing_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lingopilot-tts-kokoro-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ))
    }

    fn resolved_assets_for_voice(voice_id: &str) -> ResolvedModelAssets {
        let (lang_code, british, espeak_voice) =
            if voice_id.starts_with("bf_") || voice_id.starts_with("bm_") {
                ("b", true, Some("en"))
            } else if voice_id.starts_with("af_") || voice_id.starts_with("am_") {
                ("a", false, Some("en-us"))
            } else if voice_id.starts_with("ef_") || voice_id.starts_with("em_") {
                ("e", false, Some("es"))
            } else if voice_id.starts_with("ff_") {
                ("f", false, Some("fr"))
            } else if voice_id.starts_with("zf_") || voice_id.starts_with("zm_") {
                ("z", false, None)
            } else {
                ("j", false, None)
            };

        ResolvedModelAssets {
            model_path: PathBuf::from("C:\\models\\kokoro.onnx"),
            voices_path: PathBuf::from("C:\\models\\voices-v1.0.bin"),
            voice: ResolvedVoice {
                voice_id: voice_id.to_string(),
                lang_code,
                british,
                espeak_voice,
            },
        }
    }

    #[test]
    fn validates_espeak_runtime_dir_when_data_subdir_exists() {
        let temp_dir = TempDir::new("espeak-valid");
        create_espeak_runtime(temp_dir.path());

        validate_espeak_data_dir(temp_dir.path()).expect("runtime dir should validate");
    }

    #[test]
    fn rejects_missing_espeak_runtime_dir() {
        let missing = unique_missing_path("espeak-missing");
        let error =
            validate_espeak_data_dir(&missing).expect_err("missing runtime dir should fail");

        assert!(error.contains("Cannot use eSpeak data dir"));
    }

    #[test]
    fn resolves_platform_espeak_shared_library_from_runtime_dir() {
        let temp_dir = TempDir::new("espeak-library");
        create_espeak_runtime(temp_dir.path());
        let library_path = temp_dir.path().join(platform_espeak_library_name());
        fs::write(&library_path, b"binary").expect("shared library placeholder should be created");

        let resolved =
            resolve_espeak_library_path(temp_dir.path()).expect("shared library should resolve");

        assert_eq!(resolved, library_path);
    }

    #[test]
    fn rejects_runtime_dir_without_shared_library() {
        let temp_dir = TempDir::new("espeak-no-library");
        create_espeak_runtime(temp_dir.path());

        let error = resolve_espeak_library_path(temp_dir.path())
            .expect_err("missing shared library should fail");

        assert!(error.contains("missing shared library"));
        assert!(error.contains(platform_espeak_library_name()));
    }

    #[test]
    fn onnxruntime_env_absolute_path_wins_over_sibling_fallback() {
        let env_dir = TempDir::new("onnxruntime-env");
        let exe_dir = TempDir::new("onnxruntime-exe");
        let env_dll = create_onnxruntime_dll(env_dir.path());
        let exe_path = exe_dir.path().join("lingopilot-tts-kokoro.exe");
        let sibling_dll = create_onnxruntime_dll(exe_dir.path());

        let resolved =
            resolve_onnxruntime_library_path_for(Some(env_dll.clone().into()), &exe_path)
                .expect("absolute ORT_DYLIB_PATH should win");

        assert_eq!(resolved, env_dll);
        assert_ne!(resolved, sibling_dll);
    }

    #[test]
    fn onnxruntime_env_requires_absolute_path() {
        let exe_dir = TempDir::new("onnxruntime-relative");
        let exe_path = exe_dir.path().join("lingopilot-tts-kokoro.exe");

        let error = resolve_onnxruntime_library_path_for(
            Some("relative\\onnxruntime.dll".into()),
            &exe_path,
        )
        .expect_err("relative ORT_DYLIB_PATH should fail");

        assert_eq!(
            error,
            format!(
                "Cannot initialize ONNX Runtime: {} must be an absolute path",
                ORT_DYLIB_PATH_ENV
            )
        );
    }

    #[test]
    fn onnxruntime_missing_env_falls_back_to_sibling_library() {
        let exe_dir = TempDir::new("onnxruntime-sibling");
        let exe_path = exe_dir.path().join("lingopilot-tts-kokoro.exe");
        let sibling_dll = create_onnxruntime_dll(exe_dir.path());

        let resolved = resolve_onnxruntime_library_path_for(None, &exe_path)
            .expect("missing env should fall back to sibling dll");

        assert_eq!(resolved, sibling_dll);
    }

    #[test]
    fn onnxruntime_missing_env_and_missing_sibling_return_deterministic_error() {
        let exe_dir = TempDir::new("onnxruntime-missing");
        let exe_path = exe_dir.path().join("lingopilot-tts-kokoro.exe");
        let expected_path = exe_dir.path().join(platform_onnxruntime_library_name());

        let error = resolve_onnxruntime_library_path_for(None, &exe_path)
            .expect_err("missing sibling dll should fail");

        assert_eq!(
            error,
            format!(
                "Cannot initialize ONNX Runtime: missing shared library '{}' beside the executable; alternatively set {} to an absolute path",
                expected_path.display(),
                ORT_DYLIB_PATH_ENV
            )
        );
    }

    #[test]
    fn validates_absolute_existing_model_dir() {
        let temp_dir = TempDir::new("model-valid");

        validate_model_dir(temp_dir.path()).expect("model dir should validate");
    }

    #[test]
    fn rejects_relative_model_dir() {
        let error = validate_model_dir(Path::new("relative-model-dir"))
            .expect_err("relative path should fail");

        assert!(error.contains("path must be absolute"));
    }

    #[test]
    fn rejects_missing_model_dir() {
        let missing = unique_missing_path("model-missing");
        let error = validate_model_dir(&missing).expect_err("missing model dir should fail");

        assert!(error.contains("path does not exist"));
    }

    #[test]
    fn resolves_assets_and_voice_profile_for_american_voice() {
        let temp_dir = TempDir::new("resolve-assets");
        create_kokoro_bundle(temp_dir.path());

        let resolved =
            resolve_model_assets(temp_dir.path(), "af_heart").expect("bundle should resolve");

        assert_eq!(
            resolved.model_path,
            temp_dir.path().join("kokoro-v1.0.onnx")
        );
        assert_eq!(
            resolved.voices_path,
            temp_dir.path().join("voices-v1.0.bin")
        );
        assert_eq!(
            resolved.voice,
            ResolvedVoice {
                voice_id: "af_heart".to_string(),
                lang_code: "a",
                british: false,
                espeak_voice: Some("en-us"),
            }
        );
    }

    #[test]
    fn resolves_assets_and_voice_profile_for_british_voice() {
        let temp_dir = TempDir::new("resolve-british");
        create_kokoro_bundle(temp_dir.path());

        let resolved =
            resolve_model_assets(temp_dir.path(), "bf_emma").expect("bundle should resolve");

        assert_eq!(
            resolved.voice,
            ResolvedVoice {
                voice_id: "bf_emma".to_string(),
                lang_code: "b",
                british: true,
                espeak_voice: Some("en"),
            }
        );
    }

    #[test]
    fn resolves_spanish_voice_with_explicit_espeak_fallback() {
        let temp_dir = TempDir::new("resolve-spanish");
        create_kokoro_bundle(temp_dir.path());

        let resolved =
            resolve_model_assets(temp_dir.path(), "ef_dora").expect("bundle should resolve");

        assert_eq!(resolved.voice.lang_code, "e");
        assert_eq!(resolved.voice.espeak_voice, Some("es"));
    }

    #[test]
    fn resolves_french_voice_with_explicit_espeak_fallback() {
        let temp_dir = TempDir::new("resolve-french");
        create_kokoro_bundle(temp_dir.path());

        let resolved =
            resolve_model_assets(temp_dir.path(), "ff_siwis").expect("bundle should resolve");

        assert_eq!(resolved.voice.lang_code, "f");
        assert_eq!(resolved.voice.espeak_voice, Some("fr"));
    }

    #[test]
    fn resolves_japanese_voice_without_espeak_binding() {
        let temp_dir = TempDir::new("resolve-japanese");
        create_kokoro_bundle(temp_dir.path());

        let resolved =
            resolve_model_assets(temp_dir.path(), "jf_alpha").expect("bundle should resolve");

        assert_eq!(resolved.voice.lang_code, "j");
        assert!(!resolved.voice.british);
        assert_eq!(resolved.voice.espeak_voice, None);
    }

    #[test]
    fn resolves_mandarin_voice_without_espeak_binding() {
        let temp_dir = TempDir::new("resolve-mandarin");
        create_kokoro_bundle(temp_dir.path());

        let resolved =
            resolve_model_assets(temp_dir.path(), "zf_xiaobei").expect("bundle should resolve");

        assert_eq!(resolved.voice.lang_code, "z");
        assert!(!resolved.voice.british);
        assert_eq!(resolved.voice.espeak_voice, None);
    }

    #[test]
    fn rejects_unknown_kokoro_voice_prefix() {
        let temp_dir = TempDir::new("resolve-voice-prefix");
        create_kokoro_bundle(temp_dir.path());

        let error =
            resolve_model_assets(temp_dir.path(), "xx_unknown").expect_err("voice should fail");

        assert!(error.contains("Unsupported Kokoro voice"));
    }

    #[test]
    fn rejects_bundle_without_model_file() {
        let temp_dir = TempDir::new("resolve-no-model");
        fs::write(temp_dir.path().join("voices-v1.0.bin"), b"voices")
            .expect("voices should be created");

        let error =
            resolve_model_assets(temp_dir.path(), "af_heart").expect_err("bundle should fail");

        assert!(error.contains("expected exactly one Kokoro model (*.onnx)"));
    }

    #[test]
    fn rejects_bundle_without_voices_file() {
        let temp_dir = TempDir::new("resolve-no-voices");
        fs::write(temp_dir.path().join("kokoro-v1.0.onnx"), b"model")
            .expect("model should be created");

        let error =
            resolve_model_assets(temp_dir.path(), "af_heart").expect_err("bundle should fail");

        assert!(error.contains("expected exactly one Kokoro voices bundle (voices*.bin)"));
    }

    #[test]
    fn rejects_bundle_with_multiple_models() {
        let temp_dir = TempDir::new("resolve-many-models");
        create_kokoro_bundle(temp_dir.path());
        fs::write(temp_dir.path().join("kokoro-v1.1.onnx"), b"model")
            .expect("second model should be created");

        let error =
            resolve_model_assets(temp_dir.path(), "af_heart").expect_err("bundle should fail");

        assert!(error.contains("expected exactly one Kokoro model (*.onnx), found"));
        assert!(error.contains("kokoro-v1.0.onnx"));
        assert!(error.contains("kokoro-v1.1.onnx"));
    }

    #[test]
    fn loads_voice_styles_from_npz_bundle() {
        let temp_dir = TempDir::new("voice-bundle");
        let voices_path = temp_dir.path().join("voices-v1.0.bin");
        create_voice_npz(&voices_path);

        let styles = load_voice_styles(&voices_path).expect("voice styles should load");

        assert_eq!(styles.len(), 2);
        assert_eq!(styles["af_heart"].shape(), &[3, 3]);
        assert_eq!(styles["af_heart"][[1, 0]], 1.1_f32);
    }

    #[test]
    fn loads_voice_styles_from_npz_bundle_with_singleton_middle_axis() {
        let temp_dir = TempDir::new("voice-bundle-3d");
        let voices_path = temp_dir.path().join("voices-v1.0.bin");
        create_voice_npz_with_singleton_middle_axis(&voices_path);

        let styles = load_voice_styles(&voices_path).expect("voice styles should load");

        assert_eq!(styles.len(), 1);
        assert_eq!(styles["af_alloy"].shape(), &[3, 3]);
        assert_eq!(styles["af_alloy"][[1, 0]], 1.1_f32);
    }

    #[test]
    fn normalizes_american_espeak_phonemes_to_kokoro_symbols() {
        let normalized = normalize_english_phonemes("h e^ɪ l o^ʊ ɚ d^ʒ a^ɪ a^ʊ ɔ^ɪ t^ʃ ʔ ː", false);

        assert_eq!(normalized, "h A l O əɹ ʤ I W Y ʧ t");
    }

    #[test]
    fn normalizes_british_espeak_phonemes_to_kokoro_symbols() {
        let normalized = normalize_english_phonemes("e^ə iə ə^ʊ r", true);

        assert_eq!(normalized, "ɛː ɪə Q ɹ");
    }

    #[test]
    fn filters_non_vocab_symbols_and_keeps_supported_ones() {
        let filtered = filter_phonemes_to_vocab("g ə \n ☃ x");

        assert_eq!(filtered, "ɡ ə x");
    }

    #[test]
    fn tokenizes_phonemes_using_embedded_vocab() {
        let token_ids = tokenize_phonemes("A ə").expect("phonemes should tokenize");

        assert_eq!(token_ids, vec![24, 16, 83]);
    }

    #[test]
    fn splits_phonemes_at_preferred_boundaries_under_limit() {
        let phonemes = format!(
            "{}. {}",
            "a".repeat(kokoro_vocab::MAX_PHONEME_LENGTH - 2),
            "b".repeat(10)
        );

        let chunks = split_phonemes_for_inference(&phonemes);

        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].ends_with('.'));
        assert_eq!(chunks[1], "bbbbbbbbbb");
    }

    #[test]
    fn pcm_conversion_clamps_and_rounds_deterministically() {
        let pcm = float_audio_to_pcm16(&[-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0]);

        assert_eq!(
            pcm,
            vec![i16::MIN, i16::MIN, -16384, 0, 16384, i16::MAX, i16::MAX]
        );
    }

    #[test]
    fn synthesis_reuses_loaded_runtime_state_for_repeated_requests() {
        let loads = Arc::new(Mutex::new(Vec::new()));
        let synth_calls = Arc::new(Mutex::new(Vec::new()));
        let phonemizer_calls = Arc::new(Mutex::new(Vec::new()));
        let phonemizer = MockPhonemizer {
            calls: Arc::clone(&phonemizer_calls),
            result: Ok(PhonemeResult {
                phonemes: "A ə".to_string(),
                lang_code: "a",
            }),
        };
        let factory = MockRuntimeFactory {
            loads: Arc::clone(&loads),
            synth_calls: Arc::clone(&synth_calls),
            output: vec![0.0, 0.5],
        };
        let assets = resolved_assets_for_voice("af_heart");
        let mut cache = SynthesisCache::with_components(Box::new(phonemizer), Box::new(factory));

        let first = cache
            .synthesize("hello", &assets, 1.0)
            .expect("first synthesis should succeed");
        let second = cache
            .synthesize("hello again", &assets, 1.0)
            .expect("second synthesis should succeed");

        assert_eq!(first.sample_rate, kokoro_vocab::SAMPLE_RATE);
        assert_eq!(first.pcm16.len(), 2);
        assert_eq!(second.pcm16.len(), 2);
        assert_eq!(loads.lock().expect("loads should lock").len(), 1);
        assert_eq!(
            phonemizer_calls
                .lock()
                .expect("calls should lock")
                .as_slice(),
            &["af_heart".to_string(), "af_heart".to_string()]
        );
        assert_eq!(synth_calls.lock().expect("calls should lock").len(), 2);
    }

    #[test]
    fn japanese_voice_returns_explicit_phonemization_gap() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let phonemizer = MockPhonemizer {
            calls: Arc::clone(&calls),
            result: Err("phonemization is not implemented yet for lang_code='j'".to_string()),
        };
        let assets = resolved_assets_for_voice("jf_alpha");
        let factory = MockRuntimeFactory {
            loads: Arc::new(Mutex::new(Vec::new())),
            synth_calls: Arc::new(Mutex::new(Vec::new())),
            output: vec![0.0],
        };
        let mut cache = SynthesisCache::with_components(Box::new(phonemizer), Box::new(factory));

        let error = cache
            .synthesize("hello", &assets, 1.0)
            .expect_err("non-english phonemization should fail explicitly");

        assert_eq!(
            calls.lock().expect("calls should be lockable").as_slice(),
            &["jf_alpha".to_string()]
        );
        assert_eq!(
            error,
            "phonemization is not implemented yet for lang_code='j'"
        );
    }

    #[test]
    fn mandarin_voice_returns_explicit_phonemization_gap() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let phonemizer = MockPhonemizer {
            calls: Arc::clone(&calls),
            result: Err("phonemization is not implemented yet for lang_code='z'".to_string()),
        };
        let assets = resolved_assets_for_voice("zf_xiaobei");
        let factory = MockRuntimeFactory {
            loads: Arc::new(Mutex::new(Vec::new())),
            synth_calls: Arc::new(Mutex::new(Vec::new())),
            output: vec![0.0],
        };
        let mut cache = SynthesisCache::with_components(Box::new(phonemizer), Box::new(factory));

        let error = cache
            .synthesize("ni hao", &assets, 1.0)
            .expect_err("mandarin phonemization should fail explicitly");

        assert_eq!(
            calls.lock().expect("calls should be lockable").as_slice(),
            &["zf_xiaobei".to_string()]
        );
        assert_eq!(
            error,
            "phonemization is not implemented yet for lang_code='z'"
        );
    }

    #[test]
    #[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
    fn live_english_synthesis_produces_audio() {
        let live_assets = LiveTestAssets::from_env();
        live_assets.install_onnxruntime_env();

        let mut cache = SynthesisCache::new(
            live_assets.espeak_runtime_dir.clone(),
            ExecutionProvider::Cpu,
        );
        let assets = super::resolve_model_assets(&live_assets.model_dir, "af_heart")
            .expect("assets should resolve");

        let result = cache
            .synthesize("Hello from Kokoro", &assets, 1.0)
            .expect("synthesis should succeed");

        assert_eq!(result.sample_rate, kokoro_vocab::SAMPLE_RATE);
        assert!(!result.pcm16.is_empty());
    }

    #[test]
    #[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
    fn live_spanish_synthesis_produces_audio() {
        let live_assets = LiveTestAssets::from_env();
        live_assets.install_onnxruntime_env();

        let mut cache = SynthesisCache::new(
            live_assets.espeak_runtime_dir.clone(),
            ExecutionProvider::Cpu,
        );
        let assets = super::resolve_model_assets(&live_assets.model_dir, "ef_dora")
            .expect("assets should resolve");

        let result = cache
            .synthesize("Hola desde Kokoro", &assets, 1.0)
            .expect("synthesis should succeed");

        assert_eq!(result.sample_rate, kokoro_vocab::SAMPLE_RATE);
        assert!(!result.pcm16.is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, Kokoro assets, and a DirectX 12 adapter"]
    fn synthesize_af_heart_with_directml_end_to_end() {
        let live_assets = LiveTestAssets::from_env();
        live_assets.install_onnxruntime_env();

        let mut cache = SynthesisCache::new(
            live_assets.espeak_runtime_dir.clone(),
            ExecutionProvider::DirectMl,
        );
        let assets = super::resolve_model_assets(&live_assets.model_dir, "af_heart")
            .expect("assets should resolve");

        let result = cache
            .synthesize("Hello from Kokoro", &assets, 1.0)
            .expect("DirectML synthesis should succeed");

        assert_eq!(result.sample_rate, kokoro_vocab::SAMPLE_RATE);
        assert!(!result.pcm16.is_empty());
    }
}
