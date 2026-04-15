# lingopilot-tts-kokoro

`lingopilot-tts-kokoro` is a local Kokoro TTS sidecar. It runs as a long-lived child process, reads one newline-delimited JSON request per line from `stdin`, writes one newline-delimited JSON response to `stdout`, and writes raw PCM16 audio bytes immediately after each successful `audio` response.

`README.md` is the canonical public contract for this repository. If the implementation and this document disagree, treat that as a defect.

## Status

Current repository state: `NOT READY`

What works now:

- startup validation with `--espeak-data-dir`
- exactly one `ready` JSON header on successful startup
- strict newline-delimited request parsing
- request validation and deterministic error classification
- deterministic Kokoro bundle validation inside `model_dir`
- deterministic Kokoro voice-family validation
- real Kokoro ONNX inference and PCM16 output for supported requests
- real English phonemization for `af_*` / `am_*` / `bf_*` / `bm_*`
- deterministic eSpeak-backed phonemization for `ef_*` / `em_*`, `ff_*`, `hf_*` / `hm_*`, `if_*` / `im_*`, and `pf_*` / `pm_*`
- stderr-only observability logs

What does not work yet:

- Japanese and Mandarin synthesis for `jf_*` / `jm_*` and `zf_*` / `zm_*` is intentionally deferred for `v0.1.0`
- release packaging has not been revalidated with real Kokoro assets in CI
- a first published GitHub Release has not been smoke-tested end to end

This repository is pre-release. The public protocol is stable enough to test, but the release gates in this README are not fully closed yet.

## Overview

This project is for applications that need a local, offline TTS worker with a strict pipe protocol:

- one long-lived child process
- JSON request headers on `stdin`
- JSON response headers on `stdout`
- raw PCM bytes immediately after `audio`
- clean shutdown on `stdin` close
- no request-time network dependency

Kokoro-specific constraints:

- Kokoro voice selection is bundle-based rather than file-per-voice
- Kokoro voice IDs encode language-family prefixes such as `af_*`, `bf_*`, `jf_*`, and `pf_*`
- Kokoro requires a text-to-phoneme path before ONNX inference
- several supported non-English paths currently depend on eSpeak-backed phonemization

## Runtime Model

This repository uses a native Rust sidecar plus a Rust-integrated Kokoro backend.

- the process boundary owns the protocol lifecycle
- synthesis stays local and self-contained at request time
- packaging stays within one executable plus runtime assets
- the runtime does not depend on Python services or network calls during synthesis

## Protocol Contract

### Startup

Start the sidecar with:

```text
lingopilot-tts-kokoro --espeak-data-dir <absolute-path>
```

Rules:

- `--espeak-data-dir` is required.
- The path must be absolute.
- The directory must exist and contain `espeak-ng-data/`.
- Supported phonemization paths additionally expect the platform eSpeak shared library in the same directory.
- ONNX Runtime must be available either through `ORT_DYLIB_PATH=<absolute-path>` or as the platform ONNX Runtime shared library beside the sidecar executable.
- Unknown, duplicate, or incomplete startup arguments fail startup before `ready`.

On successful startup, the sidecar emits exactly one newline-delimited `ready` JSON object:

```json
{"type":"ready","version":"0.1.0"}
```

### Request Framing

- The host sends exactly one JSON object per line on `stdin`.
- Each request must be terminated by `\n`.
- Empty lines are ignored.
- Requests are decoded with strict field checking. Unknown fields are rejected.
- Closing `stdin` terminates the process cleanly.

### Request Schema

| Field | Type | Required | Contract |
|-------|------|----------|----------|
| `text` | string | yes | Text to synthesize. Must contain at least one non-whitespace character and be at most `8192` Unicode scalar values. |
| `voice` | string | yes | Kokoro voice ID such as `af_heart` or `bf_emma`. The sidecar currently validates the voice family prefix only. |
| `speed` | number | no | Speed multiplier. Defaults to `1.0`. Must be finite and between `0.5` and `2.0` inclusive. |
| `model_dir` | string | yes | Absolute path to an existing directory that contains exactly one Kokoro `.onnx` model file and exactly one `voices*.bin` bundle. |

Example request:

```json
{"text":"Hello, how are you?","voice":"af_heart","speed":1.0,"model_dir":"C:\\models\\kokoro-en"}
```

Additional rules:

