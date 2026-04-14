mod kokoro_vocab;
#[cfg(test)]
mod live_test_support;
mod protocol;
mod synthesis;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use protocol::{TtsRequest, TtsResponse};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer as FormatWriter;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ESPEAK_DATA_ENV: &str = "LINGOPILOT_TTS_ESPEAKNG_DATA_DIRECTORY";

#[derive(Debug, PartialEq, Eq)]
struct StartupConfig {
    espeak_data_dir: PathBuf,
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

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LINGOPILOT_TTS_LOG").unwrap_or_else(
                |_| {
                    tracing_subscriber::EnvFilter::try_from_env("RUST_LOG")
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"))
                },
            ),
        )
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

    std::env::set_var(ESPEAK_DATA_ENV, &startup.espeak_data_dir);
    tracing::info!(
        event = "espeak_runtime_selected",
        espeak_data_dir = startup.espeak_data_dir.display().to_string()
    );

    if !send_response(
        &TtsResponse::Ready {
            version: VERSION.to_string(),
        },
        "ready",
    ) {
        return ExitCode::FAILURE;
    }

    let mut synthesis_cache = synthesis::SynthesisCache::new(startup.espeak_data_dir.clone());
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(error) => {
                tracing::error!(event = "stdin_read_failed", error = error.to_string());
                break;
            }
        };

        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let request: TtsRequest = match parse_request(&line) {
            Ok(r) => r,
            Err(message) => {
                let category = if message.starts_with("Invalid JSON request:") {
                    "invalid_json"
                } else {
                    "invalid_request_payload"
                };
                tracing::warn!(
                    event = "request_rejected",
                    category,
                    line_len = line.chars().count(),
                    detail = message.as_str()
                );
                let _ = send_response(&TtsResponse::Error { message }, "error");
                continue;
            }
        };

        handle_request(&mut synthesis_cache, request);
    }

    tracing::info!(event = "stdin_closed");
    ExitCode::SUCCESS
}

fn handle_request(synthesis_cache: &mut synthesis::SynthesisCache, req: TtsRequest) {
    let text_len = req.text.chars().count();
    tracing::debug!(
        event = "request_received",
        voice = req.voice.as_str(),
        speed = req.speed as f64,
        text_len
    );

    let model_dir = Path::new(&req.model_dir);
    if let Err(message) = synthesis::validate_model_dir(model_dir) {
        tracing::warn!(
            event = "request_rejected",
            category = "invalid_request_payload",
            voice = req.voice.as_str(),
            speed = req.speed as f64,
            text_len,
            detail = message.as_str()
        );
        let _ = send_response(
            &TtsResponse::Error {
                message: format!("Invalid request payload: {}", message),
            },
            "error",
        );
        return;
    }

    let assets = match synthesis::resolve_model_assets(model_dir, &req.voice) {
        Ok(assets) => assets,
        Err(error) => {
            tracing::warn!(
                event = "request_rejected",
                category = "invalid_request_payload",
                voice = req.voice.as_str(),
                speed = req.speed as f64,
                text_len,
                detail = error.as_str()
            );
            let _ = send_response(
                &TtsResponse::Error {
                    message: format!("Invalid request payload: {}", error),
                },
                "error",
            );
            return;
        }
    };

    tracing::debug!(
        event = "kokoro_assets_resolved",
        voice = req.voice.as_str(),
        lang_code = assets.voice.lang_code,
        british = assets.voice.british,
        espeak_voice = assets.voice.espeak_voice.unwrap_or("none"),
        model_path = assets.model_path.display().to_string(),
        voices_path = assets.voices_path.display().to_string()
    );

    match synthesis_cache.synthesize(&req.text, &assets, req.speed) {
        Ok(result) => {
            let byte_len = (result.pcm16.len() * 2) as u32;

            if !send_response(
                &TtsResponse::Audio {
                    byte_length: byte_len,
                    sample_rate: result.sample_rate,
                    channels: 1,
                },
                "audio",
            ) {
                return;
            }

            let stdout = io::stdout();
            let mut out = stdout.lock();
            for sample in &result.pcm16 {
                let bytes = sample.to_le_bytes();
                if let Err(error) = out.write_all(&bytes) {
                    tracing::error!(
                        event = "stdout_write_failed",
                        stage = "audio_bytes",
                        error = error.to_string()
                    );
                    return;
                }
            }
            if let Err(error) = out.flush() {
                tracing::error!(
                    event = "stdout_flush_failed",
                    stage = "audio_bytes",
                    error = error.to_string()
                );
                return;
            }
            tracing::debug!(
                event = "request_succeeded",
                voice = req.voice.as_str(),
                speed = req.speed as f64,
                text_len,
                sample_rate = result.sample_rate as u64,
                byte_length = byte_len as u64
            );
        }
        Err(error) => {
            tracing::warn!(
                event = "request_failed",
                category = "synthesis_failed",
                voice = req.voice.as_str(),
                speed = req.speed as f64,
                text_len,
                detail = error.as_str()
            );
            let _ = send_response(
                &TtsResponse::Error {
                    message: format!("Synthesis failed: {}", error),
                },
                "error",
            );
        }
    }
}

fn parse_request(line: &str) -> Result<TtsRequest, String> {
    let request: TtsRequest = serde_json::from_str(line).map_err(|error| {
        if error.is_syntax() || error.is_eof() {
            format!("Invalid JSON request: {}", error)
        } else {
            format!("Invalid request payload: {}", error)
        }
    })?;

    request
        .validate()
        .map_err(|error| format!("Invalid request payload: {}", error))?;

    Ok(request)
}

fn load_startup_config<I, S>(args: I) -> Result<StartupConfig, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let config = parse_startup_config(args)?;
    synthesis::validate_espeak_data_dir(&config.espeak_data_dir)?;
    Ok(config)
}

fn parse_startup_config<I, S>(args: I) -> Result<StartupConfig, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let _binary = args.next();

    let mut espeak_data_dir: Option<PathBuf> = None;

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

        return Err(format!(
            "Unknown startup argument: {}",
            arg.to_string_lossy()
        ));
    }

    let Some(espeak_data_dir) = espeak_data_dir else {
        return Err("Missing required startup argument: --espeak-data-dir <path>".to_string());
    };

    Ok(StartupConfig { espeak_data_dir })
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
    use super::{parse_request, parse_startup_config, StartupConfig};
    use std::path::PathBuf;

    #[test]
    fn startup_config_requires_espeak_data_dir_flag() {
        let error = parse_startup_config(["lingopilot-tts-kokoro"])
            .expect_err("startup config should require the espeak flag");

        assert!(error.contains("--espeak-data-dir"));
    }

    #[test]
    fn startup_config_accepts_required_espeak_data_dir_flag() {
        let config = parse_startup_config([
            "lingopilot-tts-kokoro",
            "--espeak-data-dir",
            "C:\\runtime\\espeak-runtime",
        ])
        .expect("startup config should parse");

        assert_eq!(
            config,
            StartupConfig {
                espeak_data_dir: PathBuf::from("C:\\runtime\\espeak-runtime"),
            }
        );
    }

    #[test]
    fn parse_request_rejects_semantically_invalid_payload_as_invalid_request_payload() {
        let error = parse_request(
            r#"{
                "text":"   ",
                "voice":"af_heart",
                "speed":1.0,
                "model_dir":"C:\\models\\kokoro-en"
            }"#,
        )
        .expect_err("invalid semantic payload should fail");

        assert!(error.starts_with("Invalid request payload:"));
        assert!(error.contains("text must not be empty or whitespace"));
    }
}
