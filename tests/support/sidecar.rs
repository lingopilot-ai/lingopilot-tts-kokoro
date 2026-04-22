use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// Kill a child process by PID cross-platform. Used as the watchdog's last
/// resort so a blocked read in the test thread can unwind instead of hanging.
fn kill_pid(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

pub struct DeadlineGuard {
    cancelled: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl DeadlineGuard {
    pub fn cancel(mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for DeadlineGuard {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

const PRIMARY_LOG_ENV: &str = "KOKORO_TTS_LOG";
const LEGACY_LOG_ENV: &str = "LINGOPILOT_TTS_LOG";

pub struct SidecarHarness {
    child: Child,
    stdout: BufReader<ChildStdout>,
    stderr_buf: Arc<Mutex<String>>,
    stderr_thread: Option<JoinHandle<()>>,
}

fn spawn_stderr_drain(stderr: ChildStderr) -> (Arc<Mutex<String>>, JoinHandle<()>) {
    let buf = Arc::new(Mutex::new(String::new()));
    let thread_buf = buf.clone();
    let handle = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if let Ok(mut guard) = thread_buf.lock() {
                        guard.push_str(&line);
                    }
                }
                Err(_) => break,
            }
        }
    });
    (buf, handle)
}

impl SidecarHarness {
    pub fn spawn_with_runtime_and_env(
        runtime_dir: &Path,
        level: Option<&str>,
        extra_env: &[(&str, &str)],
    ) -> Self {
        Self::spawn_with_dirs_and_env(runtime_dir, None, level, extra_env)
    }

    pub fn spawn_with_dirs_and_env(
        runtime_dir: &Path,
        model_dir: Option<&Path>,
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

        command.arg("--espeak-data-dir").arg(runtime_dir);
        if let Some(model_dir) = model_dir {
            command.arg("--model-dir").arg(model_dir);
        }

        let mut child = command.spawn().expect("sidecar should start");

        let stdout = child.stdout.take().expect("stdout should be piped");
        let stderr = child.stderr.take().expect("stderr should be piped");
        let (stderr_buf, stderr_thread) = spawn_stderr_drain(stderr);

        Self {
            child,
            stdout: BufReader::new(stdout),
            stderr_buf,
            stderr_thread: Some(stderr_thread),
        }
    }

    /// Arm a watchdog that kills the sidecar process if `timeout` elapses
    /// before the guard is cancelled or dropped. Without this, a hung read
    /// on stdout would block the test thread indefinitely (see 054d160
    /// regression post-mortem).
    pub fn arm_deadline(&self, timeout: Duration) -> DeadlineGuard {
        let pid = self.child.id();
        let cancelled = Arc::new(AtomicBool::new(false));
        let thread_cancelled = cancelled.clone();
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                if thread_cancelled.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            if !thread_cancelled.load(Ordering::Relaxed) {
                eprintln!(
                    "[sidecar-watchdog] killing pid={pid} after {:?} deadline",
                    timeout
                );
                kill_pid(pid);
            }
        });
        DeadlineGuard {
            cancelled,
            handle: Some(handle),
        }
    }

    pub fn stderr_snapshot(&self) -> String {
        self.stderr_buf
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
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
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
        self.stderr_snapshot()
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
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
    }
}

/// Build a synthesize-op request for the integration test harness.
///
/// Note: `_model_dir` is ignored on the wire (the sidecar now resolves its
/// model dir at startup) but kept in the signature so callers can continue to
/// pass asset paths without restructuring.
pub fn request_for(text: &str, voice: &str, _model_dir: &Path) -> Value {
    json!({
        "op": "synthesize",
        "id": default_id(),
        "text": text,
        "voice_id": voice,
        "speed": 1.0,
    })
}

fn default_id() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("req-{nonce}")
}

pub fn assert_stderr_is_plain_text(stderr: &str) {
    assert!(!stderr.contains('\0'), "stderr must not contain NUL bytes");
    assert!(
        !stderr.contains("{\"op\""),
        "stderr must not contain protocol JSON"
    );
    assert!(
        !stderr.contains("{\"type\""),
        "stderr must not contain legacy protocol JSON"
    );
}

#[allow(dead_code)]
pub fn unique_missing_path(prefix: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    std::env::temp_dir().join(format!(
        "lingopilot-tts-kokoro-{prefix}-{}-{nonce}-{seq}",
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
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let path = std::env::temp_dir().join(format!(
            "lingopilot-tts-kokoro-{prefix}-{}-{nonce}-{seq}",
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
