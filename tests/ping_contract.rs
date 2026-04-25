// Integration tests for E-14 Ping/Pong health-check (ADR §4.2).
//
// Tests 1 and 2 do not require kokoro model assets and run in CI.
// Test 3 (`deferred_ping_proves_ordering_invariant`) is asset-dependent and
// is marked `#[ignore]`. It runs automatically when Verify-Readiness.ps1
// finds live assets (KOKORO_TTS_LIVE_* env vars). To run it manually:
//
//   KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR=<path> \
//   KOKORO_TTS_LIVE_MODEL_DIR=<path> \
//   KOKORO_TTS_LIVE_ONNXRUNTIME_DLL=<path> \
//   cargo test --test ping_contract deferred_ping_proves_ordering_invariant -- --ignored

#[path = "../src/live_test_support.rs"]
mod live_test_support;
#[path = "support/sidecar.rs"]
mod sidecar;

use std::fs;
use std::time::Duration;

use live_test_support::LiveTestAssets;
use serde_json::json;
use sidecar::{SidecarHarness, TempDir};

// ---------------------------------------------------------------------------
// Helper: create a minimal espeak runtime dir (espeak-ng-data sub-directory)
// that satisfies the sidecar's startup check.
// ---------------------------------------------------------------------------
fn create_espeak_runtime(dir: &std::path::Path) {
    fs::create_dir(dir.join("espeak-ng-data")).expect("espeak-ng-data dir should be created");
}

// ---------------------------------------------------------------------------
// Helper: spawn a harness with a fake espeak runtime and an empty model dir.
// Ping tests bypass synthesis, so missing kokoro assets are acceptable.
//
// Returns the harness plus the two TempDirs that back the espeak and model
// paths. Callers must bind the TempDirs to named variables so they are not
// dropped before the harness exits.
// ---------------------------------------------------------------------------
fn spawn_ready_sidecar() -> (SidecarHarness, TempDir, TempDir) {
    let runtime_dir = TempDir::new("ping-espeak");
    create_espeak_runtime(runtime_dir.path());
    let model_dir = TempDir::new("ping-model"); // empty — ping bypasses synthesis

    let mut sidecar = SidecarHarness::spawn_with_dirs_and_env(
        runtime_dir.path(),
        Some(model_dir.path()),
        None,
        &[],
    );

    let ready = sidecar.read_json_line();
    assert_eq!(
        ready["op"], "ready",
        "expected ready directive; got: {ready}"
    );

    (sidecar, runtime_dir, model_dir)
}

// ---------------------------------------------------------------------------
// Test 1: idle ping after ready
//
// Verifies the simplest ping/pong round-trip: spawn, wait for ready, send a
// ping, assert the pong echoes the id exactly.
// ---------------------------------------------------------------------------
#[test]
fn idle_ping_after_ready() {
    let (mut sidecar, _runtime_dir, _model_dir) = spawn_ready_sidecar();

    let guard = sidecar.arm_deadline(Duration::from_secs(5));

    sidecar.send_json(json!({"op": "ping", "id": "h1"}));
    let pong = sidecar.read_json_line();

    guard.cancel();

    assert_eq!(
        pong,
        json!({"op": "pong", "id": "h1"}),
        "pong must exactly match {{'op':'pong','id':'h1'}}"
    );

    sidecar.close_stdin();
}

// ---------------------------------------------------------------------------
// Test 2: ping between syntheses (two-ping sequential variant)
//
// Verifies the dispatch loop handles multiple sequential pings correctly,
// echoing the correct id for each. No model assets are required.
// ---------------------------------------------------------------------------
#[test]
fn ping_between_syntheses() {
    let (mut sidecar, _runtime_dir, _model_dir) = spawn_ready_sidecar();

    let guard = sidecar.arm_deadline(Duration::from_secs(5));

    // First ping
    sidecar.send_json(json!({"op": "ping", "id": "first"}));
    let pong1 = sidecar.read_json_line();
    assert_eq!(pong1["op"], "pong", "first response must be pong");
    assert_eq!(pong1["id"], "first", "first pong must echo id 'first'");

    // Second ping
    sidecar.send_json(json!({"op": "ping", "id": "second"}));
    let pong2 = sidecar.read_json_line();
    assert_eq!(pong2["op"], "pong", "second response must be pong");
    assert_eq!(pong2["id"], "second", "second pong must echo id 'second'");

    guard.cancel();

    sidecar.close_stdin();
}

// ---------------------------------------------------------------------------
// Test 3: deferred ping proves ordering invariant (asset-dependent)
//
// Verifies ADR §6.1: pong MUST NOT appear between the `audio` JSON line and
// the `done` JSON line. A ping sent while synthesis is running must be
// deferred until after `done` is emitted.
//
// Marked `#[ignore]` — runs automatically via Verify-Readiness.ps1 when live
// assets are present (KOKORO_TTS_LIVE_* env vars). To run manually, see the
// file header comment.
// ---------------------------------------------------------------------------
#[test]
#[ignore]
fn deferred_ping_proves_ordering_invariant() {
    let live_assets = LiveTestAssets::from_env();

    let ort_dylib = live_assets
        .onnxruntime_dll
        .to_str()
        .expect("onnxruntime dll path should be utf-8");

    let mut sidecar = SidecarHarness::spawn_with_dirs_and_env(
        &live_assets.espeak_runtime_dir,
        Some(&live_assets.model_dir),
        None,
        &[("ORT_DYLIB_PATH", ort_dylib)],
    );

    let guard = sidecar.arm_deadline(Duration::from_secs(60));

    let ready = sidecar.read_json_line();
    assert_eq!(ready["op"], "ready", "expected ready; got: {ready}");

    // Step 2: send a synthesize request.
    sidecar.send_json(json!({
        "op": "synthesize",
        "id": "synth-1",
        "text": "Hi",
        "voice_id": "af_heart",
        "speed": 1.0,
    }));

    // Step 3: read until we see the `audio` JSON line.
    let audio = sidecar.read_json_line();
    assert_eq!(audio["op"], "audio", "expected audio line; got: {audio}");
    assert_eq!(audio["id"], "synth-1");

    let byte_length = audio["bytes"]
        .as_u64()
        .expect("audio.bytes must be present") as usize;
    assert!(byte_length > 0, "audio must have non-zero byte length");

    // Step 4: immediately send a ping while the PCM payload is still in flight.
    sidecar.send_json(json!({"op": "ping", "id": "hDeferred"}));

    // Step 5: drain the PCM byte payload from stdout (raw binary).
    let _pcm = sidecar.read_exact_stdout_bytes(byte_length);

    // Step 6: read `done` — must arrive before pong.
    let done = sidecar.read_json_line();
    assert_eq!(done["op"], "done", "expected done before pong; got: {done}");
    assert_eq!(done["id"], "synth-1", "done must echo synth-1 id");

    // Step 7: read pong — must arrive after done, not before.
    let pong = sidecar.read_json_line();
    assert_eq!(
        pong,
        json!({"op": "pong", "id": "hDeferred"}),
        "pong must appear after done, not before"
    );

    guard.cancel();

    // Step 8: ordering is already verified by the read sequence above.
    // audio JSON → PCM bytes → done JSON → pong JSON (no interleaving).
    sidecar.close_stdin();
}
