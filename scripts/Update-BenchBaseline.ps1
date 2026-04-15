# LOCAL developer action, NOT CI.
#
# Rotating benches/baseline.json is a deliberate, human-reviewed change. Any PR
# that modifies the baseline must:
#   - include a checklist entry stating the hardware class used to capture,
#   - attach the criterion HTML reports under target/criterion as evidence,
#   - explain why rotation is intentional (release cut, hardware rebase, etc.),
#   - receive CODEOWNERS review.
#
# Criterion is invoked with --save-baseline v<ver> so historical snapshots
# accumulate under target/criterion/<group>/<id>/v<ver>/ for offline diffing.

[CmdletBinding()]
param(
    [string]$BaselineName = 'v0.1.0',
    [string]$Version = '0.1.0',
    [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$baselinePath = Join-Path $RepoRoot 'benches/baseline.json'
$criterionRoot = Join-Path $RepoRoot 'target/criterion'

Push-Location $RepoRoot
try {
    & cargo bench --bench synth -- --save-baseline $BaselineName
    if ($LASTEXITCODE -ne 0) {
        throw "cargo bench failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

function Convert-SecondsToMs {
    param([double]$Seconds)
    return [math]::Round($Seconds * 1000.0, 3)
}

$groupDir = Join-Path $criterionRoot 'synth'
if (-not (Test-Path $groupDir)) {
    throw "criterion output not found at $groupDir"
}

$measurements = [ordered]@{}
foreach ($voiceDir in Get-ChildItem -Path $groupDir -Directory) {
    $voiceEntry = [ordered]@{}
    foreach ($durationDir in Get-ChildItem -Path $voiceDir.FullName -Directory) {
        $estimatesPath = Join-Path $durationDir.FullName 'new/estimates.json'
        if (-not (Test-Path $estimatesPath)) { continue }
        $estimates = Get-Content $estimatesPath -Raw | ConvertFrom-Json
        $meanSec = [double]$estimates.mean.point_estimate / 1e9
        $stdSec  = [double]$estimates.std_dev.point_estimate / 1e9
        $p50 = Convert-SecondsToMs -Seconds $meanSec
        $p95 = Convert-SecondsToMs -Seconds ($meanSec + 1.645 * $stdSec)
        $p99 = Convert-SecondsToMs -Seconds ($meanSec + 2.326 * $stdSec)
        $voiceEntry[$durationDir.Name] = [ordered]@{
            p50_ms = $p50
            p95_ms = $p95
            p99_ms = $p99
        }
    }
    $measurements[$voiceDir.Name] = $voiceEntry
}

$baseline = [ordered]@{
    version          = $Version
    captured_at      = (Get-Date).ToUniversalTime().ToString('o')
    runner_hardware  = 'github-hosted windows-latest (Azure Standard_D4ads_v5, 4 vCPU AMD EPYC 7763, 16 GiB RAM)'
    measurements     = $measurements
}

$baseline | ConvertTo-Json -Depth 8 | Set-Content -Path $baselinePath -Encoding UTF8
Write-Host "Wrote baseline to $baselinePath"
