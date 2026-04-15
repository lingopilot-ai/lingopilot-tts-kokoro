param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("linux", "macos")]
    [string]$Platform,
    [Parameter(Mandatory = $true)]
    [ValidateSet("x86_64", "aarch64")]
    [string]$Architecture,
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,
    [Parameter(Mandatory = $true)]
    [string]$ModelDir,
    [Parameter(Mandatory = $true)]
    [string]$OnnxRuntimeLibrary,
    [Parameter(Mandatory = $true)]
    [string]$EspeakRuntimeDir,
    [string]$Version,
    [string]$OutputDir = (Join-Path (Join-Path $PSScriptRoot "..") "dist")
)

$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "ReleasePackaging.Common.ps1")

function New-TarGzArchive {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SourceDirectory,
        [Parameter(Mandatory = $true)]
        [string]$ArchivePath
    )

    $tarCommand = Get-Command tar -ErrorAction SilentlyContinue
    if (-not $tarCommand) {
        throw "The 'tar' command is required to create POSIX release archives."
    }

    $archiveDirectory = Split-Path -Parent $ArchivePath
    if (-not [string]::IsNullOrWhiteSpace($archiveDirectory)) {
        New-Item -ItemType Directory -Force -Path $archiveDirectory | Out-Null
    }

    $sourceParent = Split-Path -Parent $SourceDirectory
    $sourceLeaf = Split-Path -Leaf $SourceDirectory

    Push-Location $sourceParent
    try {
        & $tarCommand.Source -czf $ArchivePath $sourceLeaf
        if ($LASTEXITCODE -ne 0) {
            throw "The 'tar' command failed while creating '$ArchivePath'."
        }
    }
    finally {
        Pop-Location
    }
}

function Get-PosixRuntimeLibraryName {
    param([string]$PlatformName)

    if ($PlatformName -eq "macos") {
        return "libespeak-ng.dylib"
    }

    return "libespeak-ng.so"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoTomlPath = Join-Path $repoRoot "Cargo.toml"
$normalizedVersion = if ($Version) { $Version.Trim() } else { Get-PackageVersion -CargoTomlPath $cargoTomlPath }

if ($normalizedVersion.StartsWith("v")) {
    $normalizedVersion = $normalizedVersion.Substring(1)
}

$versionTag = "v$normalizedVersion"
$assetBase = "lingopilot-tts-kokoro-$versionTag-$Platform-$Architecture"
$binaryName = "lingopilot-tts-kokoro"
$runtimeLibraryName = if ($Platform -eq "macos") { "libonnxruntime.dylib" } else { "libonnxruntime.so" }
$espeakLibraryName = Get-PosixRuntimeLibraryName -PlatformName $Platform

$resolvedBinaryPath = Resolve-RequiredPath -Name "BinaryPath" -Value $BinaryPath -Kind File
$resolvedModelDir = Resolve-RequiredPath -Name "ModelDir" -Value $ModelDir -Kind Directory
$resolvedOnnxRuntimeLibrary = Resolve-RequiredPath -Name "OnnxRuntimeLibrary" -Value $OnnxRuntimeLibrary -Kind File
$resolvedEspeakRuntimeDir = Resolve-RequiredPath -Name "EspeakRuntimeDir" -Value $EspeakRuntimeDir -Kind Directory
$resolvedModelPath = Resolve-RequiredBundleFile -ModelDir $resolvedModelDir -Pattern "*.onnx" -Description "Kokoro model (*.onnx)"
$resolvedVoicesPath = Resolve-RequiredBundleFile -ModelDir $resolvedModelDir -Pattern "voices*.bin" -Description "Kokoro voices bundle (voices*.bin)"

$espeakDataDir = Join-Path $resolvedEspeakRuntimeDir "espeak-ng-data"
$espeakLibraryPath = Join-Path $resolvedEspeakRuntimeDir $espeakLibraryName
if (-not (Test-Path -LiteralPath $espeakDataDir -PathType Container)) {
    throw "The packaged runtime is incomplete: '$espeakDataDir' is missing."
}
if (-not (Test-Path -LiteralPath $espeakLibraryPath -PathType Leaf)) {
    throw "The packaged runtime is incomplete: '$espeakLibraryPath' is missing."
}
if ((Split-Path -Leaf $resolvedOnnxRuntimeLibrary) -ne $runtimeLibraryName) {
    throw "OnnxRuntimeLibrary must point to '$runtimeLibraryName' for platform '$Platform'."
}
if ((Split-Path -Leaf $resolvedBinaryPath) -ne $binaryName) {
    throw "BinaryPath must point to '$binaryName' for POSIX packaging."
}

$readmePath = Join-Path $repoRoot "README.md"
$licensePath = Join-Path $repoRoot "LICENSE"
$thirdPartyLicensesPath = Join-Path $repoRoot "THIRD_PARTY_LICENSES.txt"
foreach ($requiredPath in @($readmePath, $licensePath, $thirdPartyLicensesPath)) {
    if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) {
        throw "Required release input is missing: $requiredPath"
    }
}

$outputRoot = New-Item -ItemType Directory -Force -Path $OutputDir
$packageRoot = Join-Path $outputRoot.FullName $assetBase
$archivePath = Join-Path $outputRoot.FullName "$assetBase.tar.gz"
$checksumPath = Join-Path $outputRoot.FullName "lingopilot-tts-kokoro-$versionTag-sha256.txt"

if (Test-Path -LiteralPath $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}
if (Test-Path -LiteralPath $archivePath) {
    Remove-Item -LiteralPath $archivePath -Force
}

New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
$packageModelDir = Join-Path $packageRoot "kokoro-model"
New-Item -ItemType Directory -Force -Path $packageModelDir | Out-Null

Copy-Item -LiteralPath $resolvedBinaryPath -Destination (Join-Path $packageRoot $binaryName)
Copy-Item -LiteralPath $resolvedOnnxRuntimeLibrary -Destination (Join-Path $packageRoot $runtimeLibraryName)
Copy-Item -LiteralPath $resolvedEspeakRuntimeDir -Destination (Join-Path $packageRoot "espeak-runtime") -Recurse
Copy-Item -LiteralPath $resolvedModelPath -Destination (Join-Path $packageModelDir (Split-Path -Leaf $resolvedModelPath))
Copy-Item -LiteralPath $resolvedVoicesPath -Destination (Join-Path $packageModelDir (Split-Path -Leaf $resolvedVoicesPath))
Copy-Item -LiteralPath $readmePath -Destination (Join-Path $packageRoot "README.md")
Copy-Item -LiteralPath $licensePath -Destination (Join-Path $packageRoot "LICENSE")
Copy-Item -LiteralPath $thirdPartyLicensesPath -Destination (Join-Path $packageRoot "THIRD_PARTY_LICENSES.txt")

New-TarGzArchive -SourceDirectory $packageRoot -ArchivePath $archivePath
Update-ChecksumManifest -ChecksumPath $checksumPath -AssetPath $archivePath

Write-Host "Created POSIX release archive: $archivePath" -ForegroundColor Green
Write-Host "Updated checksum manifest: $checksumPath" -ForegroundColor Green

