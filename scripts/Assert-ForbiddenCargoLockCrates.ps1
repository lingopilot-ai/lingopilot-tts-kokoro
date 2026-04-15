# Fails if Cargo.lock contains any crate banned by AGENTS.md §10 (Runtime Policy).
# Mirrors the [bans] section of deny.toml so this check catches regressions even
# without running cargo-deny (e.g. on developer machines before push).

[CmdletBinding()]
param(
    [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$lockPath = Join-Path $RepoRoot 'Cargo.lock'
if (-not (Test-Path $lockPath)) {
    throw "Cargo.lock not found at $lockPath"
}

$forbidden = @('espeak-rs-sys', 'espeak-rs', 'piper-rs')
$lock = Get-Content $lockPath -Raw
$hits = @()
foreach ($name in $forbidden) {
    if ($lock -match "(?m)^name = ""$([regex]::Escape($name))""\s*$") {
        $hits += $name
    }
}

if ($hits.Count -gt 0) {
    Write-Error ("Forbidden crates present in Cargo.lock: " + ($hits -join ', '))
    exit 1
}

Write-Host "Cargo.lock free of forbidden crates."
