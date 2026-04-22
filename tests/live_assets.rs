#[path = "../src/live_test_support.rs"]
mod live_test_support;
#[path = "support/fingerprint.rs"]
mod fingerprint;
#[path = "support/sidecar.rs"]
mod sidecar;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use live_test_support::LiveTestAssets;
use serde_json::json;
use sidecar::{assert_stderr_is_plain_text, request_for, SidecarHarness};

fn temp_live_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();

    std::env::temp_dir().join(format!(
        "lingopilot-tts-kokoro-{prefix}-{}-{nonce}",
        std::process::id()
    ))
}

fn spawn_ready_sidecar_with_model(
    live_assets: &LiveTestAssets,
    model_dir: &Path,
    level: Option<&str>,
) -> SidecarHarness {
    let mut sidecar = SidecarHarness::spawn_with_dirs_and_env(
        &live_assets.espeak_runtime_dir,
        Some(model_dir),
        level,
        &[(
            "ORT_DYLIB_PATH",
            live_assets
                .onnxruntime_dll
                .to_str()
                .expect("dll path should be utf-8"),
        )],
    );

    let ready = sidecar.read_json_line();
    assert_eq!(ready["op"], "ready");
    assert_eq!(ready["sample_rate"], 24000);
    assert_eq!(ready["channels"], 1);
    assert_eq!(ready["encoding"], "pcm16le");
    sidecar
}

fn spawn_ready_sidecar(live_assets: &LiveTestAssets, level: Option<&str>) -> SidecarHarness {
    let model_dir = live_assets.model_dir.clone();
    spawn_ready_sidecar_with_model(live_assets, &model_dir, level)
}

fn assert_audio_response(sidecar: &mut SidecarHarness) -> usize {
    let audio = sidecar.read_json_line();
    assert_eq!(audio["op"], "audio");
    assert_eq!(audio["sample_rate"], 24000);
    assert_eq!(audio["channels"], 1);

    let byte_length = audio["bytes"]
        .as_u64()
        .expect("bytes should be present") as usize;
    assert!(byte_length > 0);
    assert_eq!(byte_length % 2, 0);

    let bytes = sidecar.read_exact_stdout_bytes(byte_length);
    assert_eq!(bytes.len(), byte_length);

    let done = sidecar.read_json_line();
    assert_eq!(done["op"], "done");
    assert_eq!(done["id"], audio["id"]);

    byte_length
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_invalid_voice_returns_payload_error_with_real_bundle() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);

    sidecar.send_json(json!({
        "op": "synthesize",
        "id": "invalid-voice-1",
        "text": "Hello from Kokoro",
        "voice_id": "xx_unknown",
        "speed": 1.0,
    }));

    let error = sidecar.read_json_line();
    assert_eq!(error["op"], "error");
    assert_eq!(error["kind"], "unknown_voice");
    assert_eq!(error["id"], "invalid-voice-1");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("Unsupported Kokoro voice"));
}

fn write_wav_pcm16le_mono_24k(path: &Path, pcm: &[u8]) {
    let sample_rate: u32 = 24_000;
    let channels: u16 = 1;
    let bits: u16 = 16;
    let data_len = pcm.len() as u32;
    let byte_rate = sample_rate * channels as u32 * (bits as u32 / 8);
    let block_align = channels * (bits / 8);

    let mut f = fs::File::create(path).expect("wav file");
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&channels.to_le_bytes()).unwrap();
    f.write_all(&sample_rate.to_le_bytes()).unwrap();
    f.write_all(&byte_rate.to_le_bytes()).unwrap();
    f.write_all(&block_align.to_le_bytes()).unwrap();
    f.write_all(&bits.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    f.write_all(pcm).unwrap();
}

fn play_wav_blocking(path: &Path) {
    #[cfg(windows)]
    {
        let status = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "(New-Object Media.SoundPlayer '{}').PlaySync()",
                    path.display()
                ),
            ])
            .status()
            .expect("powershell should be available");
        assert!(status.success(), "playback failed");
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("aplay").arg(path).status();
    }
}

const FINGERPRINT_FIXTURE: &str = "tests/fixtures/af_heart_hello.json";
const FINGERPRINT_TEXT: &str = "Hello from Kokoro";
const FINGERPRINT_VOICE: &str = "af_heart";
const FINGERPRINT_REQUEST_DEADLINE: Duration = Duration::from_secs(45);