- `language` is not part of the request contract. It is rejected as an unknown field.
- `espeak_data_dir` is not part of the request contract. eSpeak is selected only at process startup.
- Bundle resolution is strict. If `model_dir` does not contain exactly one `.onnx` file and exactly one `voices*.bin` file, the sidecar returns an `error` response.
- The current backend does not require extra vocab or config side files inside `model_dir`. If future backend changes require them, `README.md` must be updated at the same time.

### Response Framing

The sidecar writes exactly one newline-delimited JSON object per response on `stdout`.

| Type | Fields | Contract |
|------|--------|----------|
| `ready` | `version` | Emitted exactly once after successful startup. No binary data follows. |
| `audio` | `byte_length`, `sample_rate`, `channels` | Emitted for successful synthesis. Exactly `byte_length` PCM16 LE mono bytes follow immediately after the newline-delimited JSON header. `sample_rate` is currently `24000` and `channels` is currently `1`. |
| `error` | `message` | Error response. JSON only; no audio bytes follow. The process stays alive for later requests unless `stdin` is closed. |

### Current Runtime Behavior

Supported requests now run a real Kokoro ONNX backend and return `audio` plus raw PCM bytes.

Successful synthesis currently covers:

- American English: `af_*`, `am_*`
- British English: `bf_*`, `bm_*`
- Spanish: `ef_*`, `em_*`
- French: `ff_*`
- Hindi: `hf_*`, `hm_*`
- Italian: `if_*`, `im_*`
- Brazilian Portuguese: `pf_*`, `pm_*`

Still unsupported at synthesis time:

- Japanese: `jf_*`, `jm_*`
- Mandarin Chinese: `zf_*`, `zm_*`

For `v0.1.0`, this is an explicit product decision rather than an implementation accident. Those families fail explicitly with an `error` response instead of silently falling back to another path. Hosts must route `ja` and `zh_CN` to `lingopilot-tts-piper` or another non-Kokoro engine; this sidecar does not perform an internal fallback.

## Runtime Characteristics

These characteristics are part of the public contract:

- License posture: this repository is Apache-2.0. It must not add `espeak-rs-sys`, `espeak-rs`, or `piper-rs` to `Cargo.lock`. eSpeak-NG is loaded at runtime through `libloading`.
- Log level selection: `KOKORO_TTS_LOG` is the primary log-level environment variable. `LINGOPILOT_TTS_LOG` remains a temporary compatibility alias, and `RUST_LOG` remains the final fallback.
- Voice resolution: the request `voice` selects an entry inside one shared `voices*.bin` bundle; the model directory must contain exactly one `*.onnx` and exactly one `voices*.bin`.
- Speed range: supported request values are `0.5` to `2.0`.
- Sample rate: successful Kokoro responses currently report `24000`.
- eSpeak runtime selection: startup selects the runtime directory once per process through `--espeak-data-dir`.
- eSpeak linkage: the sidecar loads `espeak-ng.dll` at runtime from `--espeak-data-dir`.

## Kokoro Voice Families

The sidecar currently infers `lang_code` from the official Kokoro voice prefixes:

| Prefix family | Language |
|---------------|----------|
| `af_`, `am_` | American English |
| `bf_`, `bm_` | British English |
| `jf_`, `jm_` | Japanese |
| `zf_`, `zm_` | Mandarin Chinese |
| `ef_`, `em_` | Spanish |
| `ff_` | French |
| `hf_`, `hm_` | Hindi |
| `if_`, `im_` | Italian |
| `pf_`, `pm_` | Brazilian Portuguese |

## Differences From `lingopilot-tts-piper`

This sidecar keeps the same general sidecar family shape where possible, but several Kokoro-specific differences are intentional:

| Area | `lingopilot-tts-piper` | `lingopilot-tts-kokoro` |
|------|------------------------|--------------------------|
| Binary license | GPL-3.0-only | Apache-2.0; eSpeak is runtime-loaded through `libloading` |
| `voice` resolution | per-voice `.onnx` plus sidecar metadata files | one shared `.onnx` plus one shared `voices*.bin` bundle |
| `speed` range | `0.5` to `5.5` | `0.5` to `2.0` |
| Sample rate | voice-dependent, typically `22050` | fixed `24000` |
| `model_dir` layout | flat per-voice model layout | exactly one `*.onnx` and one `voices*.bin` |
| eSpeak runtime env var | `PIPER_ESPEAKNG_DATA_DIRECTORY` | none; runtime path is selected at startup with `--espeak-data-dir` |
| eSpeak linkage | build-time linked eSpeak bindings | runtime-loaded `espeak-ng.dll` |

