#![cfg(windows)]

use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tiny_http::{Response, Server, StatusCode};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

const PRIMARY_KOKORO_MODEL_URL_ENV: &str = "KOKORO_TTS_RELEASE_KOKORO_MODEL_URL";
const PRIMARY_ONNXRUNTIME_URL_ENV: &str = "KOKORO_TTS_RELEASE_ONNXRUNTIME_URL";
const PRIMARY_PIPER_ZIP_URL_ENV: &str = "KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL";
const LEGACY_KOKORO_MODEL_URL_ENV: &str = "LINGOPILOT_TTS_RELEASE_KOKORO_MODEL_URL";
const LEGACY_ONNXRUNTIME_URL_ENV: &str = "LINGOPILOT_TTS_RELEASE_ONNXRUNTIME_URL";
const LEGACY_PIPER_ZIP_URL_ENV: &str = "LINGOPILOT_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL";

#[derive(Clone)]
struct ReleaseFixtureSet {
    piper_zip: Vec<u8>,
    kokoro_zip: Vec<u8>,
    onnxruntime_dll: Vec<u8>,
    espeak_dll_marker: Vec<u8>,
    model_marker: Vec<u8>,
    voices_marker: Vec<u8>,
}

impl ReleaseFixtureSet {
    fn new(prefix: &str) -> Self {
        let espeak_dll_marker = format!("{prefix}-espeak-dll").into_bytes();
        let model_marker = format!("{prefix}-kokoro-model").into_bytes();
        let voices_marker = format!("{prefix}-kokoro-voices").into_bytes();
        let onnxruntime_dll = format!("{prefix}-onnxruntime-dll").into_bytes();

        Self {
            piper_zip: create_piper_release_zip(&espeak_dll_marker),
            kokoro_zip: create_kokoro_model_zip(&model_marker, &voices_marker),
            onnxruntime_dll,
            espeak_dll_marker,
            model_marker,
            voices_marker,
        }
    }
}

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

