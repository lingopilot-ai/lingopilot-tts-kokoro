mod kokoro_vocab;
#[cfg(test)]
mod live_test_support;
mod protocol;
mod synthesis;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use protocol::{ErrorKind, TtsRequest, TtsResponse};
use synthesis::ExecutionProvider;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer as FormatWriter;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SAMPLE_RATE: u32 = 24000;
const PRIMARY_LOG_ENV: &str = "KOKORO_TTS_LOG";
const LEGACY_LOG_ENV: &str = "LINGOPILOT_TTS_LOG";

#[derive(Debug, PartialEq, Eq)]
struct StartupConfig {
    espeak_data_dir: PathBuf,
    model_dir: PathBuf,
    execution_provider: ExecutionProvider,
}

struct ObservabilityFormatter;

#[derive(Default)]
struct KeyValueVisitor {
    fields: Vec<String>,
}

impl KeyValueVisitor {
    fn push(&mut self, key: &str, value: String) {
        self.fields.push(format!("{key}={value}"));
    }
}

impl Visit for KeyValueVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field.name(), format_string_value(value));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field.name(), value.to_string());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field.name(), value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field.name(), value.to_string());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field.name(), value.to_string());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.push(field.name(), format!("{value:?}"));
    }
}

impl<S, N> FormatEvent<S, N> for ObservabilityFormatter
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: FormatWriter<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut visitor = KeyValueVisitor::default();
        event.record(&mut visitor);
        write!(writer, "level={}", event.metadata().level())?;
        for field in visitor.fields {
            write!(writer, " {field}")?;
        }
        writeln!(writer)
    }
}

fn format_string_value(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '/'))
    {
        value.to_string()
    } else {
        format!("{value:?}")
    }
}

fn load_log_env_filter() -> tracing_subscriber::EnvFilter {
    for key in [PRIMARY_LOG_ENV, LEGACY_LOG_ENV, "RUST_LOG"] {
        if let Ok(filter) = tracing_subscriber::EnvFilter::try_from_env(key) {
            return filter;
        }
    }
    tracing_subscriber::EnvFilter::new("warn")
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(load_log_env_filter())
        .with_writer(io::stderr)
        .with_ansi(false)
        .event_format(ObservabilityFormatter)
        .init();

    tracing::info!(event = "startup", version = VERSION);

    let startup = match load_startup_config(std::env::args_os()) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("Startup error: {}", message);
            return ExitCode::FAILURE;
        }
    };

    tracing::info!(
        event = "espeak_runtime_selected",
        espeak_data_dir = startup.espeak_data_dir.display().to_string()
    );
    tracing::info!(
        event = "model_dir_selected",
        model_dir = startup.model_dir.display().to_string()
    );
    tracing::info!(
        event = "execution_provider_selected",
        provider = match startup.execution_provider {
            ExecutionProvider::Cpu => "cpu",
            ExecutionProvider::DirectMl => "directml",
        }
    );

    if !send_response(
        &TtsResponse::Ready {
            version: VERSION.to_string(),
            sample_rate: SAMPLE_RATE,
            channels: 1,
            encoding: "pcm16le",
        },
        "ready",
    ) {
        return ExitCode::FAILURE;
    }

    let mut synthesis_cache = synthesis::SynthesisCache::new(
        startup.espeak_data_dir.clone(),
        startup.execution_provider,
    );
    let model_dir = startup.model_dir.clone();

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(error) => {
                tracing::error!(event = "stdin_read_failed", error = error.to_string());
                return ExitCode::from(2);
            }
        };

        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        match parse_request(&line) {
            Ok(req) => handle_request(&mut synthesis_cache, &model_dir, req),
            Err(rejection) => {
                tracing::warn!(
                    event = "request_rejected",
                    category = rejection.category,
                    line_len = line.chars().count(),
                    detail = rejection.message.as_str()
                );
                let _ = send_response(
                    &TtsResponse::Error {
                        id: rejection.id,
                        kind: ErrorKind::BadRequest,
                        message: rejection.message,
                    },
                    "error",
                );
            }
        }
    }

    tracing::info!(event = "stdin_closed");
    ExitCode::SUCCESS
}