## Windows Packaging And Host Integration

The canonical Windows release is one extracted package root with this layout:

```text
<package-root>/
  lingopilot-tts-kokoro.exe
  onnxruntime.dll
  espeak-runtime/
    espeak-ng-data/
    espeak-ng.dll
    <other required eSpeak runtime files>
  kokoro-model/
    <exactly one *.onnx>
    <exactly one voices*.bin>
  README.md
  LICENSE
  THIRD_PARTY_LICENSES.txt
```

Packaging rules:

- `onnxruntime.dll` must be beside `lingopilot-tts-kokoro.exe` in the packaged release.
- `espeak-runtime/` must remain a directory and must contain both `espeak-ng-data/` and `espeak-ng.dll`.
- `kokoro-model/` is the canonical packaged `model_dir` name.
- The packaged `kokoro-model/` directory contains only the required runtime bundle files for the current backend: one `.onnx` file and one `voices*.bin` file.
- The request contract does not change for packaged releases. The host must still pass an absolute `model_dir`.
- The sidecar does not implicitly search for `kokoro-model/` relative to the executable.
- `ORT_DYLIB_PATH` remains a supported override for development and live tests, but the packaged release path is the sibling `onnxruntime.dll`.

A host application should compute `model_dir` from the extracted package root and pass that absolute path in each request. Example:

```powershell
$packageRoot = "C:\absolute\path\to\lingopilot-tts-kokoro-v0.1.0-windows-x86_64"
$runtimeDir = Join-Path $packageRoot "espeak-runtime"
$modelDir = Join-Path $packageRoot "kokoro-model"

& (Join-Path $packageRoot "lingopilot-tts-kokoro.exe") --espeak-data-dir $runtimeDir
```

## Validation

Run:

```bash
cargo check --locked
cargo test --locked
```

`cargo test --locked` is the default deterministic suite. It does not require real Kokoro assets and remains suitable for CI.

The default automated suite covers:

- startup `ready` emission
- startup failure without `ready`
- malformed JSON handling
- unknown-field rejection
- semantic payload validation
- Kokoro bundle validation
- unsupported voice-prefix rejection
- embedded Kokoro vocab tokenization
- phoneme chunking and PCM conversion
- eSpeak runtime-path resolution
- explicit unsupported-language error paths
- mock-backed runtime-cache reuse
- stdout/stderr separation on error paths
- graceful shutdown after `stdin` closes
- error responses that never leak PCM bytes onto `stdout`

## Live Assets And Packaging Helpers

Opt-in ignored live tests in `tests/live_assets.rs` cover every Sprint 6 scenario that needs a real Kokoro bundle or actual PCM output:

- invalid voice against a real `model_dir`
- American English synthesis with `af_*`
- British English synthesis with `bf_*`
- Spanish synthesis with `ef_*`
- French synthesis with `ff_*`
- Hindi synthesis with `hf_*`
- Italian synthesis with `if_*`
- Brazilian Portuguese synthesis with `pf_*`
- repeated same-process synthesis with exact PCM byte counts and stderr/stdout separation
- runtime cache-hit logging on a repeated same-voice request
- `speed` changing `byte_length`
- `model_dir` succeeding under a Windows path with spaces and non-ASCII characters

Provide these absolute paths through environment variables:

```powershell
$env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR = "C:\absolute\path\to\espeak-runtime"
$env:KOKORO_TTS_LIVE_MODEL_DIR = "C:\absolute\path\to\kokoro-model-dir"
$env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL = "C:\absolute\path\to\onnxruntime.dll"
```

Temporary compatibility aliases remain accepted during the neutrality transition:
`LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR`, `LINGOPILOT_TTS_LIVE_MODEL_DIR`, and
`LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL`.

Recommended Windows command:

```powershell
.\scripts\Test-LiveAssets.ps1
```

Equivalent direct command:

```bash
cargo test --locked -- --ignored --test-threads=1
```

The helper script validates the live paths before invoking the ignored suite serially with `--test-threads=1`, because the Windows eSpeak-backed live checks are not thread-safe inside one test process. Real Kokoro assets remain external and are not committed to this repository.

## GitHub CI Asset Configuration

The repository-owned Windows CI and release workflows expect real asset URLs to
be provided from GitHub configuration rather than committed into the repo.

Required GitHub configuration:

- secret `KOKORO_TTS_RELEASE_KOKORO_MODEL_URL`
- secret `KOKORO_TTS_RELEASE_ONNXRUNTIME_URL`
- optional repository variable `KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL`

