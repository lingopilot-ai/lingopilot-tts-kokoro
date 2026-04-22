use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

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
    _model_dir: Option<TempDir>,
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
        let model_dir = TempDir::new("model");

        let mut command = sidecar_command();
        if let Some(level) = level {
            command.env(PRIMARY_LOG_ENV, level);
        }
        command
            .arg("--espeak-data-dir")
            .arg(runtime_dir.path())
            .arg("--model-dir")
            .arg(model_dir.path());

        let mut child = command.spawn().expect("sidecar should start");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        Self {
            _runtime_dir: Some(runtime_dir),
            _model_dir: Some(model_dir),
            child,
            stdout: BufReader::new(stdout),
            stderr: BufReader::new(stderr),
        }
    }

    fn send_json(&mut self, value: Value) {
        let stdin = self.child.stdin.as_mut().expect("stdin piped");
        writeln!(stdin, "{value}").expect("request should be written");
        stdin.flush().expect("stdin should flush");
    }

    fn send_raw_line(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().expect("stdin piped");
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

    fn peek_stdout_byte(&mut self) -> Option<u8> {
        self.stdout
            .fill_buf()
            .expect("stdout should be readable")
            .first()
            .copied()
    }

    fn read_remaining_stdout(&mut self) -> String {
        let mut remaining = String::new();
        self.stdout
            .read_to_string(&mut remaining)
            .expect("stdout should be readable until EOF");
        remaining
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

fn synthesize_request(id: &str, text: &str, voice: &str) -> Value {
    json!({
        "op": "synthesize",
        "id": id,
        "text": text,
        "voice_id": voice,
        "speed": 1.0,
    })
}

fn assert_stderr_is_plain_text(stderr: &str) {
    assert!(!stderr.contains('\0'), "stderr must not contain NUL bytes");
    assert!(
        !stderr.contains("{\"op\""),
        "stderr must not contain protocol JSON"
    );
    assert!(
        !stderr.contains("{\"type\""),
        "stderr must not contain legacy protocol JSON"
    );
    assert!(
        !stderr.contains("Missing required startup argument"),
        "stderr must not reference removed startup-argument error"
    );
}

fn assert_ready(ready: &Value) {
    assert_eq!(ready["op"], "ready");
    assert_eq!(ready["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(ready["sample_rate"], 24000);
    assert_eq!(ready["channels"], 1);
    assert_eq!(ready["encoding"], "pcm16le");
}

#[test]
fn valid_startup_emits_directive_conformant_ready() {
    let mut sidecar = SidecarHarness::spawn();
    let ready = sidecar.read_json_line();
    assert_ready(&ready);

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
    assert!(output.stdout.is_empty(), "no ready on stdout");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Startup error: Unknown startup argument"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn startup_missing_espeak_path_exits_without_ready() {
    let model_dir = TempDir::new("missing-espeak-model");
    let output = sidecar_command()
        .arg("--espeak-data-dir")
        .arg(unique_missing_path("missing-espeak"))
        .arg("--model-dir")
        .arg(model_dir.path())
        .output()
        .expect("sidecar should run to completion");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Startup error: Cannot use eSpeak data dir"));
    assert_stderr_is_plain_text(&stderr);
}

#[test]
fn malformed_json_returns_bad_request_and_process_stays_alive() {
    let mut sidecar = SidecarHarness::spawn();
    assert_ready(&sidecar.read_json_line());

    sidecar.send_raw_line(r#"{"op":"synthesize","id":"r1""#);
    let error = sidecar.read_json_line();
    assert_eq!(error["op"], "error");
    assert_eq!(error["kind"], "bad_request");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("Invalid JSON request:")
    );

    sidecar.send_json(synthesize_request("r2", "", "af_heart"));
    let follow_up = sidecar.read_json_line();
    assert_eq!(follow_up["op"], "error");
    assert_eq!(follow_up["kind"], "bad_request");
    assert_eq!(follow_up["id"], "r2");
}

#[test]
fn reserved_ops_are_rejected_with_id_echo() {
    let mut sidecar = SidecarHarness::spawn();
    assert_ready(&sidecar.read_json_line());

    for op in ["audio_chunk", "cancel"] {
        sidecar.send_json(json!({ "op": op, "id": "reserved-1" }));
        let err = sidecar.read_json_line();
        assert_eq!(err["op"], "error");
        assert_eq!(err["kind"], "bad_request");
        assert_eq!(err["id"], "reserved-1");
        assert!(err["message"]
            .as_str()
            .unwrap()
            .contains("op not supported in this version"));
    }
}

#[test]
fn unknown_voice_is_rejected_with_unknown_voice_kind() {
    let mut sidecar = SidecarHarness::spawn();
    assert_ready(&sidecar.read_json_line());

    sidecar.send_json(synthesize_request("v1", "Hi", "xx_unknown"));
    let err = sidecar.read_json_line();
    assert_eq!(err["op"], "error");
    assert_eq!(err["kind"], "unknown_voice");
    assert_eq!(err["id"], "v1");
}

#[test]
fn invalid_semantic_payload_echoes_id_and_keeps_process_alive() {
    let mut sidecar = SidecarHarness::spawn();
    assert_ready(&sidecar.read_json_line());

    for (req, fragment) in [
        (
            synthesize_request("s1", "", "af_heart"),
            "text must not be empty",
        ),
        (
            synthesize_request("s2", "Hi", "   "),
            "voice_id must not be empty",
        ),
        (
            json!({"op":"synthesize","id":"s3","text":"Hi","voice_id":"af_heart","speed":2.5}),
            "between 0.5 and 2",
        ),
    ] {
        sidecar.send_json(req);
        let err = sidecar.read_json_line();
        assert_eq!(err["op"], "error");
        assert_eq!(err["kind"], "bad_request");
        assert!(err["message"].as_str().unwrap().contains(fragment));
    }
}

#[test]
fn missing_kokoro_assets_return_bad_request_before_synthesis() {
    let mut sidecar = SidecarHarness::spawn();
    assert_ready(&sidecar.read_json_line());

    sidecar.send_json(synthesize_request("req-miss", "Hi", "af_heart"));
    let err = sidecar.read_json_line();
    assert_eq!(err["op"], "error");
    assert_eq!(err["kind"], "bad_request");
    assert_eq!(err["id"], "req-miss");
    assert!(err["message"]
        .as_str()
        .unwrap()
        .contains("expected exactly one Kokoro model"));
}

#[test]
fn closing_stdin_exits_cleanly_within_two_seconds() {
    let mut sidecar = SidecarHarness::spawn_with_log_level(Some("info"));
    assert_ready(&sidecar.read_json_line());
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
fn error_paths_never_leak_binary_and_process_stays_alive() {
    let mut sidecar = SidecarHarness::spawn_with_log_level(Some("debug"));
    assert_ready(&sidecar.read_json_line());

    sidecar.send_raw_line(r#"{"op":"synthesize","id":"e1""#);
    let err1 = sidecar.read_json_line();
    assert_eq!(err1["op"], "error");

    sidecar.send_json(synthesize_request("e2", "", "af_heart"));
    assert_eq!(sidecar.peek_stdout_byte(), Some(b'{'));
    let err2 = sidecar.read_json_line();
    assert_eq!(err2["op"], "error");
    assert_eq!(err2["id"], "e2");

    sidecar.send_json(synthesize_request("e3", "Hi", "af_heart"));
    assert_eq!(sidecar.peek_stdout_byte(), Some(b'{'));
    let err3 = sidecar.read_json_line();
    assert_eq!(err3["op"], "error");
    assert_eq!(err3["id"], "e3");
    assert_eq!(err3["kind"], "bad_request");

    sidecar.close_stdin();
    let remaining = sidecar.read_remaining_stdout();
    assert!(remaining.is_empty());

    let stderr = sidecar.shutdown_and_collect_stderr();
    assert!(stderr.contains("category=invalid_json"));
    assert!(stderr.contains("category=invalid_request_payload"));
    assert_stderr_is_plain_text(&stderr);
}
