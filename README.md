# lingopilot-tts-kokoro

A local Kokoro TTS sidecar that reads one newline-delimited JSON request per line from `stdin`, writes JSON headers to `stdout`, and, on successful synthesis, writes raw PCM16 audio bytes immediately after the `audio` header.

`README.md` is the canonical public contract for this repository. If the implementation and this document disagree, treat that as a defect.

## Platform Support

| Platform | GitHub Actions validation | Official GitHub Release asset | Current status |
|----------|---------------------------|-------------------------------|----------------|
| Windows x86_64 | `cargo check --locked`, `cargo test --locked`, live-assets gate, packaged smoke test, published-release smoke test | Yes | Current downloadable artifact target |
| Linux x86_64 | Packaging scaffold only | No | Planned target only |
| Linux aarch64 | Packaging scaffold only | No | Planned target only |
| macOS x86_64 | Packaging scaffold only | No | Planned target only |
| macOS aarch64 | Packaging scaffold only | No | Planned target only |

Only Windows `x86_64` is a released artifact target today. Linux and macOS packaging helpers exist for future work, but they are not yet validated release targets.

## Current State

This repository is **Technical Ready** and **Operational Ready** for Windows `x86_64` at `v0.1.2`: the source tree is green, and the packaged + published release smoke tests cover end-to-end synthesis. **Release Ready is pending** — the latency baseline is not yet captured (E-09), the production-readiness ADR remains `Proposed` (E-10), and the `windows-latest` packaged-smoke hang investigation is open (E-08). See [docs/adr-production-readiness.md §4a](docs/adr-production-readiness.md) for the tier definitions and [docs/controlled-ambitions.md](docs/controlled-ambitions.md) for open risks. Japanese and Mandarin synthesis, and Linux/macOS release artifacts, are explicitly deferred (see below).

What works now:

- strict sidecar lifecycle and request framing
- exactly one `ready` header on successful startup
- deterministic request validation and error classification
- strict Kokoro bundle resolution inside `model_dir`
- real Kokoro ONNX inference for supported voice families
- real PCM16 output on successful synthesis
- real English phonemization for `af_*` / `am_*` / `bf_*` / `bm_*`
- deterministic eSpeak-backed phonemization for `ef_*` / `em_*`, `ff_*`, `hf_*` / `hm_*`, `if_*` / `im_*`, and `pf_*` / `pm_*`
- Windows `v0.1.2` GitHub Release published and smoke-tested end to end
- opt-in DirectML execution provider on Windows (CPU remains the default)

What does not work yet:

- Japanese synthesis for `jf_*` / `jm_*`
- Mandarin synthesis for `zf_*` / `zm_*`
- official Linux release artifacts
- official macOS release artifacts
- final license posture has not been revalidated beyond the current repository source-license statement

Treat the host-facing protocol as stable enough to integrate and test. Do not treat this repository as a fully closed, cross-platform product release yet.

### Support tiers

| Tier | Scope | Status |
|------|-------|--------|
| Supported | Windows `x86_64`, CPU execution provider | Released via GitHub Releases; smoke-tested end to end |
| Experimental (opt-in) | Windows `x86_64`, DirectML execution provider | Accepted via `--execution-provider directml`; not part of the release floor (see [docs/controlled-ambitions.md](docs/controlled-ambitions.md)) |
| Deferred | Japanese (`jf_*`/`jm_*`) and Mandarin (`zf_*`/`zm_*`) synthesis | Fails explicitly; tracked in [docs/controlled-ambitions.md](docs/controlled-ambitions.md) |
| Scaffolding only | Linux `x86_64` / `aarch64`, macOS `x86_64` / `aarch64` | CI builds and tests only; no published release asset |

Promotion criteria between tiers are recorded in [docs/controlled-ambitions.md](docs/controlled-ambitions.md).

## Quick Start

### 1. Download a Windows release

Current release contract:

- Tag format: `v<crate-version>`
- Windows asset name: `lingopilot-tts-kokoro-v<version>-windows-x86_64.zip`
- Checksum manifest: `lingopilot-tts-kokoro-v<version>-sha256.txt`
- Download base: `https://github.com/lingopilot-ai/lingopilot-tts-kokoro/releases/download/v<version>/`

