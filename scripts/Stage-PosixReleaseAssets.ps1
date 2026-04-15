# Stage POSIX release assets (Linux/macOS) for packaging smoke tests.
#
# Mirrors Stage-WindowsReleaseAssets.ps1 but for POSIX targets:
#   - downloads the Kokoro model archive (platform-agnostic),
#   - downloads the platform-appropriate ONNX Runtime .tgz,
#   - materializes an espeak runtime directory from the system package manager
#     (libespeak-ng1 + espeak-ng-data on Ubuntu; espeak-ng via Homebrew on macOS).
#
# On success, emits the set of staged paths on stdout as KEY=VALUE lines so
# callers (CI) can consume them with a parser; also writes matching entries to
# $env:GITHUB_ENV when available.

param(
    [ValidateSet("linux", "macos")]
    [string]$Platform,
    [ValidateSet("x86_64", "aarch64")]
    [string]$Architecture,
    [string]$Version,
    [string]$KokoroModelUrl = $env:KOKORO_TTS_RELEASE_KOKORO_MODEL_URL,
    [string]$OnnxRuntimeUrl = $env:KOKORO_TTS_RELEASE_ONNXRUNTIME_URL
)

$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "ReleasePackaging.Common.ps1")

if ([string]::IsNullOrWhiteSpace($Platform)) {
    if ($IsMacOS) { $Platform = "macos" }
    elseif ($IsLinux) { $Platform = "linux" }
    else { throw "Stage-PosixReleaseAssets.ps1 must run on Linux or macOS (or pass -Platform)." }
}
if ([string]::IsNullOrWhiteSpace($Architecture)) {
    $Architecture = if ($Platform -eq "macos") { "aarch64" } else { "x86_64" }
}

