# Compares the current `cargo bench` run against benches/baseline.json.
# Exits non-zero if any cell's p95 regresses by more than +10% over baseline.
# Intended to run in CI after `cargo bench --bench synth`.

[CmdletBinding()]
param(
    [double]$ThresholdPercent = 10.0,
    [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$baselinePath = Join-Path $RepoRoot 'benches/baseline.json'
if (-not (Test-Path $baselinePath)) {
    throw "baseline not found at $baselinePath"
}

$baseline = Get-Content $baselinePath -Raw | ConvertFrom-Json
$pendingFirstCapture = $false
if ($baseline.PSObject.Properties.Name -contains 'pending_first_capture') {
    $pendingFirstCapture = [bool]$baseline.pending_first_capture
}

$groupDir = Join-Path $RepoRoot 'target/criterion/synth'
if (-not (Test-Path $groupDir)) {
    throw "criterion output not found at $groupDir (did cargo bench run?)"
}

function Convert-SecondsToMs {
    param([double]$Seconds)
    return [math]::Round($Seconds * 1000.0, 3)
}

$rows = @()
$regressions = @()

foreach ($voiceProp in $baseline.measurements.PSObject.Properties) {
    $voice = $voiceProp.Name
    foreach ($durationProp in $voiceProp.Value.PSObject.Properties) {
        $duration = $durationProp.Name
        $baselineCell = $durationProp.Value
        $estimatesPath = Join-Path $groupDir "$voice/$duration/new/estimates.json"
        if (-not (Test-Path $estimatesPath)) {
            $rows += [pscustomobject]@{
                Voice = $voice; Duration = $duration;
                BaselineP95 = $baselineCell.p95_ms; CurrentP95 = 'MISSING'; Delta = 'n/a'; Status = 'MISSING'
            }
            $regressions += "$voice/$duration: missing criterion estimates"
            continue
        }
        $estimates = Get-Content $estimatesPath -Raw | ConvertFrom-Json
        $meanSec = [double]$estimates.mean.point_estimate / 1e9
        $stdSec  = [double]$estimates.std_dev.point_estimate / 1e9
        $currentP95Ms = Convert-SecondsToMs -Seconds ($meanSec + 1.645 * $stdSec)
        $baseP95 = [double]$baselineCell.p95_ms
        if ($pendingFirstCapture -or $baseP95 -le 0.0) {
            $status = 'BASELINE_PENDING'
            $deltaText = 'n/a'
        } else {
            $deltaPct = (($currentP95Ms - $baseP95) / $baseP95) * 100.0
            $deltaText = ('{0:+0.00;-0.00;0.00}%' -f $deltaPct)
            if ($deltaPct -gt $ThresholdPercent) {
                $status = 'REGRESSION'
                $regressions += "$voice/$duration: Δp95=$deltaText (baseline=$baseP95 ms, current=$currentP95Ms ms)"
            } else {
                $status = 'OK'
            }
        }
        $rows += [pscustomobject]@{
            Voice = $voice; Duration = $duration;
            BaselineP95 = $baseP95; CurrentP95 = $currentP95Ms; Delta = $deltaText; Status = $status
        }
    }
}

$summary = @()
$summary += "## Bench Gate"
$summary += ""
$summary += "Threshold: Δp95 > +$ThresholdPercent% fails."
if ($pendingFirstCapture) {
    $summary += ""
    $summary += "_Baseline is flagged ``pending_first_capture``; regressions are not enforced for this run._"
}
$summary += ""
$summary += "| Voice | Duration | Baseline p95 (ms) | Current p95 (ms) | Δ | Status |"
$summary += "| --- | --- | ---: | ---: | ---: | --- |"
foreach ($row in $rows) {
    $summary += "| $($row.Voice) | $($row.Duration) | $($row.BaselineP95) | $($row.CurrentP95) | $($row.Delta) | $($row.Status) |"
}

$summaryText = $summary -join "`n"
if ($env:GITHUB_STEP_SUMMARY) {
    Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value $summaryText -Encoding UTF8
}
Write-Host $summaryText

if ($pendingFirstCapture) {
    Write-Host "Baseline pending first capture; skipping regression enforcement."
    exit 0
}

if ($regressions.Count -gt 0) {
    Write-Error ("Bench regressions detected:`n" + ($regressions -join "`n"))
    exit 1
}

Write-Host "Bench gate OK."
