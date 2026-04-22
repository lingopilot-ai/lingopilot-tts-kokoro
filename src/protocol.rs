use serde::{Deserialize, Serialize};

const MAX_TEXT_CHARS: usize = 8192;
const MAX_ID_BYTES: usize = 128;
const MIN_SPEED: f32 = 0.5;
const MAX_SPEED: f32 = 2.0;

/// Request sent by the host process via stdin (one JSON object per line).
///
/// Tagged by the `op` discriminator per host contract (see main repo
/// `docs/sidecar-directives/kokoro.md`). Reserved ops (`audio_chunk`,
/// `cancel`) are NOT represented here — the dispatcher rejects them with
/// `bad_request` before deserialization reaches this enum.
#[derive(Debug, Deserialize)]
#[serde(tag = "op")]
pub enum TtsRequest {
    #[serde(rename = "synthesize")]
    Synthesize {
        id: String,
        text: String,
        voice_id: String,
        #[serde(default = "default_speed")]
        speed: f32,
    },
}

fn default_speed() -> f32 {
    1.0
}

impl TtsRequest {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            TtsRequest::Synthesize {
                id,
                text,
                voice_id,
                speed,
            } => {
                if id.is_empty() {
                    return Err("Invalid id: id must not be empty".to_string());
                }
                if id.len() > MAX_ID_BYTES {
                    return Err(format!(
                        "Invalid id: id must be at most {MAX_ID_BYTES} bytes"
                    ));
                }
                if text.trim().is_empty() {
                    return Err("Invalid text: text must not be empty or whitespace".to_string());
                }
                if text.chars().count() > MAX_TEXT_CHARS {
                    return Err(format!(
                        "Invalid text: text must be at most {MAX_TEXT_CHARS} characters"
                    ));
                }
                if voice_id.trim().is_empty() {
                    return Err(
                        "Invalid voice_id: voice_id must not be empty or whitespace".to_string()
                    );
                }
                if !speed.is_finite() || !(MIN_SPEED..=MAX_SPEED).contains(speed) {
                    return Err(format!(
                        "Invalid speed: speed must be a finite number between {MIN_SPEED} and {MAX_SPEED}"
                    ));
                }
                Ok(())
            }
        }
    }
}

/// Error kinds per host contract.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    BadRequest,
    UnknownVoice,
    SynthesisFailed,
    #[allow(dead_code)]
    Internal,
}

/// Response sent back to the host process via stdout.
#[derive(Debug, Serialize)]
#[serde(tag = "op")]
pub enum TtsResponse {
    #[serde(rename = "ready")]
    Ready {
        version: String,
        sample_rate: u32,
        channels: u16,
        encoding: &'static str,
    },

    #[serde(rename = "audio")]
    Audio {
        id: String,
        /// Number of PCM16 LE bytes that follow on stdout after the newline.
        bytes: u32,
        sample_rate: u32,
        channels: u16,
    },

    #[serde(rename = "done")]
    Done { id: String },

    #[serde(rename = "error")]
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        kind: ErrorKind,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{ErrorKind, TtsRequest, TtsResponse};

    fn parse(id: &str, text: &str, voice_id: &str, speed: f32) -> TtsRequest {
        serde_json::from_str(&format!(
            r#"{{"op":"synthesize","id":{id:?},"text":{text:?},"voice_id":{voice_id:?},"speed":{speed}}}"#
        ))
        .expect("request should deserialize")
    }

    #[test]
    fn request_deserializes_synthesize_op() {
        let r: TtsRequest = serde_json::from_str(
            r#"{"op":"synthesize","id":"1","text":"Hi","voice_id":"af_heart","speed":1.0}"#,
        )
        .unwrap();
        match r {
            TtsRequest::Synthesize {
                id,
                text,
                voice_id,
                speed,
            } => {
                assert_eq!(id, "1");
                assert_eq!(text, "Hi");
                assert_eq!(voice_id, "af_heart");
                assert_eq!(speed, 1.0);
            }
        }
    }

