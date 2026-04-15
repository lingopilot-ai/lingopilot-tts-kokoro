# Assert-StagedLayout.ps1
#
# Given a POSIX staged release directory (output of Package-PosixRelease.ps1
# -StageOnly), verify the presence of every file the final tarball would ship:
#   - the binary,
#   - libonnxruntime.{so,dylib},
#   - espeak-runtime/espeak-ng-data/,
#   - espeak-runtime/libespeak-ng.{so,dylib},
#   - kokoro-model/*.onnx and kokoro-model/voices*.bin,
#   - README.md, LICENSE, THIRD_PARTY_LICENSES.txt.
#
# Fails non-zero with a clear diff (expected vs actual) on any missing entry.

param(
    [Parameter(Mandatory = $true)][string]$StagingDir,
    [Parameter(Mandatory = $true)][ValidateSet("linux", "macos")][string]$Platform
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $StagingDir -PathType Container)) {
    throw "StagingDir does not exist or is not a directory: $StagingDir"
}

$runtimeLib = if ($Platform -eq "macos") { "libonnxruntime.dylib" } else { "libonnxruntime.so" }
$espeakLib  = if ($Platform -eq "macos") { "libespeak-ng.dylib"  } else { "libespeak-ng.so"  }

$requiredFiles = @(
    "lingopilot-tts-kokoro",
    $runtimeLib,
    "README.md",
    "LICENSE",
    "THIRD_PARTY_LICENSES.txt",
    "espeak-runtime/$espeakLib"
)
$requiredDirs = @(
    "espeak-runtime",
    "espeak-runtime/espeak-ng-data",
    "kokoro-model"
)
$requiredGlobs = @(
    @{ Dir = "kokoro-model"; Pattern = "*.onnx";     Description = "Kokoro model (*.onnx)" },
    @{ Dir = "kokoro-model"; Pattern = "voices*.bin"; Description = "Kokoro voices bundle (voices*.bin)" }
)

$missing = New-Object System.Collections.Generic.List[string]

foreach ($rel in $requiredDirs) {
    $p = Join-Path $StagingDir $rel
    if (-not (Test-Path -LiteralPath $p -PathType Container)) { $missing.Add("missing directory: $rel") }
}
foreach ($rel in $requiredFiles) {
    $p = Join-Path $StagingDir $rel
    if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { $missing.Add("missing file: $rel") }
}
foreach ($g in $requiredGlobs) {
    $dir = Join-Path $StagingDir $g.Dir
    if (-not (Test-Path -LiteralPath $dir -PathType Container)) { continue }
    $hits = @(Get-ChildItem -LiteralPath $dir -File | Where-Object { $_.Name -like $g.Pattern })
    if ($hits.Count -lt 1) { $missing.Add("missing glob: $($g.Dir)/$($g.Pattern)  ($($g.Description))") }
}

if ($missing.Count -gt 0) {
    Write-Host "Staged layout verification FAILED for: $StagingDir" -ForegroundColor Red
    Write-Host "Expected layout (platform=$Platform):" -ForegroundColor Red
    foreach ($rel in $requiredDirs)  { Write-Host "  [dir]  $rel" }
    foreach ($rel in $requiredFiles) { Write-Host "  [file] $rel" }
    foreach ($g in $requiredGlobs)   { Write-Host "  [glob] $($g.Dir)/$($g.Pattern)" }
    Write-Host "Findings:" -ForegroundColor Red
    foreach ($m in $missing) { Write-Host "  - $m" -ForegroundColor Red }
    Write-Host "Actual tree:" -ForegroundColor Yellow
    Get-ChildItem -LiteralPath $StagingDir -Recurse -Force | ForEach-Object {
        $rel = $_.FullName.Substring($StagingDir.Length).TrimStart('\','/')
        Write-Host "  $rel"
    }
    throw "Assert-StagedLayout: $($missing.Count) missing entries."
}

Write-Host "Staged layout OK: $StagingDir (platform=$Platform)" -ForegroundColor Green
