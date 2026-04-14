param(
    [switch]$Packaged
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$readmePath = Join-Path $repoRoot "README.md"

Push-Location $repoRoot
try {
    .\scripts\Assert-ForbiddenCargoLockCrates.ps1

    cargo check --locked
    cargo test --locked

    if (
        $env:LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR -and
        $env:LINGOPILOT_TTS_LIVE_MODEL_DIR -and
        $env:LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL
    ) {
        cargo test --locked -- --ignored
    }

    if ($Packaged) {
        $zip = Get-ChildItem -LiteralPath (Join-Path $repoRoot "dist") -Filter "*.zip" |
            Sort-Object -Property LastWriteTimeUtc -Descending |
            Select-Object -First 1
        if (-not $zip) {
            throw "No packaged archive was found under dist\\."
        }

        .\scripts\Test-WindowsReleaseArchive.ps1 -ZipPath $zip.FullName
    }

    $readme = Get-Content -LiteralPath $readmePath -Raw
    if ($readme -match "LINGOPILOT_TTS_ONNXRUNTIME_DLL") {
        throw "README.md still references the deprecated LINGOPILOT_TTS_ONNXRUNTIME_DLL env var."
    }
    if ($readme -notmatch "ORT_DYLIB_PATH") {
        throw "README.md must document ORT_DYLIB_PATH."
    }
}
finally {
    Pop-Location
}
