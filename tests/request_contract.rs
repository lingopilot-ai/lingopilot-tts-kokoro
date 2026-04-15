use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[path = "../src/protocol.rs"]
mod protocol_contract;

const PRIMARY_LOG_ENV: &str = "KOKORO_TTS_LOG";
const LEGACY_LOG_ENV: &str = "LINGOPILOT_TTS_LOG";

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

fn create_espeak_runtime(dir: &Path) {
    fs::create_dir(dir.join("espeak-ng-data")).expect("runtime data dir should be created");
}

fn unique_missing_path(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();

    std::env::temp_dir().join(format!(
        "lingopilot-tts-kokoro-{prefix}-{}-{nonce}",
        std::process::id()
    ))
}

struct SidecarHarness {
    _runtime_dir: Option<TempDir>,
    child: Child,
    stdout: BufReader<ChildStdout>,
    stderr: BufReader<ChildStderr>,
}

impl SidecarHarness {
    fn spawn() -> Self {
        Self::spawn_with_log_level(None)
    }

    fn spawn_with_log_level(level: Option<&str>) -> Self {
        let runtime_dir = TempDir::new("runtime");
        create_espeak_runtime(runtime_dir.path());
        let runtime_path = runtime_dir.path().to_path_buf();
        Self::spawn_with_runtime_and_env(&runtime_path, Some(runtime_dir), level, &[])
    }

    fn spawn_with_runtime_and_env(
        runtime_dir: &Path,
        owned_runtime_dir: Option<TempDir>,
        level: Option<&str>,
        extra_env: &[(&str, &str)],
    ) -> Self {
        let mut command = sidecar_command();
        if let Some(level) = level {
            command.env(PRIMARY_LOG_ENV, level);
        }
        for (key, value) in extra_env {
            command.env(key, value);
        }

        let mut child = command
            .arg("--espeak-data-dir")
            .arg(runtime_dir)
            .spawn()
            .expect("sidecar should start");

        let stdout = child.stdout.take().expect("stdout should be piped");
        let stderr = child.stderr.take().expect("stderr should be piped");

        Self {
            _runtime_dir: owned_runtime_dir,
            child,
            stdout: BufReader::new(stdout),
            stderr: BufReader::new(stderr),
        }
    }

    fn send_json(&mut self, value: Value) {
        let stdin = self.child.stdin.as_mut().expect("stdin should be piped");
        writeln!(stdin, "{value}").expect("request should be written");
        stdin.flush().expect("stdin should flush");
    }

    fn send_raw_line(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().expect("stdin should be piped");
        writeln!(stdin, "{line}").expect("request should be written");
        stdin.flush().expect("stdin should flush");
    }

    fn read_json_line(&mut self) -> Value {
        let mut line = String::new();
        let bytes = self
            .stdout
            .read_line(&mut line)
            .expect("stdout should be readable");
        assert!(bytes > 0, "expected a JSON line from the sidecar");

        serde_json::from_str(line.trim_end()).expect("sidecar should emit valid JSON")
    }

    fn close_stdin(&mut self) {
        let _ = self.child.stdin.take();
    }

    fn read_remaining_stdout(&mut self) -> String {
        let mut remaining = String::new();
        self.stdout
            .read_to_string(&mut remaining)
            .expect("stdout should be readable until EOF");
        remaining
    }

    fn peek_stdout_byte(&mut self) -> Option<u8> {
        self.stdout
            .fill_buf()
            .expect("stdout should be readable")
            .first()
            .copied()
    }