Optional GitHub configuration:

- repository variable `KOKORO_TTS_ENABLE_LIVE_ASSETS_CI=true` enables the
  `windows-live-assets` job in `.github/workflows/ci.yml`

The helper script below is the canonical configuration check used by both
workflows:

```powershell
.\scripts\Assert-ReleaseAssetConfiguration.ps1
```

If the optional Piper zip variable is unset, the script derives the expected
Windows Piper release archive URL from the current package version and uses that
as the eSpeak runtime source.

To stage packaging inputs into the repository-default release layout:

```powershell
$env:KOKORO_TTS_RELEASE_KOKORO_MODEL_URL = "https://example.invalid/kokoro-model.zip"
$env:KOKORO_TTS_RELEASE_ONNXRUNTIME_URL = "https://example.invalid/onnxruntime-win-x64.zip"
.\scripts\Stage-WindowsReleaseAssets.ps1
```

The repository-owned release configuration now uses these neutral names as the primary interface:

- `KOKORO_TTS_RELEASE_KOKORO_MODEL_URL`
- `KOKORO_TTS_RELEASE_ONNXRUNTIME_URL`
- `KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL` as an optional override only

If `KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL` is unset, the staging script derives the matching Piper Windows release archive URL from the current package version and uses that as the default eSpeak runtime source.

Temporary compatibility aliases remain accepted inside the scripts during the neutrality transition:
`LINGOPILOT_TTS_RELEASE_KOKORO_MODEL_URL`,
`LINGOPILOT_TTS_RELEASE_ONNXRUNTIME_URL`, and
`LINGOPILOT_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL`.

Canonical Windows packaging consumes the staged defaults directly:

```powershell
.\scripts\Package-WindowsRelease.ps1
```

That script copies the staged `.onnx` and `voices*.bin` into `kokoro-model/` inside the archive and places the staged `onnxruntime.dll` beside the executable. Explicit `-ModelDir` and `-OnnxRuntimeDll` overrides remain available for manual local packaging, but the repository workflows do not use them.

After staging, `scripts\Verify-Readiness.ps1` automatically promotes the staged defaults below into `KOKORO_TTS_LIVE_*` for ignored live tests when explicit live-test env vars are unset:

- `target\release\espeak-runtime`
- `packaging\windows\kokoro-model`
- `packaging\windows\onnxruntime.dll`

To smoke-test the produced archive with a real synthesis request:

```powershell
.\scripts\Test-WindowsReleaseArchive.ps1 `
  -ZipPath .\dist\lingopilot-tts-kokoro-v0.1.0-windows-x86_64.zip
```

To verify the first published GitHub Release from the documented download URL:

```powershell
.\scripts\Test-PublishedRelease.ps1 -Version v0.1.0
```

That helper downloads the published zip and SHA-256 manifest, verifies the
checksum, and then delegates to `scripts\Test-WindowsReleaseArchive.ps1`.

For a full local verification pass:

```powershell
.\scripts\Verify-Readiness.ps1
```

To verify a packaged archive after `dist\` has been produced:

```powershell
.\scripts\Verify-Readiness.ps1 -Packaged
```

The Windows CI live-assets gate follows the same sequence with real assets: build, stage, run `cargo test --locked -- --ignored --test-threads=1`, package, and then call `Verify-Readiness.ps1 -Packaged`.

The release workflow adds one final published-artifact check after upload:
`scripts\Test-PublishedRelease.ps1` must be able to download the new release
archive from its documented GitHub Releases URL and pass the packaged smoke
test.

## Release Readiness

Current release state: `NOT READY`

It becomes `READY` only after all are true:

- `README.md` matches actual behavior
- a valid request returns an `audio` header plus exactly `byte_length` PCM bytes
- a second valid request in the same process also succeeds
- Windows validation passes with real Kokoro assets
- release packaging is defined and tested with real assets rather than only staged structure
- the first published GitHub Release has been smoke-tested end to end

## Upstream References

- Official Kokoro repository: <https://github.com/hexgrad/kokoro>
- Official Kokoro model card: <https://huggingface.co/hexgrad/Kokoro-82M>
- Official Kokoro voice catalog: <https://huggingface.co/hexgrad/Kokoro-82M/blob/main/VOICES.md>
- ONNX export reference implementation: <https://github.com/thewh1teagle/kokoro-onnx>

## License

This repository is currently licensed under the **Apache License 2.0**. Revalidate the final license position before publishing release artifacts.