Example URLs for `v0.1.2`:

```text
https://github.com/lingopilot-ai/lingopilot-tts-kokoro/releases/download/v0.1.2/lingopilot-tts-kokoro-v0.1.2-windows-x86_64.zip
https://github.com/lingopilot-ai/lingopilot-tts-kokoro/releases/download/v0.1.2/lingopilot-tts-kokoro-v0.1.2-sha256.txt
```

### 2. Extract the package

The Windows zip contains one top-level folder named after the asset:

```text
lingopilot-tts-kokoro-v0.1.2-windows-x86_64/
  lingopilot-tts-kokoro.exe
  onnxruntime.dll
  espeak-runtime/
  kokoro-model/
  README.md
  LICENSE
  THIRD_PARTY_LICENSES.txt
```

### 3. Start the sidecar

```powershell
$packageRoot = "C:\absolute\path\to\lingopilot-tts-kokoro-v0.1.2-windows-x86_64"
$runtimeDir = Join-Path $packageRoot "espeak-runtime"

& (Join-Path $packageRoot "lingopilot-tts-kokoro.exe") --espeak-data-dir $runtimeDir
```

On successful startup, the sidecar emits exactly one newline-delimited `ready` JSON object on `stdout`:

```json
{"type":"ready","version":"0.1.2"}
```

### 4. Send a request

```json
{"text":"Hello from Kokoro","voice":"af_heart","speed":1.0,"model_dir":"C:\\absolute\\path\\to\\lingopilot-tts-kokoro-v0.1.2-windows-x86_64\\kokoro-model"}
```

### 5. Read the response

Successful synthesis returns one `audio` header:

```json
{"type":"audio","byte_length":123456,"sample_rate":24000,"channels":1}
```

Immediately after the newline, exactly `byte_length` raw PCM16 LE mono bytes follow on `stdout`.

## What To Download

Kokoro does not use one file per voice like Piper. The released package already contains the full runtime bundle it needs:

- `lingopilot-tts-kokoro.exe`
- `onnxruntime.dll`
- `espeak-runtime/`
- `kokoro-model/kokoro-v1.0.onnx`
- `kokoro-model/voices-v1.0.bin`

`kokoro-v1.0.onnx` is the inference graph. `voices-v1.0.bin` is the shared voice bundle that contains the voice style tensors used by requests such as `af_heart`, `bf_emma`, `ff_siwis`, and `pf_dora`.

There is no request-time download of voice files in this repository.

## Protocol Contract

### Lifecycle

```text
Host                          Sidecar
 |                               |
 |--- spawn process ------------>|
 |                               |--- {"type":"ready"} ---> stdout
 |                               |
 |--- {"text":"..."}\n --------> stdin
 |                               |--- {"type":"audio"} ---> stdout
 |                               |--- [PCM16 bytes] ------> stdout
 |                               |
 |--- {"text":"..."}\n --------> stdin
 |                               |--- {"type":"error"} ---> stdout
 |                               |
 |--- close stdin -------------->|  (sidecar exits cleanly)
```

### Startup Contract

Start the sidecar with:

```text
lingopilot-tts-kokoro --espeak-data-dir <absolute-path> [--execution-provider cpu|directml]
```

Rules:

- `--espeak-data-dir` is required.
- The path must be absolute.
- The directory must exist and contain `espeak-ng-data/`.
- Supported phonemization paths additionally expect the platform eSpeak shared library in the same directory.
- ONNX Runtime must be available either through `ORT_DYLIB_PATH=<absolute-path>` or as the platform ONNX Runtime shared library beside the sidecar executable.
- `--execution-provider` is optional. Accepted values are `cpu` (default) and `directml`. The flag may appear at most once.
- Unknown, duplicate, or incomplete startup arguments fail startup before `ready`.

### Execution Providers

The sidecar selects an ONNX Runtime execution provider exactly once at startup via `--execution-provider`:

