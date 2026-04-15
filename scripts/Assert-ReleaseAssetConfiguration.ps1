param(
    [string]$Version,
    [string]$KokoroModelUrl = $env:KOKORO_TTS_RELEASE_KOKORO_MODEL_URL,
    [string]$OnnxRuntimeUrl = $env:KOKORO_TTS_RELEASE_ONNXRUNTIME_URL,
    [string]$PiperWindowsReleaseZipUrl = $env:KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL
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
        return $PrimaryValue.Trim()
    }

    if (-not [string]::IsNullOrWhiteSpace($LegacyValue)) {
        return $LegacyValue.Trim()
    }

    return $null
}

function Resolve-RequiredUrl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [string]$Value
    )

    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw "$Name must be configured."
    }

    $trimmed = $Value.Trim()
    $uri = $null
    if (-not [System.Uri]::TryCreate($trimmed, [System.UriKind]::Absolute, [ref]$uri)) {
        throw "$Name must be an absolute URL. Got '$trimmed'."
    }

    if ($uri.Scheme -notin @("http", "https")) {
        throw "$Name must use http or https. Got '$trimmed'."
    }

    return $trimmed
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoTomlPath = Join-Path $repoRoot "Cargo.toml"
$normalizedVersion = if ($Version) { $Version.Trim() } else { Get-PackageVersion -CargoTomlPath $cargoTomlPath }
if ($normalizedVersion.StartsWith("v")) {
    $normalizedVersion = $normalizedVersion.Substring(1)
}

$versionTag = "v$normalizedVersion"
$resolvedKokoroModelUrl = Resolve-RequiredUrl `
    -Name "KOKORO_TTS_RELEASE_KOKORO_MODEL_URL" `
    -Value (Resolve-AliasedValue `
        -PrimaryName "KOKORO_TTS_RELEASE_KOKORO_MODEL_URL" `
        -PrimaryValue $KokoroModelUrl `
        -LegacyName "LINGOPILOT_TTS_RELEASE_KOKORO_MODEL_URL" `
        -LegacyValue $env:LINGOPILOT_TTS_RELEASE_KOKORO_MODEL_URL)
$resolvedOnnxRuntimeUrl = Resolve-RequiredUrl `
    -Name "KOKORO_TTS_RELEASE_ONNXRUNTIME_URL" `
    -Value (Resolve-AliasedValue `
        -PrimaryName "KOKORO_TTS_RELEASE_ONNXRUNTIME_URL" `
        -PrimaryValue $OnnxRuntimeUrl `
        -LegacyName "LINGOPILOT_TTS_RELEASE_ONNXRUNTIME_URL" `
        -LegacyValue $env:LINGOPILOT_TTS_RELEASE_ONNXRUNTIME_URL)
$resolvedPiperZipUrl = Resolve-AliasedValue `
    -PrimaryName "KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL" `
    -PrimaryValue $PiperWindowsReleaseZipUrl `
    -LegacyName "LINGOPILOT_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL" `
    -LegacyValue $env:LINGOPILOT_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL

if ([string]::IsNullOrWhiteSpace($resolvedPiperZipUrl)) {
    $resolvedPiperZipUrl = "https://github.com/lingopilot-ai/lingopilot-tts-piper/releases/download/$versionTag/lingopilot-tts-piper-$versionTag-windows-x86_64.zip"
}

$resolvedPiperZipUrl = Resolve-RequiredUrl `
    -Name "KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL" `
    -Value $resolvedPiperZipUrl

Write-Host "Release asset configuration is valid." -ForegroundColor Green
Write-Host "  Kokoro model URL: $resolvedKokoroModelUrl" -ForegroundColor Green
Write-Host "  ONNX Runtime URL: $resolvedOnnxRuntimeUrl" -ForegroundColor Green
Write-Host "  Piper Windows zip URL: $resolvedPiperZipUrl" -ForegroundColor Green