struct StaticHttpServer {
    base_url: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl StaticHttpServer {
    fn start(routes: HashMap<String, Vec<u8>>) -> Self {
        let server = Server::http(("127.0.0.1", 0)).expect("server should bind");
        let address = server
            .server_addr()
            .to_ip()
            .expect("HTTP server should expose an IP address");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let routes = Arc::new(routes);
        let routes_for_thread = Arc::clone(&routes);

        let thread = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match server.recv_timeout(Duration::from_millis(50)) {
                    Ok(Some(request)) => handle_request(request, &routes_for_thread),
                    Ok(None) => {}
                    Err(_) => {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        });

        Self {
            base_url: format!("http://{address}"),
            stop,
            thread: Some(thread),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

impl Drop for StaticHttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn release_staging_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn release_staging_supports_neutral_env_names_end_to_end() {
    let _guard = release_staging_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspace = build_script_test_workspace();
    let fixtures = ReleaseFixtureSet::new("neutral");
    let server = start_release_fixture_server(&fixtures, None);

    run_stage_script(
        workspace.path(),
        &[
            (
                PRIMARY_KOKORO_MODEL_URL_ENV,
                server.url("/neutral/kokoro-model.zip"),
            ),
            (
                PRIMARY_ONNXRUNTIME_URL_ENV,
                server.url("/neutral/onnxruntime.dll"),
            ),
            (
                PRIMARY_PIPER_ZIP_URL_ENV,
                server.url("/neutral/piper-release.zip"),
            ),
        ],
    );

    assert_release_staging_matches(workspace.path(), &fixtures);

    run_package_script(workspace.path(), &[]);

    let dist_dir = workspace.path().join("dist");
    let zip_path = dist_dir.join("lingopilot-tts-kokoro-v0.1.0-windows-x86_64.zip");
    let checksum_path = dist_dir.join("lingopilot-tts-kokoro-v0.1.0-sha256.txt");
    assert!(
        zip_path.is_file(),
        "expected package archive at '{}'",
        zip_path.display()
    );
    assert!(
        checksum_path.is_file(),
        "expected checksum manifest at '{}'",
        checksum_path.display()
    );

    let checksum =
        fs::read_to_string(&checksum_path).expect("checksum manifest should be readable");
    assert!(
        checksum.contains("lingopilot-tts-kokoro-v0.1.0-windows-x86_64.zip"),
        "checksum manifest should reference the packaged archive"
    );
}

#[test]
fn release_staging_falls_back_to_legacy_env_names() {
    let _guard = release_staging_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspace = build_script_test_workspace();
    let fixtures = ReleaseFixtureSet::new("legacy");
    let server = start_release_fixture_server(&fixtures, None);

    run_stage_script(
        workspace.path(),
        &[
            (
                LEGACY_KOKORO_MODEL_URL_ENV,
                server.url("/neutral/kokoro-model.zip"),
            ),
            (
                LEGACY_ONNXRUNTIME_URL_ENV,
                server.url("/neutral/onnxruntime.dll"),
            ),
            (
                LEGACY_PIPER_ZIP_URL_ENV,
                server.url("/neutral/piper-release.zip"),
            ),
        ],
    );

    assert_release_staging_matches(workspace.path(), &fixtures);
}

#[test]
fn release_staging_prefers_neutral_env_names_over_legacy_aliases() {
    let _guard = release_staging_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspace = build_script_test_workspace();
    let neutral_fixtures = ReleaseFixtureSet::new("neutral-preferred");
    let legacy_fixtures = ReleaseFixtureSet::new("legacy-fallback");
    let server = start_release_fixture_server(&neutral_fixtures, Some(&legacy_fixtures));

    run_stage_script(
        workspace.path(),
        &[
            (
                PRIMARY_KOKORO_MODEL_URL_ENV,
                server.url("/neutral/kokoro-model.zip"),
            ),
            (
                PRIMARY_ONNXRUNTIME_URL_ENV,
                server.url("/neutral/onnxruntime.dll"),
            ),
            (
                PRIMARY_PIPER_ZIP_URL_ENV,
                server.url("/neutral/piper-release.zip"),
            ),
            (
                LEGACY_KOKORO_MODEL_URL_ENV,
                server.url("/legacy/kokoro-model.zip"),
            ),
            (
                LEGACY_ONNXRUNTIME_URL_ENV,
                server.url("/legacy/onnxruntime.dll"),
            ),
            (
                LEGACY_PIPER_ZIP_URL_ENV,
                server.url("/legacy/piper-release.zip"),
            ),
        ],
    );

    assert_release_staging_matches(workspace.path(), &neutral_fixtures);

    let legacy_staged_model = fs::read(
        workspace
            .path()
            .join("packaging")
            .join("windows")
            .join("kokoro-model")
            .join("kokoro-v1.0.onnx"),
    )
    .expect("staged model should be readable");
    assert_ne!(
        legacy_staged_model, legacy_fixtures.model_marker,
        "legacy model fixture must not win when neutral env vars are also set"
    );
}

fn build_script_test_workspace() -> TempDir {
    let workspace = TempDir::new("release-staging-workspace");

    copy_workspace_file(&workspace, &["Cargo.toml"]);
    copy_workspace_file(&workspace, &["README.md"]);
    copy_workspace_file(&workspace, &["LICENSE"]);
    copy_workspace_file(&workspace, &["THIRD_PARTY_LICENSES.txt"]);
    copy_workspace_file(&workspace, &["packaging", "windows", "README.md"]);
    copy_workspace_file(&workspace, &["scripts", "Stage-WindowsReleaseAssets.ps1"]);
    copy_workspace_file(&workspace, &["scripts", "Package-WindowsRelease.ps1"]);

    let release_dir = workspace.path().join("target").join("release");
    fs::create_dir_all(&release_dir).expect("release dir should be created");
    fs::write(
        release_dir.join("lingopilot-tts-kokoro.exe"),
        b"fake-sidecar-binary",
    )
    .expect("fake binary should be created");

    workspace
}

fn copy_workspace_file(workspace: &TempDir, relative_path: &[&str]) {
    let source = relative_path.iter().fold(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        |path, segment| path.join(segment),
    );
    let destination = relative_path
        .iter()
        .fold(workspace.path().to_path_buf(), |path, segment| {
            path.join(segment)
        });

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("destination parent should be created");
    }

    fs::copy(&source, &destination).unwrap_or_else(|error| {
        panic!(
            "failed to copy '{}' to '{}': {error}",
            source.display(),
            destination.display()
        )
    });
}

fn start_release_fixture_server(
    neutral: &ReleaseFixtureSet,
    legacy: Option<&ReleaseFixtureSet>,
) -> StaticHttpServer {
    let mut routes = HashMap::from([
        (
            "/neutral/piper-release.zip".to_string(),
            neutral.piper_zip.clone(),
        ),
        (
            "/neutral/kokoro-model.zip".to_string(),
            neutral.kokoro_zip.clone(),
        ),
        (
            "/neutral/onnxruntime.dll".to_string(),
            neutral.onnxruntime_dll.clone(),
        ),
    ]);

    if let Some(legacy) = legacy {
        routes.insert(
            "/legacy/piper-release.zip".to_string(),
            legacy.piper_zip.clone(),
        );
        routes.insert(
            "/legacy/kokoro-model.zip".to_string(),
            legacy.kokoro_zip.clone(),
        );
        routes.insert(
            "/legacy/onnxruntime.dll".to_string(),
            legacy.onnxruntime_dll.clone(),
        );
    }

    StaticHttpServer::start(routes)
}

fn handle_request(request: tiny_http::Request, routes: &HashMap<String, Vec<u8>>) {
    let path = request.url().to_string();

    if let Some(body) = routes.get(&path) {
        let response = Response::from_data(body.clone()).with_status_code(StatusCode(200));
        let _ = request.respond(response);
        return;
    }

    let response = Response::from_string("not found").with_status_code(StatusCode(404));
    let _ = request.respond(response);
}

fn run_stage_script(workspace_root: &Path, envs: &[(&str, String)]) {
    run_powershell_script(
        workspace_root,
        workspace_root
            .join("scripts")
            .join("Stage-WindowsReleaseAssets.ps1"),
        envs,
    );
}

fn run_package_script(workspace_root: &Path, envs: &[(&str, String)]) {
    run_powershell_script(
        workspace_root,
        workspace_root
            .join("scripts")
            .join("Package-WindowsRelease.ps1"),
        envs,
    );
}

fn run_powershell_script(workspace_root: &Path, script_path: PathBuf, envs: &[(&str, String)]) {
    let output = Command::new(powershell_binary())
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(script_path)
        .current_dir(workspace_root)
        .env_remove(PRIMARY_KOKORO_MODEL_URL_ENV)
        .env_remove(PRIMARY_ONNXRUNTIME_URL_ENV)
        .env_remove(PRIMARY_PIPER_ZIP_URL_ENV)
        .env_remove(LEGACY_KOKORO_MODEL_URL_ENV)
        .env_remove(LEGACY_ONNXRUNTIME_URL_ENV)
        .env_remove(LEGACY_PIPER_ZIP_URL_ENV)
        .envs(envs.iter().map(|(key, value)| (*key, value)))
        .output()
        .expect("PowerShell script should run");

    assert_command_success(output);
}

fn powershell_binary() -> &'static str {
    static POWERSHELL: OnceLock<&'static str> = OnceLock::new();

    POWERSHELL.get_or_init(|| {
        for candidate in ["pwsh", "powershell"] {
            let available = Command::new(candidate)
                .arg("-NoProfile")
                .arg("-Command")
                .arg("$PSVersionTable.PSVersion.ToString()")
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false);
            if available {
                return candidate;
            }
        }

        panic!("no PowerShell executable was found in PATH");
    })
}

fn assert_command_success(output: Output) {
    if output.status.success() {
        return;
    }

    panic!(
        "script failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_release_staging_matches(workspace_root: &Path, fixtures: &ReleaseFixtureSet) {
    let runtime_root = workspace_root
        .join("target")
        .join("release")
        .join("espeak-runtime");
    let model_root = workspace_root
        .join("packaging")
        .join("windows")
        .join("kokoro-model");

    assert!(
        runtime_root.join("espeak-ng-data").is_dir(),
        "expected staged espeak-ng-data directory at '{}'",
        runtime_root.join("espeak-ng-data").display()
    );
    assert_eq!(
        fs::read(runtime_root.join("espeak-ng.dll")).expect("staged espeak dll should be readable"),
        fixtures.espeak_dll_marker
    );
    assert_eq!(
        fs::read(model_root.join("kokoro-v1.0.onnx")).expect("staged model should be readable"),
        fixtures.model_marker
    );
    assert_eq!(
        fs::read(model_root.join("voices-v1.0.bin")).expect("staged voices should be readable"),
        fixtures.voices_marker
    );
    assert_eq!(
        fs::read(
            workspace_root
                .join("packaging")
                .join("windows")
                .join("onnxruntime.dll")
        )
        .expect("staged onnxruntime dll should be readable"),
        fixtures.onnxruntime_dll
    );
}

fn create_piper_release_zip(espeak_dll_marker: &[u8]) -> Vec<u8> {
    let mut entries = vec![
        (
            "lingopilot-tts-piper-v0.1.0-windows-x86_64/espeak-runtime/espeak-ng.dll",
            espeak_dll_marker.to_vec(),
        ),
        (
            "lingopilot-tts-piper-v0.1.0-windows-x86_64/espeak-runtime/runtime-note.txt",
            b"fixture".to_vec(),
        ),
    ];
    entries.push((
        "lingopilot-tts-piper-v0.1.0-windows-x86_64/espeak-runtime/espeak-ng-data/voices.txt",
        b"fixture".to_vec(),
    ));

    create_zip(entries)
}

fn create_kokoro_model_zip(model_marker: &[u8], voices_marker: &[u8]) -> Vec<u8> {
    create_zip(vec![
        ("kokoro-model/kokoro-v1.0.onnx", model_marker.to_vec()),
        ("kokoro-model/voices-v1.0.bin", voices_marker.to_vec()),
    ])
}

fn create_zip(entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = FileOptions::default().compression_method(CompressionMethod::Stored);

    for (path, bytes) in entries {
        writer
            .start_file(path, options)
            .expect("zip entry should start");
        writer
            .write_all(&bytes)
            .expect("zip entry contents should be written");
    }

    writer
        .finish()
        .expect("zip writer should finish")
        .into_inner()
}
