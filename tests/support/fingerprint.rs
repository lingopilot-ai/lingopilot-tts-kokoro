use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

const WINDOW_MS: usize = 50;
const SAMPLE_RATE: usize = 24_000;
const WINDOW_SAMPLES: usize = SAMPLE_RATE * WINDOW_MS / 1_000;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Fingerprint {
    pub byte_length: usize,
    pub mean_rms: f64,
    pub mean_zcr: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Baseline {
    pub fingerprint: Fingerprint,
    pub byte_length_tol: f64,
    pub rms_tol: f64,
    pub zcr_tol: f64,
    pub notes: String,
}

pub fn compute(pcm16le_bytes: &[u8]) -> Fingerprint {
    assert!(pcm16le_bytes.len() % 2 == 0, "PCM bytes must be even");

    let samples: Vec<i16> = pcm16le_bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    let mut rms_acc = 0.0_f64;
    let mut zcr_acc = 0.0_f64;
    let mut windows = 0_usize;

    for chunk in samples.chunks(WINDOW_SAMPLES) {
        if chunk.len() < WINDOW_SAMPLES / 2 {
            break;
        }
        let mut sum_sq = 0.0_f64;
        let mut crossings = 0_usize;
        let mut prev_sign = chunk[0].signum();
        for &s in chunk {
            let f = s as f64 / 32_768.0;
            sum_sq += f * f;
            let sign = s.signum();
            if sign != 0 && sign != prev_sign && prev_sign != 0 {
                crossings += 1;
            }
            if sign != 0 {
                prev_sign = sign;
            }
        }
        rms_acc += (sum_sq / chunk.len() as f64).sqrt();
        zcr_acc += crossings as f64 / chunk.len() as f64;
        windows += 1;
    }

    assert!(windows > 0, "PCM too short for any analysis window");

    Fingerprint {
        byte_length: pcm16le_bytes.len(),
        mean_rms: rms_acc / windows as f64,
        mean_zcr: zcr_acc / windows as f64,
    }
}

pub fn load_baseline(path: &Path) -> Option<Baseline> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_baseline(path: &Path, baseline: &Baseline) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture dir should be creatable");
    }
    let json = serde_json::to_string_pretty(baseline).expect("baseline should serialize");
    fs::write(path, json).expect("baseline should write");
}

pub fn assert_within(actual: &Fingerprint, baseline: &Baseline) {
    let b = &baseline.fingerprint;
    let byte_low = (b.byte_length as f64 * (1.0 - baseline.byte_length_tol)) as usize;
    let byte_high = (b.byte_length as f64 * (1.0 + baseline.byte_length_tol)) as usize;
    assert!(
        (byte_low..=byte_high).contains(&actual.byte_length),
        "byte_length {} outside [{}, {}] (baseline {}, tol {})",
        actual.byte_length,
        byte_low,
        byte_high,
        b.byte_length,
        baseline.byte_length_tol
    );

    let rms_delta = (actual.mean_rms - b.mean_rms).abs() / b.mean_rms;
    assert!(
        rms_delta <= baseline.rms_tol,
        "mean_rms drift {:.4} > tol {:.4} (actual {:.6}, baseline {:.6})",
        rms_delta,
        baseline.rms_tol,
        actual.mean_rms,
        b.mean_rms
    );

    let zcr_delta = (actual.mean_zcr - b.mean_zcr).abs() / b.mean_zcr;
    assert!(
        zcr_delta <= baseline.zcr_tol,
        "mean_zcr drift {:.4} > tol {:.4} (actual {:.6}, baseline {:.6})",
        zcr_delta,
        baseline.zcr_tol,
        actual.mean_zcr,
        b.mean_zcr
    );
}
