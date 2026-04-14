use std::collections::HashMap;
use std::sync::OnceLock;

pub const MAX_PHONEME_LENGTH: usize = 510;
pub const SAMPLE_RATE: u32 = 24_000;

static KOKORO_VOCAB: OnceLock<HashMap<char, i64>> = OnceLock::new();

pub fn vocab() -> &'static HashMap<char, i64> {
    KOKORO_VOCAB.get_or_init(|| {
        let raw: HashMap<String, i64> =
            serde_json::from_str(include_str!("kokoro_vocab.json")).expect("valid Kokoro vocab");

        raw.into_iter()
            .map(|(key, value)| {
                let mut chars = key.chars();
                let ch = chars
                    .next()
                    .unwrap_or_else(|| panic!("Kokoro vocab key '{key}' must not be empty"));
                assert!(
                    chars.next().is_none(),
                    "Kokoro vocab key '{key}' must be a single character"
                );
                (ch, value)
            })
            .collect()
    })
}