- `cpu` (default): the release floor. Used when the flag is absent.
- `directml`: Windows-only opt-in. On any non-Windows target, the flag is rejected before `ready` with `Startup error: --execution-provider directml is supported only on Windows`.

DirectML uses the DirectML EP already compiled into the bundled `onnxruntime.dll`; the release zip gains no additional files.

CUDA is not supported and will not be added. See `AGENTS.md §10.2`.

DirectML output is not guaranteed to match CPU output bit-exactly. Floating-point rounding differs between CPU and GPU kernels. Do not hash PCM to compare outputs across execution providers.

If DirectML registration fails (missing DirectML runtime, driver too old, no compatible adapter), the error surfaces on the first synthesis request through the normal `error` response envelope. The sidecar never silently falls back to CPU.

If startup validation fails:

- no protocol JSON is written to `stdout`
- an operator-facing `Startup error: ...` line is written to `stderr`
- the process exits non-zero

### Request Framing

- The host sends exactly one JSON object per line on `stdin`.
- Each request must be terminated by `\n`.
- Empty lines are ignored.
- Requests are decoded with strict field checking. Unknown fields are rejected.
- Closing `stdin` terminates the process cleanly.

### Request Schema — `synthesize`

| Field | Type | Required | Contract |
|-------|------|----------|----------|
| `op` | string | yes | Must be `"synthesize"`. |
| `id` | string | yes | Client-chosen correlation id. 1 to 128 bytes. |
| `text` | string | yes | Text to synthesize. Must contain at least one non-whitespace character and be at most `8192` Unicode scalar values. |
| `voice_id` | string | yes | Kokoro voice ID such as `af_heart` or `bf_emma`. |
| `speed` | number | no | Speed multiplier. Defaults to `1.0`. Must be finite and between `0.5` and `2.0` inclusive. |

Additional request rules:

- `language` is not part of the request contract. It is rejected as an unknown field.
- `espeak_data_dir` is not part of the request contract. eSpeak is selected only at process startup.
- Bundle resolution is strict. If `model_dir` does not contain exactly one `.onnx` file and exactly one `voices*.bin` file, the sidecar returns an `error` response.
- The sidecar does not implicitly search for `kokoro-model/` relative to the executable. The host must still pass an absolute `model_dir`.

### Request Schema — `ping`

| Field | Type | Required | Contract |
|-------|------|----------|----------|
| `op` | string | yes | Must be `"ping"`. |
| `id` | string | yes | Client-chosen correlation id. 1 to 128 bytes. Echoed byte-for-byte in the `pong` response. |

`ping` is a base-protocol health-check op available from `v0.1.4`. It is not listed in the `ops` array of the `ready` response and must never appear in a negotiated capability list. No extra fields are accepted. An empty or oversize `id` returns an `error` response with `kind:"bad_request"`; the process stays alive.

### Health Check (`op:ping` / `op:pong`)

Available since `v0.1.4`. The host sends a `ping` request; the sidecar replies with a `pong` response that echoes the `id` byte-for-byte. The exchange proves the sidecar process is alive and reading `stdin` without touching the synthesis path, the ONNX session, the voice cache, or eSpeak.

**Wire shape (locked):**

Request:

```json
{"op":"ping","id":"<correlation-id>"}
```

Response:

```json
{"op":"pong","id":"<correlation-id>"}
```

**`id` rules:** required, non-empty, at most 128 bytes, echoed verbatim.

**Floor version:** hosts must gate `HealthStrategy::Ping` on `ready.version >= "0.1.4"` for this sidecar.

**Ordering invariant:** `pong` is never emitted between an `audio` line and its corresponding `done` line. The single-threaded serial stdin loop structurally guarantees this. See `docs/adr-health-ping.md` §6.1 for the invariant and guidance for any future concurrent-dispatch migration.

**Discovery:** `ping` is a base-protocol op. It does not appear in the `ops` array of the `ready` response and answers "process alive", not "worker idle".

### Response Framing

The sidecar writes exactly one newline-delimited JSON object per response on `stdout`.

