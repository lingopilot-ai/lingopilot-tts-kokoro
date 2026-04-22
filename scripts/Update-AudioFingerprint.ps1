param()

$ErrorActionPreference = "Stop"
$script = Join-Path $PSScriptRoot "Run-LiveSmoke.ps1"
& $script -UpdateFingerprint
