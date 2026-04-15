param(
    [string]$Version,
    [string]$KokoroModelUrl = $env:KOKORO_TTS_RELEASE_KOKORO_MODEL_URL,
    [string]$OnnxRuntimeUrl = $env:KOKORO_TTS_RELEASE_ONNXRUNTIME_URL,
    [string]$PiperWindowsReleaseZipUrl = $env:KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL
)

$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "ReleasePackaging.Common.ps1")

function Get-PackageVersion {
    param([string]$CargoTomlPath)

    $versionMatch = Select-String -Path $CargoTomlPath -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $versionMatch) {
        throw "Could not determine the package version from Cargo.toml."
    }

    return $versionMatch.Matches[0].Groups[1].Value
}

function Resolve-NonEmptyValue {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [string]$Value
    )

    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw "$Name must be configured. Set $Name before running this script."
    }

    return $Value.Trim()
}

function Resolve-AliasedValue {
    param(
        [Parameter(Mandatory = $true)]
        [string]$PrimaryName,
        [string]$PrimaryValue,
        [Parameter(Mandatory = $true)]
        [string]$LegacyName,
        [string]$LegacyValue
    )

    if (-not [string]::IsNullOrWhiteSpace($PrimaryValue)) {
        return Resolve-NonEmptyValue -Name $PrimaryName -Value $PrimaryValue
    }

    if (-not [string]::IsNullOrWhiteSpace($LegacyValue)) {
        return Resolve-NonEmptyValue -Name $PrimaryName -Value $LegacyValue
    }

    throw "$PrimaryName must be configured. Legacy alias accepted temporarily: $LegacyName."
}

function New-CleanDirectory {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Recurse -Force
    }

    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

function Remove-IfExists {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
}

function Invoke-Download {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Url,
        [Parameter(Mandatory = $true)]
        [string]$DestinationPath
    )

    Invoke-WebRequest -Uri $Url -OutFile $DestinationPath
}

function Resolve-SingleMatch {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.FileSystemInfo[]]$Items,
        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    if ($Items.Count -eq 1) {
        return $Items[0]
    }

    if ($Items.Count -eq 0) {
        throw "Expected exactly one $Description, found none."
    }

    $names = ($Items | ForEach-Object { "'$($_.FullName)'" }) -join ", "
    throw "Expected exactly one $Description, found $names."
}

function Expand-ArchiveToDirectory {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ArchivePath,
        [Parameter(Mandatory = $true)]
        [string]$DestinationPath
    )

    New-CleanDirectory -Path $DestinationPath
    Expand-Archive -LiteralPath $ArchivePath -DestinationPath $DestinationPath -Force
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoTomlPath = Join-Path $repoRoot "Cargo.toml"
$normalizedVersion = if ($Version) { $Version.Trim() } else { Get-PackageVersion -CargoTomlPath $cargoTomlPath }
if ($normalizedVersion.StartsWith("v")) {
    $normalizedVersion = $normalizedVersion.Substring(1)
}

$versionTag = "v$normalizedVersion"

$releaseSources = Get-ReleaseSourcesConfig -RepoRoot $repoRoot

$kokoroResolution = Resolve-ReleaseAssetUrl `
    -Name "KOKORO_TTS_RELEASE_KOKORO_MODEL_URL" `
    -EnvOverride $KokoroModelUrl `
    -LegacyOverride $env:LINGOPILOT_TTS_RELEASE_KOKORO_MODEL_URL `
    -ConfigValue $releaseSources.kokoro_model.url
$onnxResolution = Resolve-ReleaseAssetUrl `
    -Name "KOKORO_TTS_RELEASE_ONNXRUNTIME_URL" `
    -EnvOverride $OnnxRuntimeUrl `
    -LegacyOverride $env:LINGOPILOT_TTS_RELEASE_ONNXRUNTIME_URL `
    -ConfigValue $releaseSources.onnxruntime.url