fn handle_request(
    synthesis_cache: &mut synthesis::SynthesisCache,
    model_dir: &std::path::Path,
    req: TtsRequest,
) {
    let TtsRequest::Synthesize {
        id,
        text,
        voice_id,
        speed,
    } = req;

    let text_len = text.chars().count();
    tracing::debug!(
        event = "request_received",
        id = id.as_str(),
        voice_id = voice_id.as_str(),
        speed = speed as f64,
        text_len
    );

    let assets = match synthesis::resolve_model_assets(model_dir, &voice_id) {
        Ok(assets) => assets,
        Err(error) => {
            let kind = classify_voice_error(&error);
            let category = match kind {
                ErrorKind::UnknownVoice => "unknown_voice",
                _ => "invalid_request_payload",
            };
            tracing::warn!(
                event = "request_rejected",
                category,
                id = id.as_str(),
                voice_id = voice_id.as_str(),
                detail = error.as_str()
            );
            let _ = send_response(
                &TtsResponse::Error {
                    id: Some(id),
                    kind,
                    message: error,
                },
                "error",
            );
            return;
        }
    };

    tracing::debug!(
        event = "kokoro_assets_resolved",
        id = id.as_str(),
        voice_id = voice_id.as_str(),
        lang_code = assets.voice.lang_code,
        british = assets.voice.british
    );

    match synthesis_cache.synthesize(&text, &assets, speed) {
        Ok(result) => {
            let byte_len = (result.pcm16.len() * 2) as u32;

            let stdout = io::stdout();
            let mut out = stdout.lock();

            let audio_json = serde_json::to_string(&TtsResponse::Audio {
                id: id.clone(),
                bytes: byte_len,
                sample_rate: result.sample_rate,
                channels: 1,
            })
            .expect("audio response serialization");
            if let Err(error) = writeln!(out, "{audio_json}") {
                tracing::error!(
                    event = "stdout_write_failed",
                    stage = "audio",
                    error = error.to_string()
                );
                return;
            }

            for sample in &result.pcm16 {
                if let Err(error) = out.write_all(&sample.to_le_bytes()) {
                    tracing::error!(
                        event = "stdout_write_failed",
                        stage = "audio_bytes",
                        error = error.to_string()
                    );
                    return;
                }
            }

            let done_json = serde_json::to_string(&TtsResponse::Done { id: id.clone() })
                .expect("done response serialization");
            if let Err(error) = writeln!(out, "{done_json}") {
                tracing::error!(
                    event = "stdout_write_failed",
                    stage = "done",
                    error = error.to_string()
                );
                return;
            }
            if let Err(error) = out.flush() {
                tracing::error!(
                    event = "stdout_flush_failed",
                    stage = "done",
                    error = error.to_string()
                );
                return;
            }

            tracing::debug!(
                event = "request_succeeded",
                id = id.as_str(),
                voice_id = voice_id.as_str(),
                text_len,
                sample_rate = result.sample_rate as u64,
                byte_length = byte_len as u64
            );
        }
        Err(error) => {
            tracing::warn!(
                event = "request_failed",
                category = "synthesis_failed",
                id = id.as_str(),
                voice_id = voice_id.as_str(),
                detail = error.as_str()
            );
            let _ = send_response(
                &TtsResponse::Error {
                    id: Some(id),
                    kind: ErrorKind::SynthesisFailed,
                    message: error,
                },
                "error",
            );
        }
    }
}

fn classify_voice_error(error: &str) -> ErrorKind {
    if error.starts_with("Unsupported Kokoro voice")
        || error.starts_with("Invalid voice")
        || error.contains("is not present in voices bundle")
    {
        ErrorKind::UnknownVoice
    } else {
        ErrorKind::BadRequest
    }
}

#[derive(Debug)]
struct Rejection {
    id: Option<String>,
    category: &'static str,
    message: String,
}

