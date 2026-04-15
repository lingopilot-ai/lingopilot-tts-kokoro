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
    [string]$OutputDir = (Join-Path (Join-Path $PSScriptRoot "..") "dist")
)

$ErrorActionPreference = "Stop"

& (Join-Path $PSScriptRoot "Package-PosixRelease.ps1") `
    -Platform "linux" `
    -Architecture $Architecture `
    -BinaryPath $BinaryPath `
    -ModelDir $ModelDir `
    -OnnxRuntimeLibrary $OnnxRuntimeLibrary `
    -EspeakRuntimeDir $EspeakRuntimeDir `
    -Version $Version `
    -OutputDir $OutputDir

