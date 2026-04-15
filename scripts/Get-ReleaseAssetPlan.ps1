param(
    [string]$Version,
    [string]$OutputPath,
    [switch]$AsJson
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

function Get-ArchiveExtension {
    param([string]$Platform)

    if ($Platform -eq "windows") {
        return ".zip"
    }

    return ".tar.gz"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoTomlPath = Join-Path $repoRoot "Cargo.toml"
$normalizedVersion = if ([string]::IsNullOrWhiteSpace($Version)) {
    Get-PackageVersion -CargoTomlPath $cargoTomlPath
} else {
    $Version.Trim()
}

if ($normalizedVersion.StartsWith("v")) {
    $normalizedVersion = $normalizedVersion.Substring(1)
}

$versionTag = "v$normalizedVersion"
$releaseTargets = @(
    @{
        platform = "windows"
        arch = "x86_64"
        archive_extension = ".zip"
        binary_name = "lingopilot-tts-kokoro.exe"
        runtime_library = "onnxruntime.dll"
        espeak_library = "espeak-ng.dll"
        validation_state = "released"
    },
    @{
        platform = "linux"
        arch = "x86_64"
        archive_extension = ".tar.gz"
        binary_name = "lingopilot-tts-kokoro"
        runtime_library = "libonnxruntime.so"
        espeak_library = "libespeak-ng.so"
        validation_state = "planned"
    },
    @{
        platform = "linux"
        arch = "aarch64"
        archive_extension = ".tar.gz"
        binary_name = "lingopilot-tts-kokoro"
        runtime_library = "libonnxruntime.so"
        espeak_library = "libespeak-ng.so"
        validation_state = "planned"
    },
    @{
        platform = "macos"
        arch = "x86_64"
        archive_extension = ".tar.gz"
        binary_name = "lingopilot-tts-kokoro"
        runtime_library = "libonnxruntime.dylib"
        espeak_library = "libespeak-ng.dylib"
        validation_state = "planned"
    },
    @{
        platform = "macos"
        arch = "aarch64"
        archive_extension = ".tar.gz"
        binary_name = "lingopilot-tts-kokoro"
        runtime_library = "libonnxruntime.dylib"
        espeak_library = "libespeak-ng.dylib"
        validation_state = "planned"
    }
)

$plan = foreach ($target in $releaseTargets) {
    $assetBase = "lingopilot-tts-kokoro-$versionTag-$($target.platform)-$($target.arch)"
    [pscustomobject]@{
        version = $versionTag
        platform = $target.platform
        arch = $target.arch
        asset_name = "$assetBase$($target.archive_extension)"
        package_root = $assetBase
        checksum_manifest = "lingopilot-tts-kokoro-$versionTag-sha256.txt"
        binary_name = $target.binary_name
        runtime_library = $target.runtime_library
        espeak_library = $target.espeak_library
        package_model_dir = "kokoro-model"
        package_runtime_dir = "espeak-runtime"
        validation_state = $target.validation_state
    }
}

if ($OutputPath) {
    $outputDirectory = Split-Path -Parent $OutputPath
    if (-not [string]::IsNullOrWhiteSpace($outputDirectory)) {
        New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    }

    if ($AsJson) {
        $plan | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $OutputPath
    } else {
        $plan | Format-Table -AutoSize | Out-String | Set-Content -LiteralPath $OutputPath
    }
}

if ($AsJson) {
    $plan | ConvertTo-Json -Depth 4
} else {
    $plan | Format-Table -AutoSize
}