| `op` | Fields | Contract |
|------|--------|----------|
| `ready` | `version`, `sample_rate`, `channels`, `encoding` | Emitted exactly once after successful startup. No binary data follows. |
| `audio` | `id`, `bytes`, `sample_rate`, `channels` | Successful synthesis header. Immediately after the newline, exactly `bytes` raw PCM bytes follow on `stdout`. |
| `done` | `id` | Emitted after the PCM payload for a `synthesize` request. |
| `pong` | `id` | Health-check response. Echoes the `id` from the corresponding `ping` request. JSON only; no binary data follows. Available from `v0.1.4`. |
| `error` | `id`, `kind`, `message` | Error response. JSON only; no audio bytes follow. The process stays alive for later requests unless `stdin` is closed. |

Audio format:

- Encoding: PCM16 signed little-endian
- Channels: 1 (mono)
- Sample rate: `24000`
- Byte count: exactly `byte_length`

### Error Policy

Stable error prefixes:

- malformed JSON: `Invalid JSON request:`
- invalid payload or validation errors: `Invalid request payload:`
- synthesis/runtime failures: `Synthesis failed:`

The response shape, stream, and leading error category are the stable contract. The full tail text may vary by platform or by the underlying OS/library error.

### Observability

The sidecar writes structured log lines to `stderr` in the form `level=<L> event=<name> key=value ...`. Every successful synth request emits three `INFO`-level timing events:

| Event | Fields | Notes |
|-------|--------|-------|
| `phonemization_done` | `voice`, `duration_ms` | Time spent in eSpeak phonemization. |
| `model_loaded` | `duration_ms` | Emitted **only on cache miss** (first request for a given `model_dir`). |
| `inference_done` | `voice`, `chunk_count`, `duration_ms` | Total ONNX inference time across all phoneme chunks. |

Hosts can triage request latency by reading these `duration_ms` values without attaching a profiler. `stdout` is reserved for the JSON protocol and PCM bytes.

## Supported Voice Families

The sidecar currently infers `lang_code` from the official Kokoro voice prefixes:

| Prefix family | Language | Current state |
|---------------|----------|---------------|
| `af_`, `am_` | American English | Supported |
| `bf_`, `bm_` | British English | Supported |
| `ef_`, `em_` | Spanish | Supported |
| `ff_` | French | Supported |
| `hf_`, `hm_` | Hindi | Supported |
| `if_`, `im_` | Italian | Supported |
| `pf_`, `pm_` | Brazilian Portuguese | Supported |
| `jf_`, `jm_` | Japanese | Deferred (not implemented; see [docs/controlled-ambitions.md](docs/controlled-ambitions.md)) |
| `zf_`, `zm_` | Mandarin Chinese | Deferred (not implemented; see [docs/controlled-ambitions.md](docs/controlled-ambitions.md)) |

Unsupported Japanese and Mandarin families fail explicitly with an `error` response instead of silently falling back. Hosts must route those languages to `lingopilot-tts-piper` or another engine. See [docs/controlled-ambitions.md](docs/controlled-ambitions.md) for the deferral policy.

## Differences From `lingopilot-tts-piper`

This sidecar keeps the same general sidecar family shape where possible, but several Kokoro-specific differences are intentional:

| Area | `lingopilot-tts-piper` | `lingopilot-tts-kokoro` |
|------|------------------------|--------------------------|
| `voice` resolution | per-voice `.onnx` plus sidecar metadata files | one shared `.onnx` plus one shared `voices*.bin` bundle |
| `speed` range | `0.5` to `5.5` | `0.5` to `2.0` |
| Sample rate | voice-dependent, typically `22050` | fixed `24000` |
| `model_dir` layout | flat per-voice model layout | exactly one `*.onnx` and one `voices*.bin` |
| eSpeak runtime env var | `PIPER_ESPEAKNG_DATA_DIRECTORY` | none; runtime path is selected at startup with `--espeak-data-dir` |
| eSpeak linkage | build-time linked eSpeak bindings | runtime-loaded `espeak-ng.dll` |
| Binary license posture | GPL-3.0-only | repository source is Apache-2.0; eSpeak is runtime-loaded through `libloading` |

