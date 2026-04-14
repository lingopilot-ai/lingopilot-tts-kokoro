param(
    [string]$CargoLockPath = (Join-Path (Join-Path $PSScriptRoot "..") "Cargo.lock")
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $CargoLockPath -PathType Leaf)) {
    throw "Cargo.lock is missing at '$CargoLockPath'."
}

if (Select-String -Path $CargoLockPath -Pattern '^name = "(espeak-rs-sys|espeak-rs|piper-rs)"$' -Quiet) {
    throw "Forbidden GPL crate appeared in Cargo.lock"
}
