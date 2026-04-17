param(
    [switch]$Packaged,
    [switch]$SkipLiveTests
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$readmePath = Join-Path $repoRoot "README.md"
$stagedRuntimeDir = Join-Path $repoRoot "target\release\espeak-runtime"
$stagedModelDir = Join-Path $repoRoot "packaging\windows\kokoro-model"
$stagedOnnxRuntimeDll = Join-Path $repoRoot "packaging\windows\onnxruntime.dll"

function Resolve-LiveEnvValue {
    param(
        [Parameter(Mandatory = $true)]
        [string]$PrimaryName,
        [Parameter(Mandatory = $true)]
        [string]$LegacyName
    )

    $primary = (Get-Item -LiteralPath "Env:$PrimaryName" -ErrorAction SilentlyContinue).Value
    if (-not [string]::IsNullOrWhiteSpace($primary)) {
        return $primary
    }

    $legacy = (Get-Item -LiteralPath "Env:$LegacyName" -ErrorAction SilentlyContinue).Value
    if (-not [string]::IsNullOrWhiteSpace($legacy)) {
        return $legacy
    }

    return $null
}

function Resolve-RequiredPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$Value,
        [Parameter(Mandatory = $true)]
        [ValidateSet("Directory", "File")]
        [string]$Kind,
        [string]$RequiredChild
    )

    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw "$Name must point to an existing $Kind path."
    }

    if (-not [System.IO.Path]::IsPathRooted($Value)) {
        throw "$Name must be an absolute path. Got '$Value'."
    }

    if (-not (Test-Path -LiteralPath $Value)) {
        throw "$Name is missing: $Value"
    }

    $resolved = (Resolve-Path -LiteralPath $Value).Path
    switch ($Kind) {
        "Directory" {
            if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
                throw "$Name must point to an existing directory. Got '$resolved'."
            }
        }
        "File" {
            if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
                throw "$Name must point to an existing file. Got '$resolved'."
            }
        }
    }

    if ($RequiredChild) {
        $childPath = Join-Path $resolved $RequiredChild
        if (-not (Test-Path -LiteralPath $childPath -PathType Container)) {
            throw "$Name must contain '$RequiredChild'. Missing '$childPath'."
        }
    }

    return $resolved
}

function Get-LiveAssetConfiguration {
    $runtimeDir = Resolve-LiveEnvValue `
        -PrimaryName "KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR" `
        -LegacyName "LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR"
    $modelDir = Resolve-LiveEnvValue `
        -PrimaryName "KOKORO_TTS_LIVE_MODEL_DIR" `
        -LegacyName "LINGOPILOT_TTS_LIVE_MODEL_DIR"
    $onnxRuntimeDll = Resolve-LiveEnvValue `
        -PrimaryName "KOKORO_TTS_LIVE_ONNXRUNTIME_DLL" `
        -LegacyName "LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL"

    $explicitValues = @($runtimeDir, $modelDir, $onnxRuntimeDll) |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    if ($explicitValues.Count -gt 0 -and $explicitValues.Count -lt 3) {
        throw "Live asset configuration is incomplete. Set all three KOKORO_TTS_LIVE_* paths or rely on staged defaults."
    }

    if ($explicitValues.Count -eq 3) {
        return @{
            Source = "environment"
            RuntimeDir = Resolve-RequiredPath `
                -Name "KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR" `
                -Value $runtimeDir `
                -Kind Directory `
                -RequiredChild "espeak-ng-data"
            ModelDir = Resolve-RequiredPath `
                -Name "KOKORO_TTS_LIVE_MODEL_DIR" `
                -Value $modelDir `
                -Kind Directory
            OnnxRuntimeDll = Resolve-RequiredPath `
                -Name "KOKORO_TTS_LIVE_ONNXRUNTIME_DLL" `
                -Value $onnxRuntimeDll `
                -Kind File
        }
    }

    if (
        (Test-Path -LiteralPath $stagedRuntimeDir -PathType Container) -and
        (Test-Path -LiteralPath (Join-Path $stagedRuntimeDir "espeak-ng-data") -PathType Container) -and
        (Test-Path -LiteralPath $stagedModelDir -PathType Container) -and
        (Test-Path -LiteralPath $stagedOnnxRuntimeDll -PathType Leaf)
    ) {
        return @{
            Source = "staged defaults"
            RuntimeDir = Resolve-RequiredPath `
                -Name "staged espeak runtime" `
                -Value $stagedRuntimeDir `
                -Kind Directory `
                -RequiredChild "espeak-ng-data"
            ModelDir = Resolve-RequiredPath `
                -Name "staged Kokoro model dir" `
                -Value $stagedModelDir `
                -Kind Directory
            OnnxRuntimeDll = Resolve-RequiredPath `
                -Name "staged onnxruntime.dll" `
                -Value $stagedOnnxRuntimeDll `
                -Kind File
        }
    }

    return $null
}

Push-Location $repoRoot
try {
    Write-Host "[Verify-Readiness] step=assert-forbidden-cargo-lock-crates" -ForegroundColor Cyan
    .\scripts\Assert-ForbiddenCargoLockCrates.ps1

    Write-Host "[Verify-Readiness] step=cargo-check" -ForegroundColor Cyan
    cargo check --locked

    Write-Host "[Verify-Readiness] step=cargo-test" -ForegroundColor Cyan
    cargo test --locked

    if (-not $SkipLiveTests) {
        Write-Host "[Verify-Readiness] step=live-assets-lookup" -ForegroundColor Cyan
        $liveAssets = Get-LiveAssetConfiguration
        if ($liveAssets) {
            Write-Host "[Verify-Readiness] step=cargo-test-ignored source=$($liveAssets.Source)" -ForegroundColor Cyan
            $env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR = $liveAssets.RuntimeDir
            $env:KOKORO_TTS_LIVE_MODEL_DIR = $liveAssets.ModelDir
            $env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL = $liveAssets.OnnxRuntimeDll
            cargo test --locked -- --ignored --test-threads=1
        }
    }

    if ($Packaged) {
        Write-Host "[Verify-Readiness] step=packaged-archive-smoke" -ForegroundColor Cyan
        $zip = Get-ChildItem -LiteralPath (Join-Path $repoRoot "dist") -Filter "*.zip" |
            Sort-Object -Property LastWriteTimeUtc -Descending |
            Select-Object -First 1
        if (-not $zip) {
            throw "No packaged archive was found under dist\\."
        }

        .\scripts\Test-WindowsReleaseArchive.ps1 -ZipPath $zip.FullName
    }

    Write-Host "[Verify-Readiness] step=readme-check" -ForegroundColor Cyan

    $readme = Get-Content -LiteralPath $readmePath -Raw
    if ($readme -match "LINGOPILOT_TTS_ONNXRUNTIME_DLL") {
        throw "README.md still references the deprecated compatibility alias LINGOPILOT_TTS_ONNXRUNTIME_DLL. Use ORT_DYLIB_PATH instead."
    }
    if ($readme -notmatch "ORT_DYLIB_PATH") {
        throw "README.md must document ORT_DYLIB_PATH."
    }
}
finally {
    Pop-Location
}