$piperResolution = Resolve-ReleaseAssetUrl `
    -Name "KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL" `
    -EnvOverride $PiperWindowsReleaseZipUrl `
    -LegacyOverride $env:LINGOPILOT_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL `
    -ConfigValue $releaseSources.piper_windows.url

if ([string]::IsNullOrWhiteSpace($piperResolution.Value)) {
    $piperResolution = [pscustomobject]@{
        Value  = "https://github.com/lingopilot-ai/lingopilot-tts-piper/releases/download/$versionTag/lingopilot-tts-piper-$versionTag-windows-x86_64.zip"
        Source = 'derived'
    }
}

foreach ($r in @(
    @{ Name = 'KOKORO_TTS_RELEASE_KOKORO_MODEL_URL'; Resolution = $kokoroResolution },
    @{ Name = 'KOKORO_TTS_RELEASE_ONNXRUNTIME_URL';  Resolution = $onnxResolution },
    @{ Name = 'KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL'; Resolution = $piperResolution }
)) {
    if ([string]::IsNullOrWhiteSpace($r.Resolution.Value)) {
        throw "$($r.Name) must be configured (via release-sources.toml or env override)."
    }
}

$resolvedKokoroModelUrl = $kokoroResolution.Value
$resolvedOnnxRuntimeUrl = $onnxResolution.Value
$resolvedPiperZipUrl = $piperResolution.Value

function Get-PinnedSha256 {
    param($Resolution, $ConfigSha256)
    if ($Resolution.Source -eq 'toml') { return $ConfigSha256 }
    return $null
}

$kokoroSha256 = Get-PinnedSha256 -Resolution $kokoroResolution -ConfigSha256 $releaseSources.kokoro_model.sha256
$onnxSha256   = Get-PinnedSha256 -Resolution $onnxResolution   -ConfigSha256 $releaseSources.onnxruntime.sha256
$piperSha256  = Get-PinnedSha256 -Resolution $piperResolution  -ConfigSha256 $releaseSources.piper_windows.sha256

$downloadRoot = Join-Path $repoRoot "target\release-staging-downloads"
$extractRoot = Join-Path $repoRoot "target\release-staging-extract"
$releaseRuntimeDir = Join-Path $repoRoot "target\release\espeak-runtime"
$packagingDir = Join-Path $repoRoot "packaging\windows"
$packagingModelDir = Join-Path $packagingDir "kokoro-model"
$packagingOnnxRuntimeDll = Join-Path $packagingDir "onnxruntime.dll"

New-CleanDirectory -Path $downloadRoot
New-CleanDirectory -Path $extractRoot
New-Item -ItemType Directory -Force -Path $packagingDir | Out-Null
Remove-IfExists -Path $packagingModelDir
New-CleanDirectory -Path $packagingModelDir
Remove-IfExists -Path $packagingOnnxRuntimeDll

$piperZipPath = Join-Path $downloadRoot "piper-release.zip"
$kokoroArchivePath = Join-Path $downloadRoot "kokoro-model.zip"
$onnxRuntimeDownloadPath = Join-Path $downloadRoot ([System.IO.Path]::GetFileName(([System.Uri]$resolvedOnnxRuntimeUrl).AbsolutePath))

function Write-AssetProvenance {
    param($Name, $Resolution, $Path, $ExpectedSha256)
    $actual = Assert-FileSha256 -Path $Path -Expected $ExpectedSha256
    $shaLabel = if ($actual) { $actual } else { 'skipped' }
    Write-Host ("[release-asset] {0} source={1} sha256={2}" -f $Name, $Resolution.Source, $shaLabel)
}

