param(
    [string]$Version,
    [string]$KokoroModelUrl = $env:KOKORO_TTS_RELEASE_KOKORO_MODEL_URL,
    [string]$OnnxRuntimeUrl = $env:KOKORO_TTS_RELEASE_ONNXRUNTIME_URL,
    [string]$PiperWindowsReleaseZipUrl = $env:KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL
)

$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "ReleasePackaging.Common.ps1")

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

$resolvedKokoroModelUrl = Resolve-RequiredUrl -Name "KOKORO_TTS_RELEASE_KOKORO_MODEL_URL" -Value $kokoroResolution.Value
$resolvedOnnxRuntimeUrl = Resolve-RequiredUrl -Name "KOKORO_TTS_RELEASE_ONNXRUNTIME_URL" -Value $onnxResolution.Value
$resolvedPiperZipUrl = Resolve-RequiredUrl -Name "KOKORO_TTS_RELEASE_PIPER_WINDOWS_ZIP_URL" -Value $piperResolution.Value

function Test-PinnedChecksum {
    param($Name, $Resolution, $ConfigSha256)
    if ($Resolution.Source -ne 'toml') { return }
    if ([string]::IsNullOrWhiteSpace($ConfigSha256)) {
        Write-Warning "$Name comes from release-sources.toml but sha256 is empty; staging will skip integrity verification."
    }
}

Test-PinnedChecksum -Name 'kokoro_model' -Resolution $kokoroResolution -ConfigSha256 $releaseSources.kokoro_model.sha256
Test-PinnedChecksum -Name 'onnxruntime'  -Resolution $onnxResolution   -ConfigSha256 $releaseSources.onnxruntime.sha256
Test-PinnedChecksum -Name 'piper_windows' -Resolution $piperResolution -ConfigSha256 $releaseSources.piper_windows.sha256

Write-Host "Release asset configuration is valid." -ForegroundColor Green
Write-Host ("  Kokoro model URL: {0} (source={1})" -f $resolvedKokoroModelUrl, $kokoroResolution.Source) -ForegroundColor Green
Write-Host ("  ONNX Runtime URL: {0} (source={1})" -f $resolvedOnnxRuntimeUrl, $onnxResolution.Source) -ForegroundColor Green
Write-Host ("  Piper Windows zip URL: {0} (source={1})" -f $resolvedPiperZipUrl, $piperResolution.Source) -ForegroundColor Green