    #[test]
    fn request_defaults_speed() {
        let r: TtsRequest = serde_json::from_str(
            r#"{"op":"synthesize","id":"1","text":"Hi","voice_id":"af_heart"}"#,
        )
        .unwrap();
        match r {
            TtsRequest::Synthesize { speed, .. } => assert_eq!(speed, 1.0),
        }
    }

    #[test]
    fn request_rejects_empty_id() {
        let e = parse("", "Hi", "af_heart", 1.0).validate().unwrap_err();
        assert!(e.contains("id must not be empty"));
    }

    #[test]
    fn request_rejects_oversize_id() {
        let long = "x".repeat(129);
        let e = parse(&long, "Hi", "af_heart", 1.0).validate().unwrap_err();
        assert!(e.contains("at most 128 bytes"));
    }

    #[test]
    fn request_rejects_empty_text() {
        let e = parse("1", "   ", "af_heart", 1.0).validate().unwrap_err();
        assert!(e.contains("text must not be empty"));
    }

    #[test]
    fn request_rejects_empty_voice_id() {
        let e = parse("1", "Hi", "   ", 1.0).validate().unwrap_err();
        assert!(e.contains("voice_id must not be empty"));
    }

    #[test]
    fn request_rejects_speed_below_range() {
        let e = parse("1", "Hi", "af_heart", 0.49).validate().unwrap_err();
        assert!(e.contains("between 0.5 and 2"));
    }

    #[test]
    fn request_rejects_speed_above_range() {
        let e = parse("1", "Hi", "af_heart", 2.01).validate().unwrap_err();
        assert!(e.contains("between 0.5 and 2"));
    }

    #[test]
    fn request_accepts_speed_at_bounds() {
        parse("1", "Hi", "af_heart", 0.5).validate().unwrap();
        parse("1", "Hi", "af_heart", 2.0).validate().unwrap();
    }

    #[test]
    fn ready_serializes_with_op_tag() {
        let json = serde_json::to_string(&TtsResponse::Ready {
            version: "0.1.3".to_string(),
            sample_rate: 24000,
            channels: 1,
            encoding: "pcm16le",
        })
        .unwrap();
        assert!(json.contains(r#""op":"ready""#));
        assert!(json.contains(r#""sample_rate":24000"#));
        assert!(json.contains(r#""channels":1"#));
        assert!(json.contains(r#""encoding":"pcm16le""#));
        assert!(json.contains(r#""version":"0.1.3""#));
    }

    #[test]
    fn audio_serializes_with_id_and_bytes() {
        let json = serde_json::to_string(&TtsResponse::Audio {
            id: "req-1".to_string(),
            bytes: 48000,
            sample_rate: 24000,
            channels: 1,
        })
        .unwrap();
        assert!(json.contains(r#""op":"audio""#));
        assert!(json.contains(r#""id":"req-1""#));
        assert!(json.contains(r#""bytes":48000"#));
    }

    #[test]
    fn done_serializes_with_id() {
        let json = serde_json::to_string(&TtsResponse::Done {
            id: "req-1".to_string(),
        })
        .unwrap();
        assert_eq!(json, r#"{"op":"done","id":"req-1"}"#);
    }

    #[test]
    fn error_with_id_serializes() {
        let json = serde_json::to_string(&TtsResponse::Error {
            id: Some("req-1".to_string()),
            kind: ErrorKind::UnknownVoice,
            message: "no such voice".to_string(),
        })
        .unwrap();
        assert!(json.contains(r#""op":"error""#));
        assert!(json.contains(r#""id":"req-1""#));
        assert!(json.contains(r#""kind":"unknown_voice""#));
        assert!(json.contains(r#""message":"no such voice""#));
    }

    #[test]
    fn error_without_id_omits_field() {
        let json = serde_json::to_string(&TtsResponse::Error {
            id: None,
            kind: ErrorKind::BadRequest,
            message: "bad".to_string(),
        })
        .unwrap();
        assert!(!json.contains(r#""id""#));
        assert!(json.contains(r#""kind":"bad_request""#));
    }
}