fn parse_request(line: &str) -> Result<TtsRequest, Rejection> {
    // Lenient pre-parse to recover the request id for error echoing and to
    // reject reserved ops (`audio_chunk`, `cancel`) with the specified message.
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(error) => {
            return Err(Rejection {
                id: None,
                category: "invalid_json",
                message: format!("Invalid JSON request: {}", error),
            });
        }
    };

    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    if let Some(op) = value.get("op").and_then(|v| v.as_str()) {
        if op == "audio_chunk" || op == "cancel" {
            return Err(Rejection {
                id,
                category: "invalid_request_payload",
                message: "op not supported in this version".to_string(),
            });
        }
    }

    let request: TtsRequest = serde_json::from_value(value).map_err(|error| Rejection {
        id: id.clone(),
        category: "invalid_request_payload",
        message: format!("Invalid request payload: {}", error),
    })?;

    request.validate().map_err(|error| Rejection {
        id,
        category: "invalid_request_payload",
        message: format!("Invalid request payload: {}", error),
    })?;

    Ok(request)
}

fn load_startup_config<I, S>(args: I) -> Result<StartupConfig, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let config = parse_startup_config(args)?;
    synthesis::validate_espeak_data_dir(&config.espeak_data_dir)?;
    synthesis::validate_model_dir(&config.model_dir)?;
    Ok(config)
}

fn discover_default_dir(subdir: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Cannot determine executable path for auto-discovery: {e}"))?;
    let parent = exe.parent().ok_or_else(|| {
        format!(
            "Cannot determine executable directory from '{}'",
            exe.display()
        )
    })?;
    Ok(parent.join(subdir))
}

fn parse_startup_config<I, S>(args: I) -> Result<StartupConfig, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let _binary = args.next();

    let mut espeak_data_dir: Option<PathBuf> = None;
    let mut model_dir: Option<PathBuf> = None;
    let mut execution_provider: Option<ExecutionProvider> = None;

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--espeak-data-dir") {
            if espeak_data_dir.is_some() {
                return Err("Duplicate startup argument: --espeak-data-dir".to_string());
            }
            let Some(value) = args.next() else {
                return Err("Missing value for --espeak-data-dir".to_string());
            };
            espeak_data_dir = Some(PathBuf::from(value));
            continue;
        }

        if arg == OsStr::new("--model-dir") {
            if model_dir.is_some() {
                return Err("Duplicate startup argument: --model-dir".to_string());
            }
            let Some(value) = args.next() else {
                return Err("Missing value for --model-dir".to_string());
            };
            model_dir = Some(PathBuf::from(value));
            continue;
        }

        if arg == OsStr::new("--execution-provider") {
            if execution_provider.is_some() {
                return Err("Duplicate startup argument: --execution-provider".to_string());
            }
            let Some(value) = args.next() else {
                return Err("Missing value for --execution-provider".to_string());
            };
            let value_str = value.to_string_lossy().to_string();
            let ep = match value_str.as_str() {
                "cpu" => ExecutionProvider::Cpu,
                "directml" => {
                    #[cfg(not(target_os = "windows"))]
                    {
                        return Err(
                            "--execution-provider directml is supported only on Windows"
                                .to_string(),
                        );
                    }
                    #[cfg(target_os = "windows")]
                    {
                        ExecutionProvider::DirectMl
                    }
                }
                other => {
                    return Err(format!(
                        "Invalid value for --execution-provider: '{}' (expected 'cpu' or 'directml')",
                        other
                    ));
                }
            };
            execution_provider = Some(ep);
            continue;
        }

        return Err(format!(
            "Unknown startup argument: {}",
            arg.to_string_lossy()
        ));
    }

    let espeak_data_dir = match espeak_data_dir {
        Some(p) => p,
        None => discover_default_dir("espeak-runtime")?,
    };
    let model_dir = match model_dir {
        Some(p) => p,
        None => discover_default_dir("kokoro-model")?,
    };

    Ok(StartupConfig {
        espeak_data_dir,
        model_dir,
        execution_provider: execution_provider.unwrap_or(ExecutionProvider::Cpu),
    })
}

