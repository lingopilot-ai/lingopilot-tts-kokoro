//! Golden-string backcompat tests for the eSpeak phonemizer pipeline.
//!
//! Pragmatic coverage: only `af_heart` (American English) and `bf_emma`
//! (British English) are pinned. An eSpeak upgrade that regresses phoneme
//! output will show up on either of these; pinning every supported language
//! added maintenance churn without catching additional classes of regression.
//!
//! Both tests are `#[ignore]` because they require a real eSpeak runtime on disk.

#[path = "../src/live_test_support.rs"]
mod live_test_support;

use live_test_support::LiveTestAssets;
use lingopilot_tts_kokoro::synthesis::phonemize_for_test;

// Baselines captured from release run 24779170648 (v0.1.3 tag, windows-latest,
// eSpeak runtime staged by Stage-WindowsReleaseAssets.ps1). Regenerate only
// after a deliberate eSpeak/phoneme-pipeline change by re-running the test
// once and pasting the string from the panic message.
const GOLDEN_AF_HEART: &str = "ðə kwˈɪk bɹˈaʊn fˈɑks dʒˈʌmps ˌoʊvəɹ ðə lˈAɪzi dˈɑɡ";
const GOLDEN_BF_EMMA: &str = "ðə kwˈɪk bɹˈaʊn fˈɒks dʒˈʌmps ˌəʊvə ðə lˈAɪzi dˈɒɡ";

const SENTENCE_EN: &str = "The quick brown fox jumps over the lazy dog.";

fn assert_golden(voice_id: &str, sentence: &str, expected: &str) {
    let assets = LiveTestAssets::from_env();
    let result = phonemize_for_test(sentence, voice_id, &assets.espeak_runtime_dir)
        .unwrap_or_else(|e| panic!("phonemize_for_test failed for {voice_id}: {e}"));

    assert_eq!(
        result.phonemes, expected,
        "Phoneme regression for voice {voice_id}"
    );
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn golden_af_heart_american_english() {
    assert_golden("af_heart", SENTENCE_EN, GOLDEN_AF_HEART);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn golden_bf_emma_british_english() {
    assert_golden("bf_emma", SENTENCE_EN, GOLDEN_BF_EMMA);
}
