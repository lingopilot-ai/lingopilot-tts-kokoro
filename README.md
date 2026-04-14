# lingopilot-tts-kokoro

A local TTS sidecar for Kokoro-based synthesis. It runs as a long-lived child process, reads newline-delimited JSON requests from `stdin`, writes JSON headers to `stdout`, and writes raw PCM16 audio bytes immediately after each successful `audio` response.

`README.md` is the canonical public contract for this repository. If the implementation and this document disagree, treat that as a defect.

## Status

Current repository state: `NOT READY`

What works now:

- startup validation with `--espeak-data-dir`
- one `ready` JSON header on successful startup
- strict newline-delimited request parsing
- request validation and error classification
- deterministic Kokoro bundle validation inside `model_dir`
- deterministic Kokoro voice-prefix validation
- real Kokoro ONNX inference and PCM16 output for supported requests
- real English phonemization for `af_*` / `am_*` / `bf_*` / `bm_*`
- deterministic eSpeak-first phonemization fallback for `ef_*` / `em_*`, `ff_*`, `hf_*` / `hm_*`, `if_*` / `im_*`, and `pf_*` / `pm_*`
- stderr-only observability logs

What does not work yet:

- Japanese and Mandarin phonemization (`jf_*` / `jm_*`, `zf_*` / `zm_*`)
- release packaging with validated Kokoro assets

This repository is pre-release. The public protocol is stable enough to test, but the release gates in this README are not fully closed yet.

## Overview

This repository is designed for applications that want a local, offline TTS worker with a strict pipe protocol:

- one long-lived child process
- JSON request headers on `stdin`
- JSON response headers on `stdout`
- raw PCM bytes after `audio`
- clean shutdown on `stdin` close

Kokoro-specific constraints:

- Kokoro voice selection is not file-per-voice; voices live inside a bundle such as `voices-v1.0.bin`
- Kokoro uses language-specific voice families such as `af_*`, `bf_*`, `jf_*`, `pf_*`
- Kokoro requires a text-to-phoneme path before ONNX inference
- The official Kokoro stack depends on `misaki`, and several language paths still depend on eSpeak fallback behavior

## Runtime Model

This repository uses a native Rust sidecar plus a Rust-integrated Kokoro backend.

- it keeps one process boundary for protocol ownership
- it keeps the request path offline and self-contained
- it packages cleanly as a single executable plus runtime assets
- it avoids request-time dependence on Python or network services

## Current Request Contract

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

Those unsupported families still fail explicitly with an `error` response instead of silently falling back to another path.

## Runtime Characteristics

These characteristics are part of the public contract:

- License posture: this repository is Apache-2.0. It must not add `espeak-rs-sys`, `espeak-rs`, or `piper-rs` to `Cargo.lock`. eSpeak-NG is loaded at runtime through `libloading`.
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

## Windows Packaging

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

Applications should compute `model_dir` from the extracted package root and pass that absolute path in each request. Example:

```powershell
$packageRoot = "C:\absolute\path\to\lingopilot-tts-kokoro-v0.1.0-windows-x86_64"
$runtimeDir = Join-Path $packageRoot "espeak-runtime"
$modelDir = Join-Path $packageRoot "kokoro-model"

& (Join-Path $packageRoot "lingopilot-tts-kokoro.exe") --espeak-data-dir $runtimeDir
```

## Validation

Run:

```bash
cargo check
cargo test
```

`cargo test` is the default deterministic suite. It does not require real Kokoro assets and remains suitable for CI.

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

Opt-in ignored live tests cover every scenario that needs a real Kokoro bundle or actual PCM output:

- invalid voice against a real `model_dir`
- one successful English synthesis request
- one successful non-English eSpeak-backed synthesis request
- repeated same-process synthesis with exact PCM byte counts
- stdout/stderr separation during successful synthesis
- direct backend synthesis for one English and one non-English supported family

Provide these absolute paths through environment variables:

```powershell
$env:LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR = "C:\absolute\path\to\espeak-runtime"
$env:LINGOPILOT_TTS_LIVE_MODEL_DIR = "C:\absolute\path\to\kokoro-model-dir"
$env:LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL = "C:\absolute\path\to\onnxruntime.dll"
```

Recommended Windows command:

```powershell
.\scripts\Test-LiveAssets.ps1
```

Equivalent direct command:

```bash
cargo test --locked -- --ignored
```

The helper script validates the live paths before invoking the ignored suite. Real Kokoro assets remain external and are not committed to this repository.

To stage packaging inputs into the repository-default release layout:

```powershell
$env:LINGOPILOT_TTS_RELEASE_KOKORO_MODEL_URL = "https://example.invalid/kokoro-model.zip"
$env:LINGOPILOT_TTS_RELEASE_ONNXRUNTIME_URL = "https://example.invalid/onnxruntime-win-x64.zip"
.\scripts\Stage-WindowsReleaseAssets.ps1
```

Canonical Windows packaging resolves staged defaults automatically, but still accepts explicit overrides:

```powershell
.\scripts\Package-WindowsRelease.ps1 `
  -ModelDir C:\absolute\path\to\kokoro-model-dir `
  -OnnxRuntimeDll C:\absolute\path\to\onnxruntime.dll
```

That script copies the resolved `.onnx` and `voices*.bin` into `kokoro-model/` inside the archive and places `onnxruntime.dll` beside the executable.

To smoke-test the produced archive with a real synthesis request:

```powershell
.\scripts\Test-WindowsReleaseArchive.ps1 `
  -ZipPath .\dist\lingopilot-tts-kokoro-v0.1.0-windows-x86_64.zip
```

For a full local verification pass:

```powershell
.\scripts\Verify-Readiness.ps1
```

## Release Readiness

Current release state: `NOT READY`

It becomes `READY` only after all are true:

- README matches actual behavior
- a valid request returns an `audio` header plus exactly `byte_length` PCM bytes
- a second valid request in the same process also succeeds
- Windows validation passes with real Kokoro assets
- release packaging is defined and tested

## Upstream References

- Official Kokoro repository: <https://github.com/hexgrad/kokoro>
- Official Kokoro model card: <https://huggingface.co/hexgrad/Kokoro-82M>
- Official Kokoro voice catalog: <https://huggingface.co/hexgrad/Kokoro-82M/blob/main/VOICES.md>
- ONNX export reference implementation: <https://github.com/thewh1teagle/kokoro-onnx>

## License

This repository is currently licensed under the **Apache License 2.0** to match the intended Kokoro model direction. Revalidate the final license position before the backend is completed and before publishing release artifacts.
