use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const PRIMARY_LOG_ENV: &str = "KOKORO_TTS_LOG";
const LEGACY_LOG_ENV: &str = "LINGOPILOT_TTS_LOG";

pub struct SidecarHarness {
    child: Child,
    stdout: BufReader<ChildStdout>,
    stderr: BufReader<ChildStderr>,
}

impl SidecarHarness {
    pub fn spawn_with_runtime_and_env(
        runtime_dir: &Path,
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
            child,
            stdout: BufReader::new(stdout),
            stderr: BufReader::new(stderr),
        }
    }

    pub fn send_json(&mut self, value: Value) {
        let stdin = self.child.stdin.as_mut().expect("stdin should be piped");
        writeln!(stdin, "{value}").expect("request should be written");
        stdin.flush().expect("stdin should flush");
    }

    pub fn read_json_line(&mut self) -> Value {
        let mut line = String::new();
        let bytes = self
            .stdout
            .read_line(&mut line)
            .expect("stdout should be readable");
        assert!(bytes > 0, "expected a JSON line from the sidecar");

        serde_json::from_str(line.trim_end()).expect("sidecar should emit valid JSON")
    }

    pub fn close_stdin(&mut self) {
        let _ = self.child.stdin.take();
    }

    pub fn read_exact_stdout_bytes(&mut self, byte_length: usize) -> Vec<u8> {
        let mut bytes = vec![0_u8; byte_length];
        self.stdout
            .read_exact(&mut bytes)
            .expect("stdout should contain the expected PCM bytes");
        bytes
    }

    #[allow(dead_code)]
    pub fn peek_stdout_byte(&mut self) -> Option<u8> {
        self.stdout
            .fill_buf()
            .expect("stdout should be readable")
            .first()
            .copied()
    }

    pub fn read_remaining_stdout(&mut self) -> String {
        let mut remaining = String::new();
        self.stdout
            .read_to_string(&mut remaining)
            .expect("stdout should be readable until EOF");
        remaining
    }

    pub fn shutdown_and_collect_stderr(&mut self) -> String {
        self.close_stdin();
        let _ = self.child.wait();

        let mut stderr = String::new();
        self.stderr
            .read_to_string(&mut stderr)
            .expect("stderr should be readable");
        stderr
    }

    #[allow(dead_code)]
    pub fn wait_for_exit(&mut self, timeout: Duration) -> Option<i32> {
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

pub fn request_for(text: &str, voice: &str, model_dir: &Path) -> Value {
    json!({
        "text": text,
        "voice": voice,
        "speed": 1.0,
        "model_dir": model_dir,
    })
}

pub fn assert_stderr_is_plain_text(stderr: &str) {
    assert!(!stderr.contains('\0'), "stderr must not contain NUL bytes");
    assert!(
        !stderr.contains("{\"type\""),
        "stderr must not contain protocol JSON"
    );
}

#[allow(dead_code)]
pub fn unique_missing_path(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();

    std::env::temp_dir().join(format!(
        "lingopilot-tts-kokoro-{prefix}-{}-{nonce}",
        std::process::id()
    ))
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

#[allow(dead_code)]
pub struct TempDir {
    path: PathBuf,
}

#[allow(dead_code)]
impl TempDir {
    pub fn new(prefix: &str) -> Self {
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

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
