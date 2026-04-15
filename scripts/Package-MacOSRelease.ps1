param(
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
    [string]$OutputDir = (Join-Path (Join-Path $PSScriptRoot "..") "dist"),
    [switch]$StageOnly
)

$ErrorActionPreference = "Stop"

$forwardedArgs = @{
    Platform           = "macos"
    Architecture       = $Architecture
    BinaryPath         = $BinaryPath
    ModelDir           = $ModelDir
    OnnxRuntimeLibrary = $OnnxRuntimeLibrary
    EspeakRuntimeDir   = $EspeakRuntimeDir
    Version            = $Version
    OutputDir          = $OutputDir
}
if ($PSBoundParameters.ContainsKey('StageOnly') -and $StageOnly) {
    $forwardedArgs['StageOnly'] = $true
}

& (Join-Path $PSScriptRoot "Package-PosixRelease.ps1") @forwardedArgs

