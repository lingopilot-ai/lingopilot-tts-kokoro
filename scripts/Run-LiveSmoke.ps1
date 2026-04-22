param(
    [switch]$UpdateFingerprint,
    [switch]$Play
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

$stagedRoot = Get-ChildItem -Path (Join-Path $repoRoot "dist") -Directory -Filter "lingopilot-tts-kokoro-*windows*" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1

if (-not $stagedRoot) {
    throw "No staged Windows release found under dist/. Run scripts\Stage-WindowsReleaseAssets.ps1 first."
}

$espeakRuntime = Join-Path $stagedRoot.FullName "espeak-runtime"
$modelDir = Join-Path $stagedRoot.FullName "kokoro-model"
$onnxDll = Join-Path $stagedRoot.FullName "onnxruntime.dll"

foreach ($p in @($espeakRuntime, $modelDir, $onnxDll)) {
    if (-not (Test-Path -LiteralPath $p)) {
        throw "Missing live asset: $p. Re-run scripts\Stage-WindowsReleaseAssets.ps1."
    }
}

Write-Host "Using staged assets from: $($stagedRoot.FullName)"

$env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR = $espeakRuntime
$env:KOKORO_TTS_LIVE_MODEL_DIR = $modelDir
$env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL = $onnxDll

if ($UpdateFingerprint) {
    $env:KOKORO_TTS_UPDATE_FINGERPRINT = "1"
} else {
    Remove-Item Env:KOKORO_TTS_UPDATE_FINGERPRINT -ErrorAction SilentlyContinue
}

if ($Play) {
    $env:KOKORO_TTS_PLAY_FINGERPRINT = "1"
} else {
    Remove-Item Env:KOKORO_TTS_PLAY_FINGERPRINT -ErrorAction SilentlyContinue
}

Push-Location $repoRoot
try {
    cargo test --test live_assets live_english_af_heart_fingerprint -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw "live smoke failed (exit $LASTEXITCODE)" }
}
finally {
    Pop-Location
}