fn read_audio_pcm_with_deadline(sidecar: &mut SidecarHarness) -> Vec<u8> {
    let guard = sidecar.arm_deadline(FINGERPRINT_REQUEST_DEADLINE);
    let audio = sidecar.read_json_line();
    assert_eq!(audio["op"], "audio");
    let byte_length = audio["bytes"].as_u64().expect("bytes field") as usize;
    assert!(byte_length > 0 && byte_length % 2 == 0);
    let bytes = sidecar.read_exact_stdout_bytes(byte_length);
    let done = sidecar.read_json_line();
    assert_eq!(done["op"], "done");
    assert_eq!(done["id"], audio["id"]);
    guard.cancel();
    bytes
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_english_af_heart_fingerprint() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);

    sidecar.send_json(request_for(
        FINGERPRINT_TEXT,
        FINGERPRINT_VOICE,
        &live_assets.model_dir,
    ));

    let pcm = read_audio_pcm_with_deadline(&mut sidecar);
    let actual = fingerprint::compute(&pcm);

    if std::env::var("KOKORO_TTS_PLAY_FINGERPRINT").map(|v| v == "1").unwrap_or(false) {
        let wav_path = std::env::temp_dir().join("lingopilot-tts-kokoro-fingerprint.wav");
        write_wav_pcm16le_mono_24k(&wav_path, &pcm);
        eprintln!("▶ playing {}", wav_path.display());
        play_wav_blocking(&wav_path);
    }

    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FINGERPRINT_FIXTURE);
    let update_mode = std::env::var("KOKORO_TTS_UPDATE_FINGERPRINT")
        .map(|v| v == "1")
        .unwrap_or(false);

    match fingerprint::load_baseline(&fixture_path) {
        Some(baseline) if !update_mode => {
            fingerprint::assert_within(&actual, &baseline);
            eprintln!(
                "fingerprint OK: bytes={} rms={:.4} zcr={:.4}",
                actual.byte_length, actual.mean_rms, actual.mean_zcr
            );
        }
        _ => {
            let baseline = fingerprint::Baseline {
                fingerprint: actual.clone(),
                byte_length_tol: 0.10,
                rms_tol: 0.15,
                zcr_tol: 0.10,
                notes: format!(
                    "af_heart voice, text={FINGERPRINT_TEXT:?}. Regenerate with \
                     KOKORO_TTS_UPDATE_FINGERPRINT=1 when model/ORT changes are intentional."
                ),
            };
            fingerprint::write_baseline(&fixture_path, &baseline);
            panic!(
                "Fingerprint baseline written to {}. Re-run without \
                 KOKORO_TTS_UPDATE_FINGERPRINT=1 to verify. Computed: bytes={} rms={:.4} zcr={:.4}",
                fixture_path.display(),
                actual.byte_length,
                actual.mean_rms,
                actual.mean_zcr
            );
        }
    }
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_american_english_request_returns_audio_and_exact_pcm_bytes() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);
    sidecar.send_json(request_for(
        "Hello from Kokoro",
        "af_heart",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_british_english_request_returns_audio_and_exact_pcm_bytes() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);
    sidecar.send_json(request_for(
        "Hello from Kokoro",
        "bf_emma",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_spanish_request_returns_audio() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);
    sidecar.send_json(request_for(
        "Hola desde Kokoro",
        "ef_dora",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_french_request_returns_audio() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);
    sidecar.send_json(request_for(
        "Bonjour depuis Kokoro",
        "ff_siwis",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_hindi_request_returns_audio() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);
    sidecar.send_json(request_for(
        "Namaste from Kokoro",
        "hf_alpha",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_italian_request_returns_audio() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);
    sidecar.send_json(request_for(
        "Ciao da Kokoro",
        "if_sara",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_brazilian_portuguese_request_returns_audio() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);
    sidecar.send_json(request_for(
        "Ola do Kokoro",
        "pf_dora",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_same_process_requests_both_succeed_and_keep_streams_separated() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, Some("debug"));

    sidecar.send_json(request_for(
        "Hello from Kokoro",
        "af_heart",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);

    sidecar.send_json(request_for(
        "Hola desde Kokoro",
        "ef_dora",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);

    sidecar.close_stdin();
    let remaining_stdout = sidecar.read_remaining_stdout();
    assert!(remaining_stdout.is_empty());

    let stderr = sidecar.shutdown_and_collect_stderr();
    assert!(stderr.contains("level=DEBUG event=request_succeeded"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_successful_request_emits_timing_events() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, Some("info"));

    sidecar.send_json(request_for(
        "Hello from Kokoro",
        "af_heart",
        &live_assets.model_dir,
    ));
    let _ = assert_audio_response(&mut sidecar);

    let stderr = sidecar.shutdown_and_collect_stderr();
    assert_stderr_is_plain_text(&stderr);

    for needle in [
        "level=INFO event=phonemization_done",
        "level=INFO event=model_loaded",
        "level=INFO event=inference_done",
    ] {
        assert!(
            stderr.contains(needle),
            "stderr must contain `{needle}`; got:\n{stderr}"
        );
    }
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_cache_hit_is_logged_on_second_same_voice_request() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, Some("debug"));

    for text in ["Hello from Kokoro", "Hello again from Kokoro"] {
        sidecar.send_json(request_for(text, "af_heart", &live_assets.model_dir));
        let _ = assert_audio_response(&mut sidecar);
    }

    let stderr = sidecar.shutdown_and_collect_stderr();
    assert!(stderr.contains("event=kokoro_runtime_cache_hit"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_speed_parameter_changes_byte_length() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets, None);

    sidecar.send_json(json!({
        "op": "synthesize",
        "id": "speed-slow",
        "text": "Hello from Kokoro",
        "voice_id": "af_heart",
        "speed": 0.8,
    }));
    let slow_len = assert_audio_response(&mut sidecar);

    sidecar.send_json(json!({
        "op": "synthesize",
        "id": "speed-fast",
        "text": "Hello from Kokoro",
        "voice_id": "af_heart",
        "speed": 1.3,
    }));
    let fast_len = assert_audio_response(&mut sidecar);

    assert_ne!(slow_len, fast_len);
    assert!(slow_len > fast_len);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_model_dir_with_spaces_and_non_ascii_succeeds() {
    let live_assets = LiveTestAssets::from_env();
    let special_model_dir = temp_live_dir("Paulo Ação model");
    fs::create_dir_all(&special_model_dir).expect("special model dir should be created");

    for entry in fs::read_dir(&live_assets.model_dir).expect("model dir should be readable") {
        let entry = entry.expect("dir entry should be readable");
        let destination = special_model_dir.join(entry.file_name());
        fs::copy(entry.path(), destination).expect("model asset should copy");
    }

    let mut sidecar = spawn_ready_sidecar_with_model(&live_assets, &special_model_dir, None);

    sidecar.send_json(request_for(
        "Hello from Kokoro",
        "af_heart",
        &special_model_dir,
    ));

    let _ = assert_audio_response(&mut sidecar);

    let _ = fs::remove_dir_all(&special_model_dir);
}