function New-CleanDirectory {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

function Invoke-Download {
    param(
        [Parameter(Mandatory = $true)][string]$Url,
        [Parameter(Mandatory = $true)][string]$DestinationPath
    )
    Invoke-WebRequest -Uri $Url -OutFile $DestinationPath
}

function Resolve-SingleMatch {
    param(
        [Parameter(Mandatory = $true)][System.IO.FileSystemInfo[]]$Items,
        [Parameter(Mandatory = $true)][string]$Description
    )
    if ($Items.Count -eq 1) { return $Items[0] }
    if ($Items.Count -eq 0) { throw "Expected exactly one $Description, found none." }
    $names = ($Items | ForEach-Object { "'$($_.FullName)'" }) -join ", "
    throw "Expected exactly one $Description, found $names."
}

function Expand-TarArchive {
    param(
        [Parameter(Mandatory = $true)][string]$ArchivePath,
        [Parameter(Mandatory = $true)][string]$DestinationPath
    )
    $tarCommand = Get-Command tar -ErrorAction SilentlyContinue
    if (-not $tarCommand) { throw "The 'tar' command is required to extract '$ArchivePath'." }
    New-CleanDirectory -Path $DestinationPath
    & $tarCommand.Source -xzf $ArchivePath -C $DestinationPath
    if ($LASTEXITCODE -ne 0) { throw "tar failed extracting '$ArchivePath'." }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoTomlPath = Join-Path $repoRoot "Cargo.toml"
$normalizedVersion = if ($Version) { $Version.Trim() } else { Get-PackageVersion -CargoTomlPath $cargoTomlPath }
if ($normalizedVersion.StartsWith("v")) { $normalizedVersion = $normalizedVersion.Substring(1) }

$releaseSources = Get-ReleaseSourcesConfig -RepoRoot $repoRoot

$ortSectionKey = if ($Platform -eq "macos") { "onnxruntime_macos_arm64" } else { "onnxruntime_linux_x64" }

$kokoroResolution = Resolve-ReleaseAssetUrl `
    -Name "KOKORO_TTS_RELEASE_KOKORO_MODEL_URL" `
    -EnvOverride $KokoroModelUrl `
    -LegacyOverride $env:LINGOPILOT_TTS_RELEASE_KOKORO_MODEL_URL `
    -ConfigValue $releaseSources.kokoro_model.url
$onnxResolution = Resolve-ReleaseAssetUrl `
    -Name "KOKORO_TTS_RELEASE_ONNXRUNTIME_URL" `
    -EnvOverride $OnnxRuntimeUrl `
    -LegacyOverride $null `
    -ConfigValue $releaseSources[$ortSectionKey].url

foreach ($r in @(
    @{ Name = 'KOKORO_TTS_RELEASE_KOKORO_MODEL_URL'; Resolution = $kokoroResolution },
    @{ Name = "release-sources.toml [$ortSectionKey].url"; Resolution = $onnxResolution }
)) {
    if ([string]::IsNullOrWhiteSpace($r.Resolution.Value)) {
        throw "$($r.Name) must be configured (via release-sources.toml or env override)."
    }
}

function Get-PinnedSha256 {
    param($Resolution, $ConfigSha256)
    if ($Resolution.Source -eq 'toml') { return $ConfigSha256 }
    return $null
}

$kokoroSha256 = Get-PinnedSha256 -Resolution $kokoroResolution -ConfigSha256 $releaseSources.kokoro_model.sha256
$onnxSha256   = Get-PinnedSha256 -Resolution $onnxResolution   -ConfigSha256 $releaseSources[$ortSectionKey].sha256

$stagingRoot = Join-Path $repoRoot "target/posix-staging/$Platform"
$downloadRoot = Join-Path $stagingRoot "_downloads"
$extractRoot = Join-Path $stagingRoot "_extract"
$modelDir = Join-Path $stagingRoot "kokoro-model"
$espeakRuntimeDir = Join-Path $stagingRoot "espeak-runtime"

New-CleanDirectory -Path $stagingRoot
New-CleanDirectory -Path $downloadRoot
New-CleanDirectory -Path $extractRoot
New-CleanDirectory -Path $modelDir
New-CleanDirectory -Path $espeakRuntimeDir

$kokoroArchivePath = Join-Path $downloadRoot "kokoro-model.zip"
$onnxArchivePath = Join-Path $downloadRoot ([System.IO.Path]::GetFileName(([System.Uri]$onnxResolution.Value).AbsolutePath))

function Write-AssetProvenance {
    param($Name, $Resolution, $Path, $ExpectedSha256)
    $actual = Assert-FileSha256 -Path $Path -Expected $ExpectedSha256
    $shaLabel = if ($actual) { $actual } else { 'skipped' }
    Write-Host ("[release-asset] {0} source={1} sha256={2}" -f $Name, $Resolution.Source, $shaLabel)
}

# --- Kokoro model ---
Invoke-Download -Url $kokoroResolution.Value -DestinationPath $kokoroArchivePath
Write-AssetProvenance -Name 'kokoro_model' -Resolution $kokoroResolution -Path $kokoroArchivePath -ExpectedSha256 $kokoroSha256
$kokoroExtractDir = Join-Path $extractRoot "kokoro"
New-CleanDirectory -Path $kokoroExtractDir
if ([System.IO.Path]::GetExtension($kokoroArchivePath) -ieq ".zip") {
    Expand-Archive -LiteralPath $kokoroArchivePath -DestinationPath $kokoroExtractDir -Force
} else {
    Expand-TarArchive -ArchivePath $kokoroArchivePath -DestinationPath $kokoroExtractDir
}
$modelPath = Resolve-SingleMatch -Items @(Get-ChildItem -LiteralPath $kokoroExtractDir -File -Recurse | Where-Object { $_.Name -like "*.onnx" }) -Description "Kokoro model (*.onnx) in archive"
$voicesPath = Resolve-SingleMatch -Items @(Get-ChildItem -LiteralPath $kokoroExtractDir -File -Recurse | Where-Object { $_.Name -like "voices*.bin" }) -Description "Kokoro voices bundle (voices*.bin) in archive"
Copy-Item -LiteralPath $modelPath.FullName -Destination (Join-Path $modelDir $modelPath.Name)
Copy-Item -LiteralPath $voicesPath.FullName -Destination (Join-Path $modelDir $voicesPath.Name)

# --- ONNX Runtime ---
Invoke-Download -Url $onnxResolution.Value -DestinationPath $onnxArchivePath
Write-AssetProvenance -Name $ortSectionKey -Resolution $onnxResolution -Path $onnxArchivePath -ExpectedSha256 $onnxSha256
$onnxExtractDir = Join-Path $extractRoot "onnxruntime"
Expand-TarArchive -ArchivePath $onnxArchivePath -DestinationPath $onnxExtractDir
$runtimeLibraryName = if ($Platform -eq "macos") { "libonnxruntime.dylib" } else { "libonnxruntime.so" }
$onnxLibCandidates = @(Get-ChildItem -LiteralPath $onnxExtractDir -File -Recurse | Where-Object {
    $_.Name -eq $runtimeLibraryName -or $_.Name -like "$runtimeLibraryName.*"
})
if ($onnxLibCandidates.Count -eq 0) {
    throw "No ONNX Runtime library matching '$runtimeLibraryName' found inside '$onnxArchivePath'."
}
# Prefer unversioned name; else take the first versioned one and rename.
$preferred = $onnxLibCandidates | Where-Object { $_.Name -eq $runtimeLibraryName } | Select-Object -First 1
if (-not $preferred) { $preferred = $onnxLibCandidates | Select-Object -First 1 }
$onnxRuntimeLibraryPath = Join-Path $stagingRoot $runtimeLibraryName
Copy-Item -LiteralPath $preferred.FullName -Destination $onnxRuntimeLibraryPath

# --- espeak runtime via system package manager ---
function Find-EspeakDataDir {
    param([string[]]$Candidates)
    foreach ($c in $Candidates) {
        if (-not [string]::IsNullOrWhiteSpace($c) -and (Test-Path -LiteralPath $c -PathType Container)) {
            return (Resolve-Path -LiteralPath $c).Path
        }
    }
    throw ("Could not locate espeak-ng-data under any of: " + ($Candidates -join ", "))
}

function Find-EspeakLibrary {
    param([string[]]$Candidates)
    foreach ($c in $Candidates) {
        if (-not [string]::IsNullOrWhiteSpace($c)) {
            $matches = @(Get-ChildItem -Path $c -ErrorAction SilentlyContinue | Where-Object {
                $_.Name -like "libespeak-ng*"
            })
            if ($matches.Count -gt 0) {
                # Prefer unversioned-suffixed symlink if present, else first hit.
                $pref = $matches | Where-Object { $_.Name -match "^libespeak-ng\.(so|dylib)$" } | Select-Object -First 1
                if (-not $pref) { $pref = $matches | Select-Object -First 1 }
                return (Resolve-Path -LiteralPath $pref.FullName).Path
            }
        }
    }
    throw ("Could not locate libespeak-ng library under any of: " + ($Candidates -join ", "))
}

if ($Platform -eq "linux") {
    # Ubuntu runner provides apt; installation is expected to be done by the CI step before invoking this script.
    $dataDir = Find-EspeakDataDir -Candidates @(
        "/usr/lib/x86_64-linux-gnu/espeak-ng-data",
        "/usr/share/espeak-ng-data"
    )
    $libPath = Find-EspeakLibrary -Candidates @(
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib"
    )
    $targetLibName = "libespeak-ng.so"
} else {
    $brewPrefix = & brew --prefix espeak-ng 2>$null
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($brewPrefix)) {
        throw "brew --prefix espeak-ng failed. Ensure 'brew install espeak-ng' ran before staging."
    }
    $brewPrefix = $brewPrefix.Trim()
    $dataDir = Find-EspeakDataDir -Candidates @(
        (Join-Path $brewPrefix "share/espeak-ng-data"),
        (Join-Path $brewPrefix "share/espeak-ng/espeak-ng-data")
    )
    $libPath = Find-EspeakLibrary -Candidates @(
        (Join-Path $brewPrefix "lib")
    )
    $targetLibName = "libespeak-ng.dylib"
}

Copy-Item -LiteralPath $dataDir -Destination (Join-Path $espeakRuntimeDir "espeak-ng-data") -Recurse
Copy-Item -LiteralPath $libPath -Destination (Join-Path $espeakRuntimeDir $targetLibName)

$binaryPath = Join-Path $repoRoot "target/release/lingopilot-tts-kokoro"
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw "Release binary not found at '$binaryPath'. Run 'cargo build --release --locked' first."
}

$summary = [ordered]@{
    KOKORO_TTS_POSIX_STAGING_DIR       = $stagingRoot
    KOKORO_TTS_POSIX_MODEL_DIR         = $modelDir
    KOKORO_TTS_POSIX_ONNXRUNTIME_LIB   = $onnxRuntimeLibraryPath
    KOKORO_TTS_POSIX_ESPEAK_RUNTIME    = $espeakRuntimeDir
    KOKORO_TTS_POSIX_BINARY            = $binaryPath
    KOKORO_TTS_POSIX_PLATFORM          = $Platform
    KOKORO_TTS_POSIX_ARCHITECTURE      = $Architecture
}

if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) {
    foreach ($entry in $summary.GetEnumerator()) {
        "$($entry.Key)=$($entry.Value)" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
    }
}

Write-Host "Staged POSIX release assets:" -ForegroundColor Green
foreach ($entry in $summary.GetEnumerator()) {
    Write-Host ("  {0}={1}" -f $entry.Key, $entry.Value) -ForegroundColor Green
}
