//! Manual test: synthesizes one phrase per enabled language and plays each
//! through the default audio output (speakers).
//!
//! Run with:
//!   cargo test --locked -- --ignored --nocapture play_all_languages
//!
//! Requires the live eSpeak runtime + ONNX Runtime + Kokoro model assets.

#[path = "../src/live_test_support.rs"]
mod live_test_support;
#[path = "support/sidecar.rs"]
mod sidecar;

use live_test_support::LiveTestAssets;
use sidecar::{request_for, SidecarHarness};

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

/// Phrases to synthesize — one per language family.
const PHRASES: &[(&str, &str, &str)] = &[
    ("af_heart", "en-us", "The quick brown fox jumps over the lazy dog."),
    ("bf_emma",  "en-gb", "The quick brown fox jumps over the lazy dog."),
    ("ef_dora",  "es",    "El veloz zorro marrón salta sobre el perro perezoso."),
    ("ff_siwis", "fr",    "Le renard brun rapide saute par-dessus le chien paresseux."),
    ("hf_alpha", "hi",    "Namaste, aaj ka din bahut accha hai."),
    ("if_sara",  "it",    "La volpe marrone veloce salta sopra il cane pigro."),
    ("pf_dora",  "pt-br", "A rápida raposa marrom pula sobre o cachorro preguiçoso."),
    ("pm_santa", "pt-br", "Olá, eu sou o Papai Noel e desejo a todos um Feliz Natal."),
];

const SAMPLE_RATE: u32 = 24000;
const CHANNELS: u16 = 1;
const BITS_PER_SAMPLE: u16 = 16;

fn write_wav(path: &Path, pcm_bytes: &[u8]) {
    let data_len = pcm_bytes.len() as u32;
    let file_len = 36 + data_len;
    let byte_rate = SAMPLE_RATE * (CHANNELS as u32) * (BITS_PER_SAMPLE as u32 / 8);
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);

    let mut f = fs::File::create(path).expect("should create WAV file");
    // RIFF header
    f.write_all(b"RIFF").unwrap();
    f.write_all(&file_len.to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    // fmt chunk
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap(); // chunk size
    f.write_all(&1u16.to_le_bytes()).unwrap();  // PCM format
    f.write_all(&CHANNELS.to_le_bytes()).unwrap();
    f.write_all(&SAMPLE_RATE.to_le_bytes()).unwrap();
    f.write_all(&byte_rate.to_le_bytes()).unwrap();
    f.write_all(&block_align.to_le_bytes()).unwrap();
    f.write_all(&BITS_PER_SAMPLE.to_le_bytes()).unwrap();
    // data chunk
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    f.write_all(pcm_bytes).unwrap();
}

fn play_wav(path: &Path) {
    // Windows: use PowerShell to play synchronously
    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(New-Object Media.SoundPlayer '{}').PlaySync()",
                path.display()
            ),
        ])
        .status()
        .expect("powershell should be available");
    assert!(status.success(), "audio playback failed");
}

fn spawn_ready_sidecar(live_assets: &LiveTestAssets) -> SidecarHarness {
    let mut sidecar = SidecarHarness::spawn_with_dirs_and_env(
        &live_assets.espeak_runtime_dir,
        Some(&live_assets.model_dir),
        None,
        &[(
            "ORT_DYLIB_PATH",
            live_assets
                .onnxruntime_dll
                .to_str()
                .expect("dll path should be utf-8"),
        )],
    );
    let ready = sidecar.read_json_line();
    assert_eq!(ready["op"], "ready");
    sidecar
}

#[test]
#[ignore = "Requires a real packaged eSpeak runtime, ONNX Runtime DLL, and Kokoro assets"]
fn play_all_languages() {
    let live_assets = LiveTestAssets::from_env();
    let mut sidecar = spawn_ready_sidecar(&live_assets);

    let out_dir = std::env::temp_dir().join("lingopilot-tts-kokoro-play-all");
    let _ = fs::remove_dir_all(&out_dir);
    fs::create_dir_all(&out_dir).expect("output dir should be created");

    for (voice, lang, text) in PHRASES {
        eprintln!("\n▶ [{lang}] voice={voice}: \"{text}\"");

        sidecar.send_json(request_for(text, voice, &live_assets.model_dir));

        let audio = sidecar.read_json_line();
        assert_eq!(audio["op"], "audio", "expected audio response for {voice}, got: {audio}");

        let byte_length = audio["bytes"]
            .as_u64()
            .expect("bytes should be present") as usize;

        let pcm_bytes = sidecar.read_exact_stdout_bytes(byte_length);
        let done = sidecar.read_json_line();
        assert_eq!(done["op"], "done");

        let wav_path = out_dir.join(format!("{voice}_{lang}.wav"));
        write_wav(&wav_path, &pcm_bytes);

        eprintln!("  ✓ {} bytes → {}", byte_length, wav_path.display());
        play_wav(&wav_path);
    }

    eprintln!("\n✅ All {} languages played. WAV files in: {}", PHRASES.len(), out_dir.display());
}