Invoke-Download -Url $resolvedPiperZipUrl -DestinationPath $piperZipPath
Write-AssetProvenance -Name 'piper_windows' -Resolution $piperResolution -Path $piperZipPath -ExpectedSha256 $piperSha256
Invoke-Download -Url $resolvedKokoroModelUrl -DestinationPath $kokoroArchivePath
Write-AssetProvenance -Name 'kokoro_model' -Resolution $kokoroResolution -Path $kokoroArchivePath -ExpectedSha256 $kokoroSha256
Invoke-Download -Url $resolvedOnnxRuntimeUrl -DestinationPath $onnxRuntimeDownloadPath
Write-AssetProvenance -Name 'onnxruntime' -Resolution $onnxResolution -Path $onnxRuntimeDownloadPath -ExpectedSha256 $onnxSha256

$piperExtractDir = Join-Path $extractRoot "piper"
Expand-ArchiveToDirectory -ArchivePath $piperZipPath -DestinationPath $piperExtractDir
$espeakRuntimeDir = Resolve-SingleMatch -Items @(Get-ChildItem -LiteralPath $piperExtractDir -Directory -Recurse | Where-Object { $_.Name -eq "espeak-runtime" }) -Description "espeak-runtime directory inside the Piper release archive"
if (Test-Path -LiteralPath $releaseRuntimeDir) {
    Remove-Item -LiteralPath $releaseRuntimeDir -Recurse -Force
}
Copy-Item -LiteralPath $espeakRuntimeDir.FullName -Destination $releaseRuntimeDir -Recurse

$kokoroExtractDir = Join-Path $extractRoot "kokoro"
Expand-ArchiveToDirectory -ArchivePath $kokoroArchivePath -DestinationPath $kokoroExtractDir
$modelPath = Resolve-SingleMatch -Items @(Get-ChildItem -LiteralPath $kokoroExtractDir -File -Recurse | Where-Object { $_.Name -like "*.onnx" }) -Description "Kokoro model (*.onnx) inside the Kokoro model archive"
$voicesPath = Resolve-SingleMatch -Items @(Get-ChildItem -LiteralPath $kokoroExtractDir -File -Recurse | Where-Object { $_.Name -like "voices*.bin" }) -Description "Kokoro voices bundle (voices*.bin) inside the Kokoro model archive"
Copy-Item -LiteralPath $modelPath.FullName -Destination (Join-Path $packagingModelDir $modelPath.Name)
Copy-Item -LiteralPath $voicesPath.FullName -Destination (Join-Path $packagingModelDir $voicesPath.Name)

if ([System.IO.Path]::GetExtension($onnxRuntimeDownloadPath) -ieq ".zip") {
    $onnxExtractDir = Join-Path $extractRoot "onnxruntime"
    Expand-ArchiveToDirectory -ArchivePath $onnxRuntimeDownloadPath -DestinationPath $onnxExtractDir
    $onnxRuntimeDll = Resolve-SingleMatch -Items @(Get-ChildItem -LiteralPath $onnxExtractDir -File -Recurse | Where-Object { $_.Name -ieq "onnxruntime.dll" }) -Description "onnxruntime.dll inside the ONNX Runtime archive"
    Copy-Item -LiteralPath $onnxRuntimeDll.FullName -Destination $packagingOnnxRuntimeDll
} else {
    if ([System.IO.Path]::GetFileName($onnxRuntimeDownloadPath) -ine "onnxruntime.dll") {
        throw "KOKORO_TTS_RELEASE_ONNXRUNTIME_URL must point to onnxruntime.dll or to a zip that contains it. Got '$onnxRuntimeDownloadPath'."
    }
    Copy-Item -LiteralPath $onnxRuntimeDownloadPath -Destination $packagingOnnxRuntimeDll
}

Write-Host "Staged release assets:" -ForegroundColor Green
Write-Host "  eSpeak runtime: $releaseRuntimeDir" -ForegroundColor Green
Write-Host "  Kokoro model dir: $packagingModelDir" -ForegroundColor Green
Write-Host "  ONNX Runtime DLL: $packagingOnnxRuntimeDll" -ForegroundColor Green
