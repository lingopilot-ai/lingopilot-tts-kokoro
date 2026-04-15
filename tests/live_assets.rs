#[path = "../src/live_test_support.rs"]
mod live_test_support;
#[path = "support/sidecar.rs"]
mod sidecar;

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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

fn spawn_ready_sidecar(live_assets: &LiveTestAssets, level: Option<&str>) -> SidecarHarness {
    let mut sidecar = SidecarHarness::spawn_with_runtime_and_env(
        &live_assets.espeak_runtime_dir,
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
    assert_eq!(ready["type"], "ready");
    sidecar
}

fn assert_audio_response(sidecar: &mut SidecarHarness) -> usize {
    let audio = sidecar.read_json_line();
    assert_eq!(audio["type"], "audio");
    assert_eq!(audio["sample_rate"], 24000);
    assert_eq!(audio["channels"], 1);

    let byte_length = audio["byte_length"]
        .as_u64()
        .expect("byte_length should be present") as usize;
    assert!(byte_length > 0);
    assert_eq!(byte_length % 2, 0);

    let bytes = sidecar.read_exact_stdout_bytes(byte_length);
    assert_eq!(bytes.len(), byte_length);
    byte_length
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn live_invalid_voice_returns_payload_error_with_real_bundle() {
    let live_assets = LiveTestAssets::from_env();

    let mut sidecar = spawn_ready_sidecar(&live_assets, None);

    sidecar.send_json(json!({
        "text": "Hello from Kokoro",
        "voice": "xx_unknown",
        "speed": 1.0,
        "model_dir": live_assets.model_dir,
    }));

    let error = sidecar.read_json_line();
    assert_eq!(error["type"], "error");

    let message = error["message"]
        .as_str()
        .expect("message should be present");
    assert!(message.starts_with("Invalid request payload:"));
    assert!(message.contains("Unsupported Kokoro voice"));
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
    assert!(
        remaining_stdout.is_empty(),
        "stdout must not contain log output after audio payloads"
    );

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

    for line in stderr.lines().filter(|l| {
        l.contains("event=phonemization_done")
            || l.contains("event=model_loaded")
            || l.contains("event=inference_done")
    }) {
        let idx = line
            .find("duration_ms=")
            .unwrap_or_else(|| panic!("timing line missing duration_ms: {line}"));
        let tail = &line[idx + "duration_ms=".len()..];
        let first = tail.chars().next().expect("duration_ms value must exist");
        assert!(
            first.is_ascii_digit(),
            "duration_ms value must start with a digit: {line}"
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

    let slow_request = json!({
        "text": "Hello from Kokoro",
        "voice": "af_heart",
        "speed": 0.8,
        "model_dir": live_assets.model_dir,
    });
    sidecar.send_json(slow_request);
    let slow_len = assert_audio_response(&mut sidecar);

    let fast_request = json!({
        "text": "Hello from Kokoro",
        "voice": "af_heart",
        "speed": 1.3,
        "model_dir": live_assets.model_dir,
    });
    sidecar.send_json(fast_request);
    let fast_len = assert_audio_response(&mut sidecar);

    assert_ne!(slow_len, fast_len);
    assert!(
        slow_len > fast_len,
        "slower speech should produce more samples"
    );
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

    let mut sidecar = spawn_ready_sidecar(&live_assets, None);

    sidecar.send_json(request_for(
        "Hello from Kokoro",
        "af_heart",
        &special_model_dir,
    ));

    let _ = assert_audio_response(&mut sidecar);

    let _ = fs::remove_dir_all(&special_model_dir);
}