## Release Distribution

Release operator flow for Windows:

1. Ensure deterministic tests and live-assets validation pass.
2. Build the release binary.
3. Stage the real Kokoro model bundle, ONNX Runtime, and eSpeak runtime.
4. Package the Windows zip and checksum manifest.
5. Push a `v<crate-version>` tag.
6. Wait for `.github/workflows/release.yml` to publish the GitHub Release assets.
7. Download and verify the published zip plus checksum.
8. Mark the release validated only after the downloaded zip passes the packaged smoke test.

Local verification commands:

```powershell
.\scripts\Verify-Readiness.ps1
.\scripts\Verify-Readiness.ps1 -Packaged
.\scripts\Test-PublishedRelease.ps1 -Version v0.1.2
```

Published release verification:

```powershell
.\scripts\Test-PublishedRelease.ps1 -Version v0.1.2 -Repository lingopilot-ai/lingopilot-tts-kokoro
```

That helper downloads the published zip and SHA-256 manifest from GitHub Releases, verifies the checksum, and then runs the packaged smoke test.

## Validation

Deterministic suite:

```bash
cargo check --locked
cargo test --locked
```

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
- runtime-cache reuse
- stdout/stderr separation on error paths
- graceful shutdown after `stdin` closes
- error responses that never leak PCM bytes onto `stdout`

Live-assets suite:

```powershell
.\scripts\Test-LiveAssets.ps1
```

Equivalent direct command:

```bash
cargo test --locked -- --ignored --test-threads=1
```

Required live-assets environment variables:

```powershell
$env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR = "C:\absolute\path\to\espeak-runtime"
$env:KOKORO_TTS_LIVE_MODEL_DIR = "C:\absolute\path\to\kokoro-model-dir"
$env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL = "C:\absolute\path\to\onnxruntime.dll"
```

The helper validates the paths before invoking the ignored suite serially with `--test-threads=1`, because the Windows eSpeak-backed live checks are not thread-safe inside one test process.

## GitHub CI Asset Configuration

The Windows CI and release workflows read upstream asset URLs (and pinned SHA-256 checksums) from the committed `release-sources.toml` at the repo root. No repository secrets are required; forks can reproduce the release on a clean runner.

`release-sources.toml` exposes three tables — `[kokoro_model]`, `[onnxruntime]`, `[piper_windows]` — each with `url` and `sha256` fields. Leave `[piper_windows]` empty to derive the URL from the current tag.

Local overrides for staging against custom URLs:

- `KOKORO_TTS_RELEASE_KOKORO_MODEL_URL`
- `KOKORO_TTS_RELEASE_ONNXRUNTIME_URL`
- `KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL`

Env overrides bypass `release-sources.toml` entirely; checksum verification is skipped for overridden URLs.

Optional GitHub configuration:

- repository variable `KOKORO_TTS_ENABLE_LIVE_ASSETS_CI=true` enables the `windows-live-assets` job in `.github/workflows/ci.yml`

Canonical validation helper:

```powershell
.\scripts\Assert-ReleaseAssetConfiguration.ps1
```

Canonical staging helper:

```powershell
$env:KOKORO_TTS_RELEASE_KOKORO_MODEL_URL = "https://example.invalid/kokoro-model.zip"
$env:KOKORO_TTS_RELEASE_ONNXRUNTIME_URL = "https://example.invalid/onnxruntime-win-x64.zip"
.\scripts\Stage-WindowsReleaseAssets.ps1
```

Canonical Windows packaging command:

```powershell
.\scripts\Package-WindowsRelease.ps1
```

## Planned Cross-Platform Release Layout

When Linux and macOS packaging is added, this repository should keep one GitHub Release tag with one asset per supported platform/architecture pair plus one shared checksum manifest.

Planned asset naming:

