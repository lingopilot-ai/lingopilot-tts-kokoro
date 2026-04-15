use std::path::PathBuf;

#[allow(dead_code)]
pub(crate) const LIVE_TEST_IGNORE_REASON: &str =
    "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets";
const PRIMARY_LIVE_ESPEAK_RUNTIME_ENV: &str = "KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR";
const LEGACY_LIVE_ESPEAK_RUNTIME_ENV: &str = "LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR";
const PRIMARY_LIVE_MODEL_ENV: &str = "KOKORO_TTS_LIVE_MODEL_DIR";
const LEGACY_LIVE_MODEL_ENV: &str = "LINGOPILOT_TTS_LIVE_MODEL_DIR";
const PRIMARY_LIVE_ONNXRUNTIME_ENV: &str = "KOKORO_TTS_LIVE_ONNXRUNTIME_DLL";
const LEGACY_LIVE_ONNXRUNTIME_ENV: &str = "LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL";

pub(crate) struct LiveTestAssets {
    pub(crate) espeak_runtime_dir: PathBuf,
    pub(crate) model_dir: PathBuf,
    pub(crate) onnxruntime_dll: PathBuf,
}

impl LiveTestAssets {
    pub(crate) fn from_env() -> Self {
        let espeak_runtime_dir = require_absolute_dir_env(
            PRIMARY_LIVE_ESPEAK_RUNTIME_ENV,
            Some(LEGACY_LIVE_ESPEAK_RUNTIME_ENV),
            Some("espeak-ng-data"),
        );
        let model_dir =
            require_absolute_dir_env(PRIMARY_LIVE_MODEL_ENV, Some(LEGACY_LIVE_MODEL_ENV), None);
        let onnxruntime_dll = require_absolute_file_env(
            PRIMARY_LIVE_ONNXRUNTIME_ENV,
            Some(LEGACY_LIVE_ONNXRUNTIME_ENV),
        );

        Self {
            espeak_runtime_dir,
            model_dir,
            onnxruntime_dll,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn install_onnxruntime_env(&self) {
        std::env::set_var("ORT_DYLIB_PATH", &self.onnxruntime_dll);
    }
}

fn require_absolute_dir_env(
    key: &str,
    legacy_key: Option<&str>,
    required_child: Option<&str>,
) -> PathBuf {
    let path = require_absolute_path_env(key, legacy_key);

    if !path.is_dir() {
        panic!(
            "{key} must point to an existing directory, got '{}'",
            path.display()
        );
    }

    if let Some(child) = required_child {
        let child_path = path.join(child);
        if !child_path.is_dir() {
            panic!(
                "{key} must contain '{}', missing '{}'",
                child,
                child_path.display()
            );
        }
    }

    path
}

fn require_absolute_file_env(key: &str, legacy_key: Option<&str>) -> PathBuf {
    let path = require_absolute_path_env(key, legacy_key);

    if !path.is_file() {
        panic!(
            "{key} must point to an existing file, got '{}'",
            path.display()
        );
    }

    path
}

fn require_absolute_path_env(key: &str, legacy_key: Option<&str>) -> PathBuf {
    let raw = resolve_env_value(key, legacy_key).unwrap_or_else(|message| panic!("{message}"));
    let path = PathBuf::from(&raw);

    if !path.is_absolute() {
        panic!("{key} must be an absolute path, got '{raw}'");
    }

    path
}

fn resolve_env_value(key: &str, legacy_key: Option<&str>) -> Result<String, String> {
    if let Ok(value) = std::env::var(key) {
        return Ok(value);
    }

    if let Some(legacy_key) = legacy_key {
        if let Ok(value) = std::env::var(legacy_key) {
            return Ok(value);
        }

        return Err(format!(
            "{key} must be set to an absolute existing path (legacy alias: {legacy_key})"
        ));
    }

    Err(format!("{key} must be set to an absolute existing path"))
}

#[cfg(test)]
mod tests {
    use super::{
        LiveTestAssets, LEGACY_LIVE_ESPEAK_RUNTIME_ENV, LEGACY_LIVE_MODEL_ENV,
        LEGACY_LIVE_ONNXRUNTIME_ENV, PRIMARY_LIVE_ESPEAK_RUNTIME_ENV, PRIMARY_LIVE_MODEL_ENV,
        PRIMARY_LIVE_ONNXRUNTIME_ENV,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
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
                "lingopilot-tts-kokoro-live-env-{prefix}-{}-{nonce}",
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

    fn set_live_env(key: &str, value: &Path) {
        std::env::set_var(key, value);
    }

    fn clear_live_envs() {
        for key in [
            PRIMARY_LIVE_ESPEAK_RUNTIME_ENV,
            LEGACY_LIVE_ESPEAK_RUNTIME_ENV,
            PRIMARY_LIVE_MODEL_ENV,
            LEGACY_LIVE_MODEL_ENV,
            PRIMARY_LIVE_ONNXRUNTIME_ENV,
            LEGACY_LIVE_ONNXRUNTIME_ENV,
            "ORT_DYLIB_PATH",
        ] {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn live_assets_prefer_neutral_env_names_when_both_are_set() {
        let _guard = env_lock().lock().expect("env lock should be available");
        clear_live_envs();

        let primary_runtime = TempDir::new("primary-runtime");
        let legacy_runtime = TempDir::new("legacy-runtime");
        let primary_model = TempDir::new("primary-model");
        let legacy_model = TempDir::new("legacy-model");
        let temp_file_dir = TempDir::new("dll-dir");
        let primary_dll = temp_file_dir.path().join("primary-onnxruntime.dll");
        let legacy_dll = temp_file_dir.path().join("legacy-onnxruntime.dll");

        fs::create_dir(primary_runtime.path().join("espeak-ng-data"))
            .expect("primary runtime data dir should exist");
        fs::create_dir(legacy_runtime.path().join("espeak-ng-data"))
            .expect("legacy runtime data dir should exist");
        fs::write(&primary_dll, b"primary").expect("primary dll should exist");
        fs::write(&legacy_dll, b"legacy").expect("legacy dll should exist");

        set_live_env(PRIMARY_LIVE_ESPEAK_RUNTIME_ENV, primary_runtime.path());
        set_live_env(LEGACY_LIVE_ESPEAK_RUNTIME_ENV, legacy_runtime.path());
        set_live_env(PRIMARY_LIVE_MODEL_ENV, primary_model.path());
        set_live_env(LEGACY_LIVE_MODEL_ENV, legacy_model.path());
        set_live_env(PRIMARY_LIVE_ONNXRUNTIME_ENV, &primary_dll);
        set_live_env(LEGACY_LIVE_ONNXRUNTIME_ENV, &legacy_dll);

        let assets = LiveTestAssets::from_env();

        assert_eq!(assets.espeak_runtime_dir, primary_runtime.path());
        assert_eq!(assets.model_dir, primary_model.path());
        assert_eq!(assets.onnxruntime_dll, primary_dll);

        clear_live_envs();
    }

    #[test]
    fn live_assets_fall_back_to_legacy_env_names() {
        let _guard = env_lock().lock().expect("env lock should be available");
        clear_live_envs();

        let runtime = TempDir::new("legacy-runtime-only");
        let model = TempDir::new("legacy-model-only");
        let temp_file_dir = TempDir::new("legacy-dll-dir");
        let onnxruntime_dll = temp_file_dir.path().join("onnxruntime.dll");

        fs::create_dir(runtime.path().join("espeak-ng-data"))
            .expect("runtime data dir should exist");
        fs::write(&onnxruntime_dll, b"legacy").expect("dll should exist");

        set_live_env(LEGACY_LIVE_ESPEAK_RUNTIME_ENV, runtime.path());
        set_live_env(LEGACY_LIVE_MODEL_ENV, model.path());
        set_live_env(LEGACY_LIVE_ONNXRUNTIME_ENV, &onnxruntime_dll);

        let assets = LiveTestAssets::from_env();

        assert_eq!(assets.espeak_runtime_dir, runtime.path());
        assert_eq!(assets.model_dir, model.path());
        assert_eq!(assets.onnxruntime_dll, onnxruntime_dll);

        clear_live_envs();
    }
}
