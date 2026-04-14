use std::path::PathBuf;

#[allow(dead_code)]
pub(crate) const LIVE_TEST_IGNORE_REASON: &str =
    "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets";

pub(crate) struct LiveTestAssets {
    pub(crate) espeak_runtime_dir: PathBuf,
    pub(crate) model_dir: PathBuf,
    pub(crate) onnxruntime_dll: PathBuf,
}

impl LiveTestAssets {
    pub(crate) fn from_env() -> Self {
        let espeak_runtime_dir = require_absolute_dir_env(
            "LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR",
            Some("espeak-ng-data"),
        );
        let model_dir = require_absolute_dir_env("LINGOPILOT_TTS_LIVE_MODEL_DIR", None);
        let onnxruntime_dll = require_absolute_file_env("LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL");

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

fn require_absolute_dir_env(key: &str, required_child: Option<&str>) -> PathBuf {
    let path = require_absolute_path_env(key);

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

fn require_absolute_file_env(key: &str) -> PathBuf {
    let path = require_absolute_path_env(key);

    if !path.is_file() {
        panic!(
            "{key} must point to an existing file, got '{}'",
            path.display()
        );
    }

    path
}

fn require_absolute_path_env(key: &str) -> PathBuf {
    let raw = std::env::var(key)
        .unwrap_or_else(|_| panic!("{key} must be set to an absolute existing path"));
    let path = PathBuf::from(&raw);

    if !path.is_absolute() {
        panic!("{key} must be an absolute path, got '{raw}'");
    }

    path
}