    fn shutdown_and_collect_stderr(&mut self) -> String {
        self.close_stdin();
        let _ = self.child.wait();

        let mut stderr = String::new();
        self.stderr
            .read_to_string(&mut stderr)
            .expect("stderr should be readable");
        stderr
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        loop {
            match self
                .child
                .try_wait()
                .expect("process status should be readable")
            {
                Some(status) => return status.code(),
                None if Instant::now() >= deadline => return None,
                None => thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}

impl Drop for SidecarHarness {
    fn drop(&mut self) {
        self.close_stdin();
        let _ = self.child.wait();
    }
}

fn sidecar_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lingopilot-tts-kokoro"));
    command
        .env_remove(PRIMARY_LOG_ENV)
        .env_remove(LEGACY_LOG_ENV)
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn request_for(text: &str, voice: &str, model_dir: &Path) -> Value {
    json!({
        "text": text,
        "voice": voice,
        "speed": 1.0,
        "model_dir": model_dir,
    })
}

fn valid_supported_request(model_dir: &Path) -> Value {
    request_for(
        "Hello from the Kokoro sidecar scaffold",
        "af_heart",
        model_dir,
    )
}

fn create_kokoro_bundle(dir: &Path) {
    fs::write(dir.join("kokoro-v1.0.onnx"), b"model").expect("model should be created");
    fs::write(dir.join("voices-v1.0.bin"), b"voices").expect("voices bundle should be created");
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type")]
enum PiperCompatibleResponse {
    #[serde(rename = "ready")]
    Ready { version: String },
    #[serde(rename = "audio")]
    Audio {
        byte_length: u32,
        sample_rate: u32,
        channels: u16,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

fn assert_locked_error_prefix(message: &str, expected_prefix: &str) {
    const ALLOWED_PREFIXES: [&str; 3] = [
        "Invalid JSON request:",
        "Invalid request payload:",
        "Synthesis failed:",
    ];

    assert!(
        ALLOWED_PREFIXES.contains(&expected_prefix),
        "test must only assert locked error prefixes"
    );
    assert!(
        ALLOWED_PREFIXES
            .iter()
            .any(|prefix| message.starts_with(prefix)),
        "error message '{message}' did not use a locked prefix"
    );
    assert!(
        message.starts_with(expected_prefix),
        "error message '{message}' did not start with '{expected_prefix}'"
    );
}

fn assert_stderr_is_plain_text(stderr: &str) {
    assert!(!stderr.contains('\0'), "stderr must not contain NUL bytes");
    assert!(
        !stderr.contains("{\"type\""),
        "stderr must not contain protocol JSON"
    );
}

#[test]
fn valid_startup_flag_emits_exactly_one_ready() {
    let mut sidecar = SidecarHarness::spawn();

    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["version"], env!("CARGO_PKG_VERSION"));

    sidecar.close_stdin();
    let remaining = sidecar.read_remaining_stdout();
    assert!(remaining.is_empty(), "expected no extra stdout after ready");
}

#[test]
fn startup_unknown_argument_exits_without_ready() {
    let output = sidecar_command()
        .arg("--unknown-flag")
        .output()
        .expect("sidecar should run to completion");

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "expected no ready output on stdout"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Startup error: Unknown startup argument"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn startup_duplicate_argument_exits_without_ready() {
    let runtime_dir = TempDir::new("duplicate-startup-runtime");
    create_espeak_runtime(runtime_dir.path());

    let output = sidecar_command()
        .arg("--espeak-data-dir")
        .arg(runtime_dir.path())
        .arg("--espeak-data-dir")
        .arg(runtime_dir.path())
        .output()
        .expect("sidecar should run to completion");

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "expected no ready output on stdout"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Startup error: Duplicate startup argument"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn startup_missing_flag_value_exits_without_ready() {
    let output = sidecar_command()
        .arg("--espeak-data-dir")
        .output()
        .expect("sidecar should run to completion");

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "expected no ready output on stdout"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Startup error: Missing value for --espeak-data-dir"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn missing_startup_flag_exits_without_ready() {
    let output = sidecar_command()
        .output()
        .expect("sidecar should run to completion");

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "expected no ready output on stdout"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Startup error: Missing required startup argument"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn invalid_startup_path_exits_without_ready() {
    let output = sidecar_command()
        .arg("--espeak-data-dir")
        .arg(unique_missing_path("missing-espeak-startup"))
        .output()
        .expect("sidecar should run to completion");

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "expected no ready output on stdout"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Startup error: Cannot use eSpeak data dir"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn malformed_json_returns_error_and_process_stays_alive() {
    let mut sidecar = SidecarHarness::spawn();

    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    sidecar.send_raw_line(r#"{"text":"Hello","voice":"af_heart""#);

    let error = sidecar.read_json_line();
    assert_eq!(error["type"], "error");
    assert_locked_error_prefix(
        error["message"]
            .as_str()
            .expect("message should be present"),
        "Invalid JSON request:",
    );

    sidecar.send_json(json!({
        "text": "",
        "voice": "af_heart",
        "speed": 1.0,
        "model_dir": "C:\\models\\kokoro-en",
    }));

    let follow_up = sidecar.read_json_line();
    assert_eq!(follow_up["type"], "error");
    assert_locked_error_prefix(
        follow_up["message"]
            .as_str()
            .expect("message should be present"),
        "Invalid request payload:",
    );
}

#[test]
fn request_with_legacy_espeak_data_dir_is_rejected_and_process_stays_alive() {
    let mut sidecar = SidecarHarness::spawn();
    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    sidecar.send_json(json!({
        "text": "Hello",
        "voice": "af_heart",
        "speed": 1.0,
        "model_dir": "unused",
        "espeak_data_dir": "unused"
    }));

    let error = sidecar.read_json_line();
    assert_eq!(error["type"], "error");
    let message = error["message"]
        .as_str()
        .expect("message should be present");
    assert_locked_error_prefix(message, "Invalid request payload:");
    assert!(message.contains("unknown field `espeak_data_dir`"));
}

#[test]
fn request_with_language_is_rejected_and_process_stays_alive() {
    let mut sidecar = SidecarHarness::spawn();
    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    sidecar.send_json(json!({
        "text": "Hello",
        "language": "en",
        "voice": "af_heart",
        "speed": 1.0,
        "model_dir": "unused"
    }));

    let error = sidecar.read_json_line();
    assert_eq!(error["type"], "error");
    let message = error["message"]
        .as_str()
        .expect("message should be present");
    assert_locked_error_prefix(message, "Invalid request payload:");
    assert!(message.contains("unknown field `language`"));
}

#[test]
fn invalid_semantic_payload_returns_error_and_process_stays_alive() {
    let mut sidecar = SidecarHarness::spawn();
    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    let invalid_requests = vec![
        (
            json!({
                "text": "",
                "voice": "af_heart",
                "speed": 1.0,
                "model_dir": "C:\\models\\kokoro-en",
            }),
            "text must not be empty or whitespace",
        ),
        (
            json!({
                "text": "Hello",
                "voice": "",
                "speed": 1.0,
                "model_dir": "C:\\models\\kokoro-en",
            }),
            "voice must not be empty or whitespace",
        ),
        (
            json!({
                "text": "Hello",
                "voice": "af_heart",
                "speed": 2.01,
                "model_dir": "C:\\models\\kokoro-en",
            }),
            "speed must be a finite number between 0.5 and 2",
        ),
    ];

    for (request, expected_fragment) in invalid_requests {
        sidecar.send_json(request);

        let error = sidecar.read_json_line();
        assert_eq!(error["type"], "error");
        let message = error["message"]
            .as_str()
            .expect("message should be present");
        assert_locked_error_prefix(message, "Invalid request payload:");
        assert!(message.contains(expected_fragment));
    }
}

#[test]
fn missing_kokoro_assets_return_payload_error_before_synthesis() {
    let mut sidecar = SidecarHarness::spawn();
    let empty_bundle = TempDir::new("empty-bundle");

    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    sidecar.send_json(valid_supported_request(empty_bundle.path()));

    let error = sidecar.read_json_line();
    assert_eq!(error["type"], "error");
    let message = error["message"]
        .as_str()
        .expect("message should be present");
    assert_locked_error_prefix(message, "Invalid request payload:");
    assert!(message.contains("expected exactly one Kokoro model (*.onnx)"));
}

#[test]
fn debug_logging_keeps_stdout_protocol_only_and_stderr_text_only_on_error_paths() {
    let mut sidecar = SidecarHarness::spawn_with_log_level(Some("debug"));
    let bundle = TempDir::new("debug-empty-bundle");

    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    sidecar.send_json(valid_supported_request(bundle.path()));
    let error = sidecar.read_json_line();
    assert_eq!(error["type"], "error");
    assert_locked_error_prefix(
        error["message"]
            .as_str()
            .expect("message should be present"),
        "Invalid request payload:",
    );

    sidecar.close_stdin();
    let remaining_stdout = sidecar.read_remaining_stdout();
    assert!(
        remaining_stdout.is_empty(),
        "stdout must not contain log output"
    );

    let stderr = sidecar.shutdown_and_collect_stderr();
    assert!(stderr.contains("level=INFO event=startup"));
    assert!(stderr.contains("level=WARN event=request_rejected"));
    assert!(stderr.contains("category=invalid_request_payload"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn neutral_log_env_is_preferred_over_legacy_alias() {
    let runtime_dir = TempDir::new("neutral-log-runtime");
    create_espeak_runtime(runtime_dir.path());
    let runtime_path = runtime_dir.path().to_path_buf();
    let mut sidecar = SidecarHarness::spawn_with_runtime_and_env(
        &runtime_path,
        Some(runtime_dir),
        None,
        &[(PRIMARY_LOG_ENV, "debug"), (LEGACY_LOG_ENV, "error")],
    );
    let bundle = TempDir::new("neutral-log-bundle");

    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    sidecar.send_json(valid_supported_request(bundle.path()));
    let error = sidecar.read_json_line();
    assert_eq!(error["type"], "error");

    let stderr = sidecar.shutdown_and_collect_stderr();
    assert!(stderr.contains("level=INFO event=startup"));
    assert!(stderr.contains("level=WARN event=request_rejected"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn legacy_log_env_remains_supported_as_a_temporary_alias() {
    let runtime_dir = TempDir::new("legacy-log-runtime");
    create_espeak_runtime(runtime_dir.path());
    let runtime_path = runtime_dir.path().to_path_buf();
    let mut sidecar = SidecarHarness::spawn_with_runtime_and_env(
        &runtime_path,
        Some(runtime_dir),
        None,
        &[(LEGACY_LOG_ENV, "debug")],
    );
    let bundle = TempDir::new("legacy-log-bundle");

    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    sidecar.send_json(valid_supported_request(bundle.path()));
    let error = sidecar.read_json_line();
    assert_eq!(error["type"], "error");

    let stderr = sidecar.shutdown_and_collect_stderr();
    assert!(stderr.contains("level=INFO event=startup"));
    assert!(stderr.contains("level=WARN event=request_rejected"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn protocol_responses_remain_piper_compatible() {
    let responses = [
        protocol_contract::TtsResponse::Ready {
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        protocol_contract::TtsResponse::Audio {
            byte_length: 24,
            sample_rate: 24000,
            channels: 1,
        },
        protocol_contract::TtsResponse::Error {
            message: "Invalid request payload: example".to_string(),
        },
    ];

    for response in responses {
        let json = serde_json::to_string(&response).expect("response should serialize");
        let parsed: PiperCompatibleResponse =
            serde_json::from_str(&json).expect("piper-compatible parser should accept response");

        match parsed {
            PiperCompatibleResponse::Ready { version } => {
                assert_eq!(version, env!("CARGO_PKG_VERSION"));
            }
            PiperCompatibleResponse::Audio {
                byte_length,
                sample_rate,
                channels,
            } => {
                assert_eq!(byte_length, 24);
                assert_eq!(sample_rate, 24000);
                assert_eq!(channels, 1);
            }
            PiperCompatibleResponse::Error { message } => {
                assert_locked_error_prefix(&message, "Invalid request payload:");
            }
        }
    }
}

#[test]
fn protocol_response_json_shapes_remain_exact() {
    let ready = serde_json::to_value(protocol_contract::TtsResponse::Ready {
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
    .expect("ready response should serialize");
    assert_eq!(
        ready,
        json!({
            "type": "ready",
            "version": env!("CARGO_PKG_VERSION"),
        })
    );

    let audio = serde_json::to_value(protocol_contract::TtsResponse::Audio {
        byte_length: 24,
        sample_rate: 24000,
        channels: 1,
    })
    .expect("audio response should serialize");
    assert_eq!(
        audio,
        json!({
            "type": "audio",
            "byte_length": 24,
            "sample_rate": 24000,
            "channels": 1,
        })
    );

    let error = serde_json::to_value(protocol_contract::TtsResponse::Error {
        message: "Synthesis failed: example".to_string(),
    })
    .expect("error response should serialize");
    assert_eq!(
        error,
        json!({
            "type": "error",
            "message": "Synthesis failed: example",
        })
    );
}

#[test]
fn closing_stdin_exits_cleanly_within_two_seconds() {
    let mut sidecar = SidecarHarness::spawn_with_log_level(Some("info"));
    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    sidecar.close_stdin();
    let exit_code = sidecar
        .wait_for_exit(Duration::from_secs(2))
        .expect("sidecar should exit after stdin closes");

    assert_eq!(exit_code, 0);

    let mut stderr = String::new();
    sidecar
        .stderr
        .read_to_string(&mut stderr)
        .expect("stderr should be readable");
    assert!(stderr.contains("event=stdin_closed"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn error_paths_never_leak_binary_payload_and_process_stays_alive() {
    let mut sidecar = SidecarHarness::spawn_with_log_level(Some("debug"));
    let synthesis_failure_bundle = TempDir::new("synthesis-failure-bundle");
    create_kokoro_bundle(synthesis_failure_bundle.path());
    let missing_bundle = TempDir::new("missing-bundle");

    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    sidecar.send_raw_line(r#"{"text":"Hello","voice":"af_heart""#);
    let invalid_json = sidecar.read_json_line();
    assert_eq!(invalid_json["type"], "error");
    assert_locked_error_prefix(
        invalid_json["message"]
            .as_str()
            .expect("message should be present"),
        "Invalid JSON request:",
    );

    sidecar.send_json(json!({
        "text": "",
        "voice": "af_heart",
        "speed": 1.0,
        "model_dir": "C:\\models\\kokoro-en",
    }));
    assert_eq!(sidecar.peek_stdout_byte(), Some(b'{'));
    let invalid_payload = sidecar.read_json_line();
    assert_eq!(invalid_payload["type"], "error");
    assert_locked_error_prefix(
        invalid_payload["message"]
            .as_str()
            .expect("message should be present"),
        "Invalid request payload:",
    );

    sidecar.send_json(valid_supported_request(missing_bundle.path()));
    assert_eq!(sidecar.peek_stdout_byte(), Some(b'{'));
    let bundle_error = sidecar.read_json_line();
    assert_eq!(bundle_error["type"], "error");
    assert_locked_error_prefix(
        bundle_error["message"]
            .as_str()
            .expect("message should be present"),
        "Invalid request payload:",
    );

    sidecar.send_json(request_for(
        "Konnichiwa from Kokoro",
        "jf_alpha",
        synthesis_failure_bundle.path(),
    ));
    assert_eq!(sidecar.peek_stdout_byte(), Some(b'{'));
    let synthesis_error = sidecar.read_json_line();
    assert_eq!(synthesis_error["type"], "error");
    assert_locked_error_prefix(
        synthesis_error["message"]
            .as_str()
            .expect("message should be present"),
        "Synthesis failed:",
    );

    sidecar.send_json(json!({
        "text": "Hello",
        "language": "en",
        "voice": "af_heart",
        "speed": 1.0,
        "model_dir": "unused"
    }));
    assert_eq!(sidecar.peek_stdout_byte(), Some(b'{'));
    let follow_up = sidecar.read_json_line();
    assert_eq!(follow_up["type"], "error");
    let message = follow_up["message"]
        .as_str()
        .expect("message should be present");
    assert_locked_error_prefix(message, "Invalid request payload:");
    assert!(message.contains("unknown field `language`"));

    sidecar.close_stdin();
    let remaining_stdout = sidecar.read_remaining_stdout();
    assert!(
        remaining_stdout.is_empty(),
        "stdout must not contain leaked binary payloads after error responses"
    );

    let stderr = sidecar.shutdown_and_collect_stderr();
    assert!(stderr.contains("category=invalid_json"));
    assert!(stderr.contains("category=invalid_request_payload"));
    assert!(stderr.contains("category=synthesis_failed"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn deferred_japanese_and_mandarin_requests_fail_cleanly_and_later_valid_request_is_processed() {
    let mut sidecar = SidecarHarness::spawn_with_log_level(Some("debug"));
    let synthesis_failure_bundle = TempDir::new("deferred-language-bundle");
    create_kokoro_bundle(synthesis_failure_bundle.path());

    let ready = sidecar.read_json_line();
    assert_eq!(ready["type"], "ready");

    for (voice, lang_code, text) in [
        ("jf_alpha", "j", "Konnichiwa from Kokoro"),
        ("zf_xiaobei", "z", "Ni hao from Kokoro"),
    ] {
        sidecar.send_json(request_for(text, voice, synthesis_failure_bundle.path()));
        assert_eq!(sidecar.peek_stdout_byte(), Some(b'{'));

        let error = sidecar.read_json_line();
        assert_eq!(error["type"], "error");

        let message = error["message"]
            .as_str()
            .expect("message should be present");
        assert_locked_error_prefix(message, "Synthesis failed:");
        assert!(message.contains(&format!(
            "phonemization is not implemented yet for lang_code='{lang_code}'"
        )));
    }

    sidecar.send_json(valid_supported_request(synthesis_failure_bundle.path()));
    assert_eq!(sidecar.peek_stdout_byte(), Some(b'{'));

    let follow_up = sidecar.read_json_line();
    assert_eq!(follow_up["type"], "error");
    assert_locked_error_prefix(
        follow_up["message"]
            .as_str()
            .expect("message should be present"),
        "Synthesis failed:",
    );

    sidecar.close_stdin();
    let remaining_stdout = sidecar.read_remaining_stdout();
    assert!(
        remaining_stdout.is_empty(),
        "stdout must not contain leaked binary payloads after deferred-language errors"
    );

    let stderr = sidecar.shutdown_and_collect_stderr();
    assert!(stderr.contains("category=synthesis_failed"));
    assert_stderr_is_plain_text(&stderr);
}
