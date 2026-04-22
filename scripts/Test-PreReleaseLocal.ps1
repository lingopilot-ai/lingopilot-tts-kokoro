# Pre-release local validation.
#
# Mirrors the release-artifact job in .github/workflows/release.yml step-for-step
# so that "green locally" predicts "green on tag push". Run this BEFORE creating
# or pushing a tag. If any step fails here, the equivalent CI step will fail.
#
# Usage:
#   scripts\Test-PreReleaseLocal.ps1 -Version v0.1.3
#
# Optional:
#   -SkipPhonemizerGoldens   (skip ignored phonemizer backcompat tests)
#   -SkipLiveFingerprint     (skip ignored live audio fingerprint test)

param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [switch]$SkipPhonemizerGoldens,
    [switch]$SkipLiveFingerprint
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot

function Invoke-Step {
    param([string]$Name, [scriptblock]$Body)
    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    & $Body
    if ($LASTEXITCODE -ne $null -and $LASTEXITCODE -ne 0) {
        throw "Step failed: $Name (exit $LASTEXITCODE)"
    }
}

try {
    # 1. Tag/version alignment (mirrors: Validate tag against Cargo.toml)
    Invoke-Step "Assert tag matches Cargo.toml" {
        .\scripts\Assert-ReleaseTagMatchesVersion.ps1 -Tag $Version
    }

    # 2. Forbidden crates guard
    Invoke-Step "Assert-ForbiddenCargoLockCrates" {
        .\scripts\Assert-ForbiddenCargoLockCrates.ps1
    }

    # 3. Unit + integration tests (contract, staging, phonemizer unit)
    Invoke-Step "cargo test --locked" {
        cargo test --locked
    }

    # 4. Release asset configuration
    Invoke-Step "Assert-ReleaseAssetConfiguration" {
        .\scripts\Assert-ReleaseAssetConfiguration.ps1 -Version $Version
    }

    # 5. Release build
    Invoke-Step "cargo build --release --locked" {
        cargo build --release --locked
    }

    # 6. Stage assets
    Invoke-Step "Stage-WindowsReleaseAssets" {
        .\scripts\Stage-WindowsReleaseAssets.ps1 -Version $Version
    }

    # 7. Phonemizer goldens against staged eSpeak runtime
    if (-not $SkipPhonemizerGoldens) {
        $env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR = Join-Path $repoRoot "target\release\espeak-runtime"
        $env:KOKORO_TTS_LIVE_MODEL_DIR = Join-Path $repoRoot "packaging\windows\kokoro-model"
        $env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL = Join-Path $repoRoot "packaging\windows\onnxruntime.dll"
        Invoke-Step "Phonemizer golden baselines (af_heart, bf_emma)" {
            cargo test --locked --test phonemizer_espeak_backcompat -- --ignored --test-threads=1
        }
    }

    # 8. Package release archive
    Invoke-Step "Package-WindowsRelease" {
        .\scripts\Package-WindowsRelease.ps1 -Version $Version
    }

    # 9. Packaged readiness (same flags as release.yml)
    Invoke-Step "Verify-Readiness -Packaged" {
        .\scripts\Verify-Readiness.ps1 -Packaged -SkipLiveTests -SkipPackagedSmoke
    }

    $zipPath = "dist/lingopilot-tts-kokoro-$Version-windows-x86_64.zip"

    # 10. Directive smoke gate (zero-arg ready handshake)
    Invoke-Step "Test-ReleaseSmokeGate" {
        .\scripts\Test-ReleaseSmokeGate.ps1 -ZipPath $zipPath
    }

    # 11. End-to-end archive smoke test
    Invoke-Step "Test-WindowsReleaseArchive" {
        .\scripts\Test-WindowsReleaseArchive.ps1 -ZipPath $zipPath
    }

    # 12. Live audio fingerprint (the only test that proves real PCM is produced)
    if (-not $SkipLiveFingerprint) {
        Invoke-Step "Live af_heart audio fingerprint" {
            .\scripts\Run-LiveSmoke.ps1
        }
    }

    Write-Host "`nPRE-RELEASE LOCAL: ALL STEPS PASSED for $Version" -ForegroundColor Green
    Write-Host "Safe to: git tag $Version && git push origin $Version"
}
finally {
    Pop-Location
}
