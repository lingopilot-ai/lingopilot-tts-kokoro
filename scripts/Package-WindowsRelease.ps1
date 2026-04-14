param(
    [string]$ModelDir,
    [string]$OnnxRuntimeDll,
    [string]$Version,
    [string]$OutputDir = (Join-Path (Join-Path $PSScriptRoot "..") "dist")
)

$ErrorActionPreference = "Stop"

function Get-PackageVersion {
    param([string]$CargoTomlPath)

    $versionMatch = Select-String -Path $CargoTomlPath -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $versionMatch) {
        throw "Could not determine the package version from Cargo.toml."
    }

    return $versionMatch.Matches[0].Groups[1].Value
}

function Resolve-RequiredPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$Value,
        [Parameter(Mandatory = $true)]
        [ValidateSet("Directory", "File")]
        [string]$Kind
    )

    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw "$Name must point to an existing $Kind path."
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

    return $resolved
}

function Resolve-StagedReleaseInput {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [string]$ProvidedValue,
        [Parameter(Mandatory = $true)]
        [string]$DefaultValue,
        [Parameter(Mandatory = $true)]
        [ValidateSet("Directory", "File")]
        [string]$Kind
    )

    if (-not [string]::IsNullOrWhiteSpace($ProvidedValue)) {
        return Resolve-RequiredPath -Name $Name -Value $ProvidedValue -Kind $Kind
    }

    if (-not (Test-Path -LiteralPath $DefaultValue)) {
        throw "$Name was not provided and no staged default exists at '$DefaultValue'. Run scripts\\Stage-WindowsReleaseAssets.ps1 first or pass -$Name explicitly."
    }

    return Resolve-RequiredPath -Name $Name -Value $DefaultValue -Kind $Kind
}

function Resolve-RequiredBundleFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ModelDir,
        [Parameter(Mandatory = $true)]
        [string]$Pattern,
        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    $matches = @(
        Get-ChildItem -LiteralPath $ModelDir -File |
            Where-Object { $_.Name -like $Pattern } |
            Sort-Object -Property Name
    )

    if ($matches.Count -eq 1) {
        return $matches[0].FullName
    }

    if ($matches.Count -eq 0) {
        throw "Cannot use model_dir '$ModelDir': expected exactly one $Description"
    }

    $names = ($matches | ForEach-Object { "'$($_.Name)'" }) -join ", "
    throw "Cannot use model_dir '$ModelDir': expected exactly one $Description, found $names"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoTomlPath = Join-Path $repoRoot "Cargo.toml"
$normalizedVersion = if ($Version) { $Version.Trim() } else { Get-PackageVersion -CargoTomlPath $cargoTomlPath }

if ($normalizedVersion.StartsWith("v")) {
    $normalizedVersion = $normalizedVersion.Substring(1)
}

$versionTag = "v$normalizedVersion"
$assetBase = "lingopilot-tts-kokoro-$versionTag-windows-x86_64"
$binaryPath = Join-Path $repoRoot "target\release\lingopilot-tts-kokoro.exe"
$runtimeDir = Join-Path $repoRoot "target\release\espeak-runtime"
$packagingDir = Join-Path $repoRoot "packaging\windows"
$defaultModelDir = Join-Path $packagingDir "kokoro-model"
$defaultOnnxRuntimeDll = Join-Path $packagingDir "onnxruntime.dll"
$readmePath = Join-Path $repoRoot "README.md"
$licensePath = Join-Path $repoRoot "LICENSE"
$thirdPartyLicensesPath = Join-Path $repoRoot "THIRD_PARTY_LICENSES.txt"
$resolvedModelDir = Resolve-StagedReleaseInput -Name "ModelDir" -ProvidedValue $ModelDir -DefaultValue $defaultModelDir -Kind Directory
$resolvedOnnxRuntimeDll = Resolve-StagedReleaseInput -Name "OnnxRuntimeDll" -ProvidedValue $OnnxRuntimeDll -DefaultValue $defaultOnnxRuntimeDll -Kind File
$resolvedModelPath = Resolve-RequiredBundleFile -ModelDir $resolvedModelDir -Pattern "*.onnx" -Description "Kokoro model (*.onnx)"
$resolvedVoicesPath = Resolve-RequiredBundleFile -ModelDir $resolvedModelDir -Pattern "voices*.bin" -Description "Kokoro voices bundle (voices*.bin)"

foreach ($requiredPath in @($binaryPath, $runtimeDir, $readmePath, $licensePath, $thirdPartyLicensesPath)) {
    if (-not (Test-Path $requiredPath)) {
        throw "Required release input is missing: $requiredPath"
    }
}

if (-not (Test-Path (Join-Path $runtimeDir "espeak-ng-data"))) {
    throw "The packaged runtime is incomplete: '$runtimeDir\espeak-ng-data' is missing."
}

$espeakLibraryPath = Join-Path $runtimeDir "espeak-ng.dll"
if (-not (Test-Path -LiteralPath $espeakLibraryPath -PathType Leaf)) {
    throw "The packaged runtime is incomplete: '$espeakLibraryPath' is missing."
}

$outputRoot = New-Item -ItemType Directory -Force -Path $OutputDir
$packageRoot = Join-Path $outputRoot.FullName $assetBase
$zipPath = Join-Path $outputRoot.FullName "$assetBase.zip"
$checksumPath = Join-Path $outputRoot.FullName "lingopilot-tts-kokoro-$versionTag-sha256.txt"

if (Test-Path $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}

if (Test-Path $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}

New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
$packageModelDir = Join-Path $packageRoot "kokoro-model"
New-Item -ItemType Directory -Force -Path $packageModelDir | Out-Null

Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $packageRoot "lingopilot-tts-kokoro.exe")
Copy-Item -LiteralPath $resolvedOnnxRuntimeDll -Destination (Join-Path $packageRoot "onnxruntime.dll")
Copy-Item -LiteralPath $runtimeDir -Destination (Join-Path $packageRoot "espeak-runtime") -Recurse
Copy-Item -LiteralPath $resolvedModelPath -Destination (Join-Path $packageModelDir (Split-Path -Leaf $resolvedModelPath))
Copy-Item -LiteralPath $resolvedVoicesPath -Destination (Join-Path $packageModelDir (Split-Path -Leaf $resolvedVoicesPath))
Copy-Item -LiteralPath $readmePath -Destination (Join-Path $packageRoot "README.md")
Copy-Item -LiteralPath $licensePath -Destination (Join-Path $packageRoot "LICENSE")
Copy-Item -LiteralPath $thirdPartyLicensesPath -Destination (Join-Path $packageRoot "THIRD_PARTY_LICENSES.txt")

Compress-Archive -LiteralPath $packageRoot -DestinationPath $zipPath -Force

$hash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
$checksumLine = "{0}  {1}" -f $hash, (Split-Path -Leaf $zipPath)
Set-Content -LiteralPath $checksumPath -Value $checksumLine -NoNewline

Write-Host "Created release archive: $zipPath" -ForegroundColor Green
Write-Host "Created checksum manifest: $checksumPath" -ForegroundColor Green
