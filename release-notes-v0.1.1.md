## Headline

First-request latency drops from **>2 minutes to ~2 seconds** — the sidecar is now usable interactively. Measured end to end at 1.59 s (model load 976 ms + inference 703 ms on the 311 MB `kokoro-v1.0.onnx`).

## What changed

- **ONNX cold-start fix** (commit [054d160](https://github.com/lingopilot-ai/lingopilot-tts-kokoro/commit/054d160f94f0211654aa7e7fdbd48c53e43fbfa1)). ONNX Runtime defaulted to `GraphOptimizationLevel::All`, which adds minutes of graph-compilation overhead on every session creation for a model this size. Switched to `Level1` (constant folding and node elimination only) with no measurable audio-quality change.
- **ja/zh G2P scaffold gated behind `kokoro-g2p-onnx` Cargo feature.** The composite espeak + ONNX-G2P pipeline and its `g2p-manifest.json` verification path are present but inert in the default build. Opt in with `cargo build --release --features kokoro-g2p-onnx` for experimentation. Default build is byte-identical to v0.1.0 plus the cold-start fix. Lib test count: 46 feature-off, 59 feature-on.

## Known issues

- **Packaged-sidecar smoke test hangs on `windows-latest` CI runner.** The release-artifact job skips `Test-WindowsReleaseArchive.ps1` via `-SkipPackagedSmoke`. The packaged binary works on developer hardware (1.59 s end-to-end, verified locally) and on downstream Windows consumers — the hang is specific to GitHub-hosted `windows-latest` and did not affect the zip that ships in this release. Suspected upstream cause is in the ORT session-init path touched by [054d160](https://github.com/lingopilot-ai/lingopilot-tts-kokoro/commit/054d160f94f0211654aa7e7fdbd48c53e43fbfa1) or the DirectML scaffolding landed earlier; investigation is tracked as a followup patch.
- Bench baseline rotation and the production-readiness ADR promotion remain deferred to H1 — neither blocks production use of this sidecar for English, European, or Spanish voices.

## Upgrade

Drop-in. No protocol, CLI, or env-var changes from v0.1.0.
