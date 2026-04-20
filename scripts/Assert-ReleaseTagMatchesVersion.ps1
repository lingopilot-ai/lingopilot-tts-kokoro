param(
    [Parameter(Mandatory = $true)]
    [string]$Tag
)

$ErrorActionPreference = "Stop"

if ($Tag.StartsWith("refs/tags/")) {
    $Tag = $Tag.Substring("refs/tags/".Length)
}

if (-not $Tag.StartsWith("v")) {
    throw "Release tag '$Tag' is invalid. Expected format: v<crate-version>."
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoTomlPath = Join-Path $repoRoot "Cargo.toml"
$versionMatch = Select-String -Path $cargoTomlPath -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1

if (-not $versionMatch) {
    throw "Could not determine the package version from Cargo.toml."
}

$packageVersion = $versionMatch.Matches[0].Groups[1].Value
$expectedTag = "v$packageVersion"

$rcTagPattern = "^$([regex]::Escape($expectedTag))-rc[-.A-Za-z0-9]*$"
if ($Tag -ne $expectedTag -and $Tag -notmatch $rcTagPattern) {
    throw "Release tag '$Tag' does not match Cargo.toml version '$packageVersion'. Expected '$expectedTag' or '$expectedTag-rc*'."
}

Write-Host "Validated release tag $Tag against Cargo.toml version $packageVersion." -ForegroundColor Green