```text
lingopilot-tts-kokoro-${LATEST_RELEASE}-windows-x86_64.zip
lingopilot-tts-kokoro-${LATEST_RELEASE}-linux-x86_64.tar.gz
lingopilot-tts-kokoro-${LATEST_RELEASE}-linux-aarch64.tar.gz
lingopilot-tts-kokoro-${LATEST_RELEASE}-macos-x86_64.tar.gz
lingopilot-tts-kokoro-${LATEST_RELEASE}-macos-aarch64.tar.gz
lingopilot-tts-kokoro-${LATEST_RELEASE}-sha256.txt
```

Platform-specific runtime library names:

| Platform | ONNX Runtime library | eSpeak library |
|----------|----------------------|----------------|
| Windows | `onnxruntime.dll` | `espeak-ng.dll` |
| Linux | `libonnxruntime.so` | `libespeak-ng.so` |
| macOS | `libonnxruntime.dylib` | `libespeak-ng.dylib` |

Canonical asset-plan helper:

```powershell
.\scripts\Get-ReleaseAssetPlan.ps1 -Version ${LATEST_RELEASE}
.\scripts\Get-ReleaseAssetPlan.ps1 -Version ${LATEST_RELEASE} -AsJson
```

Packaging scaffolds for future explicit-input local assembly:

```powershell
.\scripts\Package-LinuxRelease.ps1 `
  -Architecture x86_64 `
  -BinaryPath C:\abs\linux\lingopilot-tts-kokoro `
  -ModelDir C:\abs\kokoro-model `
  -OnnxRuntimeLibrary C:\abs\linux\libonnxruntime.so `
  -EspeakRuntimeDir C:\abs\linux\espeak-runtime `
  -Version ${LATEST_RELEASE}
```

```powershell
.\scripts\Package-MacOSRelease.ps1 `
  -Architecture aarch64 `
  -BinaryPath C:\abs\macos\lingopilot-tts-kokoro `
  -ModelDir C:\abs\kokoro-model `
  -OnnxRuntimeLibrary C:\abs\macos\libonnxruntime.dylib `
  -EspeakRuntimeDir C:\abs\macos\espeak-runtime `
  -Version ${LATEST_RELEASE}
```

Those scripts only assemble archives from explicit inputs. They do not mean Linux or macOS are currently supported release targets by this repository.

## Latency

Warm-inference p-quantiles for the current release, captured by the `windows-bench` CI job against the frozen baseline at [benches/baseline.json](benches/baseline.json). Regressions of `Δp95 > +10 %` vs. the frozen baseline fail the bench gate on PRs and the `bench-gate` job on release tags. First capture is pending — tracked as **E-09** in [BACKLOG.md](BACKLOG.md).

| Version | Voice | 1s p50 / p95 / p99 (ms) | 5s p50 / p95 / p99 (ms) | 20s p50 / p95 / p99 (ms) |
| --- | --- | --- | --- | --- |
| v0.1.2 (pending first capture) | af_heart | — / — / — | — / — / — | — / — / — |
| v0.1.2 (pending first capture) | pf_dora | — / — / — | — / — / — | — / — / — |
| v0.1.2 (pending first capture) | ef_alice | — / — / — | — / — / — | — / — / — |

Runner hardware class: github-hosted `windows-latest` (Azure Standard_D4ads_v5, 4 vCPU AMD EPYC 7763, 16 GiB RAM). Measurements are warm-inference p-quantiles over N=50 per cell; cold-start variance is intentionally excluded. Each row is populated from a first green `windows-bench` run on `main` via [scripts/Update-BenchBaseline.ps1](scripts/Update-BenchBaseline.ps1), which requires PR-checklist + CODEOWNERS review to rotate.

## Upstream References

- Official Kokoro repository: <https://github.com/hexgrad/kokoro>
- Official Kokoro model card: <https://huggingface.co/hexgrad/Kokoro-82M>
- Official Kokoro voice catalog: <https://huggingface.co/hexgrad/Kokoro-82M/blob/main/VOICES.md>
- ONNX export reference implementation: <https://github.com/thewh1teagle/kokoro-onnx>

## License

This repository is currently licensed under the **Apache License 2.0**. Revalidate the final distributed-binary license position separately from the repository source-license statement.
