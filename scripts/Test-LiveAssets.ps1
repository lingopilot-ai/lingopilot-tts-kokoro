param(
    [string]$EspeakRuntimeDir = $env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR,
    [string]$ModelDir = $env:KOKORO_TTS_LIVE_MODEL_DIR,
    [string]$OnnxRuntimeDll = $env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL
)

$ErrorActionPreference = "Stop"

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
        return $PrimaryValue
    }

    if (-not [string]::IsNullOrWhiteSpace($LegacyValue)) {
        return $LegacyValue
    }

    return $null
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
        throw "$Name must be set to an absolute existing $Kind path."
    }

    if (-not [System.IO.Path]::IsPathRooted($Value)) {
        throw "$Name must be an absolute path. Got '$Value'."
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

$resolvedRuntimeDir = Resolve-RequiredPath `
    -Name "KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR" `
    -Value (Resolve-AliasedValue `
        -PrimaryName "KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR" `
        -PrimaryValue $EspeakRuntimeDir `
        -LegacyName "LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR" `
        -LegacyValue $env:LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR) `
    -Kind Directory
$resolvedModelDir = Resolve-RequiredPath `
    -Name "KOKORO_TTS_LIVE_MODEL_DIR" `
    -Value (Resolve-AliasedValue `
        -PrimaryName "KOKORO_TTS_LIVE_MODEL_DIR" `
        -PrimaryValue $ModelDir `
        -LegacyName "LINGOPILOT_TTS_LIVE_MODEL_DIR" `
        -LegacyValue $env:LINGOPILOT_TTS_LIVE_MODEL_DIR) `
    -Kind Directory
$resolvedOnnxRuntimeDll = Resolve-RequiredPath `
    -Name "KOKORO_TTS_LIVE_ONNXRUNTIME_DLL" `
    -Value (Resolve-AliasedValue `
        -PrimaryName "KOKORO_TTS_LIVE_ONNXRUNTIME_DLL" `
        -PrimaryValue $OnnxRuntimeDll `
        -LegacyName "LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL" `
        -LegacyValue $env:LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL) `
    -Kind File

$espeakDataDir = Join-Path $resolvedRuntimeDir "espeak-ng-data"
if (-not (Test-Path -LiteralPath $espeakDataDir -PathType Container)) {
    throw "KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR must contain 'espeak-ng-data'. Missing '$espeakDataDir'."
}

$env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR = $resolvedRuntimeDir
$env:KOKORO_TTS_LIVE_MODEL_DIR = $resolvedModelDir
$env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL = $resolvedOnnxRuntimeDll

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot
try {
    cargo test --locked -- --ignored
}
finally {
    Pop-Location
}
