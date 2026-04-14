param(
    [string]$EspeakRuntimeDir = $env:LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR,
    [string]$ModelDir = $env:LINGOPILOT_TTS_LIVE_MODEL_DIR,
    [string]$OnnxRuntimeDll = $env:LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL
)

$ErrorActionPreference = "Stop"

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

$resolvedRuntimeDir = Resolve-RequiredPath -Name "LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR" -Value $EspeakRuntimeDir -Kind Directory
$resolvedModelDir = Resolve-RequiredPath -Name "LINGOPILOT_TTS_LIVE_MODEL_DIR" -Value $ModelDir -Kind Directory
$resolvedOnnxRuntimeDll = Resolve-RequiredPath -Name "LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL" -Value $OnnxRuntimeDll -Kind File

$espeakDataDir = Join-Path $resolvedRuntimeDir "espeak-ng-data"
if (-not (Test-Path -LiteralPath $espeakDataDir -PathType Container)) {
    throw "LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR must contain 'espeak-ng-data'. Missing '$espeakDataDir'."
}

$env:LINGOPILOT_TTS_LIVE_ESPEAK_RUNTIME_DIR = $resolvedRuntimeDir
$env:LINGOPILOT_TTS_LIVE_MODEL_DIR = $resolvedModelDir
$env:LINGOPILOT_TTS_LIVE_ONNXRUNTIME_DLL = $resolvedOnnxRuntimeDll

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot
try {
    cargo test --locked -- --ignored
}
finally {
    Pop-Location
}