fn send_response(response: &TtsResponse, response_type: &'static str) -> bool {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let json = serde_json::to_string(response).expect("failed to serialize response");
    if let Err(error) = writeln!(out, "{json}") {
        tracing::error!(
            event = "stdout_write_failed",
            stage = "response",
            response_type,
            error = error.to_string()
        );
        return false;
    }
    if let Err(error) = out.flush() {
        tracing::error!(
            event = "stdout_flush_failed",
            stage = "response",
            response_type,
            error = error.to_string()
        );
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{classify_voice_error, parse_request, parse_startup_config, ExecutionProvider};
    use crate::protocol::{ErrorKind, TtsRequest};
    use std::path::PathBuf;

    #[test]
    fn startup_config_accepts_explicit_espeak_and_model_dir_flags() {
        let config = parse_startup_config([
            "lingopilot-tts-kokoro",
            "--espeak-data-dir",
            "C:\\runtime\\espeak-runtime",
            "--model-dir",
            "C:\\models\\kokoro-en",
        ])
        .expect("startup config should parse");
        assert_eq!(
            config.espeak_data_dir,
            PathBuf::from("C:\\runtime\\espeak-runtime")
        );
        assert_eq!(config.model_dir, PathBuf::from("C:\\models\\kokoro-en"));
        assert_eq!(config.execution_provider, ExecutionProvider::Cpu);
    }

    #[test]
    fn startup_config_auto_discovers_when_flags_omitted() {
        let config = parse_startup_config(["lingopilot-tts-kokoro"])
            .expect("startup config should parse with auto-discovery");
        assert!(config.espeak_data_dir.ends_with("espeak-runtime"));
        assert!(config.model_dir.ends_with("kokoro-model"));
    }

    #[test]
    fn startup_config_rejects_unknown_argument() {
        let error = parse_startup_config(["lingopilot-tts-kokoro", "--bogus"])
            .expect_err("unknown arg should be rejected");
        assert!(error.starts_with("Unknown startup argument"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn startup_config_accepts_directml_execution_provider() {
        let config = parse_startup_config([
            "lingopilot-tts-kokoro",
            "--espeak-data-dir",
            "C:\\runtime\\espeak-runtime",
            "--model-dir",
            "C:\\models\\kokoro-en",
            "--execution-provider",
            "directml",
        ])
        .expect("startup config should parse");
        assert_eq!(config.execution_provider, ExecutionProvider::DirectMl);
    }

    #[test]
    fn parse_request_accepts_synthesize_op() {
        let req = parse_request(
            r#"{"op":"synthesize","id":"r1","text":"Hi","voice_id":"af_heart","speed":1.0}"#,
        )
        .expect("valid request");
        match req {
            TtsRequest::Synthesize { id, voice_id, .. } => {
                assert_eq!(id, "r1");
                assert_eq!(voice_id, "af_heart");
            }
        }
    }

    #[test]
    fn parse_request_rejects_reserved_audio_chunk_op_and_echoes_id() {
        let err = parse_request(r#"{"op":"audio_chunk","id":"r2"}"#).expect_err("reserved op");
        assert_eq!(err.id.as_deref(), Some("r2"));
        assert!(err.message.contains("op not supported in this version"));
    }

    #[test]
    fn parse_request_rejects_reserved_cancel_op() {
        let err = parse_request(r#"{"op":"cancel","id":"r3"}"#).expect_err("reserved op");
        assert_eq!(err.id.as_deref(), Some("r3"));
        assert!(err.message.contains("op not supported in this version"));
    }

    #[test]
    fn parse_request_echoes_id_on_semantic_validation_failure() {
        let err = parse_request(
            r#"{"op":"synthesize","id":"r4","text":"   ","voice_id":"af_heart","speed":1.0}"#,
        )
        .expect_err("blank text");
        assert_eq!(err.id.as_deref(), Some("r4"));
        assert!(err.message.starts_with("Invalid request payload:"));
    }

    #[test]
    fn parse_request_returns_none_id_on_malformed_json() {
        let err = parse_request("not json").expect_err("bad json");
        assert!(err.id.is_none());
        assert!(err.message.starts_with("Invalid JSON request:"));
    }

    #[test]
    fn classify_voice_error_recognises_unknown_voice_cases() {
        assert!(matches!(
            classify_voice_error("Unsupported Kokoro voice 'xx_test': ..."),
            ErrorKind::UnknownVoice
        ));
        assert!(matches!(
            classify_voice_error("Invalid voice: voice must not be empty or whitespace"),
            ErrorKind::UnknownVoice
        ));
        assert!(matches!(
            classify_voice_error(
                "Kokoro voice 'af_ghost' is not present in voices bundle 'C:/x/voices.bin'"
            ),
            ErrorKind::UnknownVoice
        ));
        assert!(matches!(
            classify_voice_error("Invalid model_dir 'C:/x': path must be absolute"),
            ErrorKind::BadRequest
        ));
    }
}
