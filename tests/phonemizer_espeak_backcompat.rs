//! Golden-string backcompat tests for the eSpeak phonemizer pipeline.
//!
//! Each test calls `phonemize_for_test` with a canonical sentence for a specific
//! voice family and asserts that the phoneme output matches a committed golden
//! string. This catches silent regressions in eSpeak upgrades or phoneme
//! normalisation changes.
//!
//! All tests are `#[ignore]` because they require a real eSpeak runtime on disk.

#[path = "../src/live_test_support.rs"]
mod live_test_support;

use live_test_support::LiveTestAssets;
use lingopilot_tts_kokoro::synthesis::phonemize_for_test;

// ---------------------------------------------------------------------------
// Golden strings — captured from a stable CI runner.
// IMPORTANT: update these only intentionally after confirming the new output is
// correct. Each value is the `PhonemeResult.phonemes` field.
// ---------------------------------------------------------------------------

// Baselines captured from release run 24779170648 (v0.1.3 tag, windows-latest,
// eSpeak runtime staged by Stage-WindowsReleaseAssets.ps1). Regenerate only
// after a deliberate eSpeak/phoneme-pipeline change by re-running the test
// once and pasting the string from the panic message.
const GOLDEN_AF_HEART: &str = "ðə kwˈɪk bɹˈaʊn fˈɑks dʒˈʌmps ˌoʊvəɹ ðə lˈAɪzi dˈɑɡ";
const GOLDEN_BF_EMMA: &str = "ðə kwˈɪk bɹˈaʊn fˈɒks dʒˈʌmps ˌəʊvə ðə lˈAɪzi dˈɒɡ";
const GOLDEN_EF_DORA: &str = "el βelˈoθ θˈoro marˈon sˈalta sˌoβɾe el pˈero pˌeɾeθˈoso";
const GOLDEN_FF_SIWIS: &str = "lə ʁənˈaʁ bʁˈœ\u{303} ʁapˈid sˈot paʁdəsˈy lə ʃjˈɛ\u{303} paʁɛsˈø";
const GOLDEN_HF_ALPHA: &str = "(en)nˈamast(hi) (en)ˈɑːdʒ kˈɑː dˈɪn bˈahʌt ɐkhˈa hˈaɪ(hi)";
const GOLDEN_IF_SARA: &str = "la vˈolpe marɾˈone velˈotʃe sˈalta sˈopra il kˈane pˈiɡro";
const GOLDEN_PF_DORA: &str = "a xˈapidæ xˌapˈozæ maxˈoŋ pˈulæ sˈobri ʊ kˌaʃˈoxʊ prˌeɡisˈozʊ";
const GOLDEN_PM_SANTA: &str = "a xˈapidæ xˌapˈozæ maxˈoŋ pˈulæ sˈobri ʊ kˌaʃˈoxʊ prˌeɡisˈozʊ";

// ---------------------------------------------------------------------------
// Canonical sentences (one per language).
// ---------------------------------------------------------------------------
const SENTENCE_EN: &str = "The quick brown fox jumps over the lazy dog.";
const SENTENCE_ES: &str = "El veloz zorro marrón salta sobre el perro perezoso.";
const SENTENCE_FR: &str = "Le renard brun rapide saute par-dessus le chien paresseux.";
const SENTENCE_HI: &str = "Namaste, aaj ka din bahut accha hai.";
const SENTENCE_IT: &str = "La volpe marrone veloce salta sopra il cane pigro.";
const SENTENCE_PT: &str = "A rápida raposa marrom pula sobre o cachorro preguiçoso.";

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------
fn assert_golden(voice_id: &str, sentence: &str, expected: &str) {
    let assets = LiveTestAssets::from_env();
    let result = phonemize_for_test(sentence, voice_id, &assets.espeak_runtime_dir)
        .unwrap_or_else(|e| panic!("phonemize_for_test failed for {voice_id}: {e}"));

    if expected == "<<CAPTURE_BASELINE>>" {
        panic!(
            "BASELINE NOT YET CAPTURED for {voice_id}.\n\
             Run this test once and update the golden const with:\n\
             {:?}",
            result.phonemes
        );
    }

    assert_eq!(
        result.phonemes, expected,
        "Phoneme regression for voice {voice_id}"
    );
}

// ---------------------------------------------------------------------------
// Tests — one per voice family
// ---------------------------------------------------------------------------

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

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn golden_ef_dora_spanish() {
    assert_golden("ef_dora", SENTENCE_ES, GOLDEN_EF_DORA);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn golden_ff_siwis_french() {
    assert_golden("ff_siwis", SENTENCE_FR, GOLDEN_FF_SIWIS);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn golden_hf_alpha_hindi() {
    assert_golden("hf_alpha", SENTENCE_HI, GOLDEN_HF_ALPHA);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn golden_if_sara_italian() {
    assert_golden("if_sara", SENTENCE_IT, GOLDEN_IF_SARA);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn golden_pf_dora_portuguese() {
    assert_golden("pf_dora", SENTENCE_PT, GOLDEN_PF_DORA);
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn golden_pm_santa_portuguese_male() {
    assert_golden("pm_santa", SENTENCE_PT, GOLDEN_PM_SANTA);
}
