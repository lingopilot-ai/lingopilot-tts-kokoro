param(
    [string]$SiblingTomlPath,
    [string]$SiblingTomlUrl,
    [switch]$SkipNetworkFallback
)

# Enforces ORT-pin parity between this repo's `release-sources.toml` and the
# sibling sidecar's (`lingopilot-tts-kokoro` ↔ `lingopilot-tts-piper`).
#
# Resolution order for the sibling manifest:
#   1. -SiblingTomlPath argument
#   2. LINGOPILOT_TTS_PARITY_SIBLING_TOML environment variable
#   3. ../<peer-repo>/release-sources.toml on disk
#   4. raw GitHub URL (unless -SkipNetworkFallback)
#
# Fails loudly on any mismatch in the `[onnxruntime*]` sections so drift
# cannot reach a packaged artifact.

$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "ReleasePackaging.Common.ps1")

function ConvertFrom-ReleaseSourcesTomlText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    $tempPath = Join-Path ([System.IO.Path]::GetTempPath()) ("lingopilot-release-sources-" + [System.Guid]::NewGuid().ToString("N") + ".toml")
    try {
        Set-Content -LiteralPath $tempPath -Value $Text -Encoding UTF8
        return ConvertFrom-ReleaseSourcesToml -Path $tempPath
    }
    finally {
        if (Test-Path -LiteralPath $tempPath) {
            Remove-Item -LiteralPath $tempPath -Force
        }
    }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$localTomlPath = Join-Path $repoRoot "release-sources.toml"
if (-not (Test-Path -LiteralPath $localTomlPath)) {
    throw "Local release-sources.toml not found at '$localTomlPath'."
}

$cargoTomlPath = Join-Path $repoRoot "Cargo.toml"
if (-not (Test-Path -LiteralPath $cargoTomlPath)) {
    throw "Cargo.toml not found at '$cargoTomlPath'."
}

$packageNameMatch = Select-String -Path $cargoTomlPath -Pattern '^\s*name\s*=\s*"(lingopilot-tts-[a-z]+)"' | Select-Object -First 1
if (-not $packageNameMatch) {
    throw "Could not determine local package name from Cargo.toml."
}
$localName = $packageNameMatch.Matches[0].Groups[1].Value
$peerName = switch ($localName) {
    'lingopilot-tts-kokoro' { 'lingopilot-tts-piper' }
    'lingopilot-tts-piper'  { 'lingopilot-tts-kokoro' }
    default { throw "ORT pin parity only supports lingopilot-tts-{piper,kokoro}; got '$localName'." }
}

function Resolve-SiblingToml {
    param(
        [string]$ExplicitPath,
        [string]$ExplicitUrl,
        [string]$PeerName,
        [string]$RepoRoot,
        [switch]$SkipNetwork
    )

    $envPath = [System.Environment]::GetEnvironmentVariable('LINGOPILOT_TTS_PARITY_SIBLING_TOML')

    foreach ($candidate in @($ExplicitPath, $envPath)) {
        if (-not [string]::IsNullOrWhiteSpace($candidate)) {
            if (-not (Test-Path -LiteralPath $candidate)) {
                throw "Sibling release-sources.toml override '$candidate' does not exist."
            }
            return [pscustomobject]@{ Source = "file:$candidate"; Text = (Get-Content -LiteralPath $candidate -Raw) }
        }
    }

    $siblingDiskPath = Join-Path (Split-Path -Parent $RepoRoot) (Join-Path $PeerName "release-sources.toml")
    if (Test-Path -LiteralPath $siblingDiskPath) {
        return [pscustomobject]@{ Source = "file:$siblingDiskPath"; Text = (Get-Content -LiteralPath $siblingDiskPath -Raw) }
    }

    if ($SkipNetwork) {
        throw "Sibling release-sources.toml not found on disk and network fallback disabled. Tried '$siblingDiskPath'."
    }

    $url = if (-not [string]::IsNullOrWhiteSpace($ExplicitUrl)) {
        $ExplicitUrl
    } else {
        "https://raw.githubusercontent.com/lingopilot-ai/$PeerName/main/release-sources.toml"
    }

    Write-Host "Fetching sibling manifest from $url"
    try {
        $response = Invoke-WebRequest -Uri $url -UseBasicParsing
    } catch {
        throw "Failed to fetch sibling release-sources.toml from '$url': $($_.Exception.Message)"
    }
    return [pscustomobject]@{ Source = "url:$url"; Text = $response.Content }
}

$local = ConvertFrom-ReleaseSourcesToml -Path $localTomlPath
$sibling = Resolve-SiblingToml `
    -ExplicitPath $SiblingTomlPath `
    -ExplicitUrl $SiblingTomlUrl `
    -PeerName $peerName `
    -RepoRoot $repoRoot `
    -SkipNetwork:$SkipNetworkFallback
$siblingTable = ConvertFrom-ReleaseSourcesTomlText -Text $sibling.Text

$sections = @('onnxruntime', 'onnxruntime_linux_x64', 'onnxruntime_macos_arm64')
$fields = @('url', 'sha256')
$mismatches = @()

foreach ($section in $sections) {
    $localSection = if ($local.ContainsKey($section)) { $local[$section] } else { @{} }
    $siblingSection = if ($siblingTable.ContainsKey($section)) { $siblingTable[$section] } else { @{} }

    foreach ($field in $fields) {
        $localValue = $localSection[$field]
        $siblingValue = $siblingSection[$field]

        if ([string]::IsNullOrWhiteSpace($localValue)) {
            $mismatches += "[$section] $field — missing in $localName"
            continue
        }
        if ([string]::IsNullOrWhiteSpace($siblingValue)) {
            $mismatches += "[$section] $field — missing in $peerName"
            continue
        }
        if ($localValue.Trim() -ne $siblingValue.Trim()) {
            $mismatches += "[$section] $field" + [System.Environment]::NewLine + `
                "    $localName = $localValue" + [System.Environment]::NewLine + `
                "    $peerName  = $siblingValue"
        }
    }
}

if ($mismatches.Count -gt 0) {
    $joined = ($mismatches -join [System.Environment]::NewLine)
    $lines = @(
        "ORT pin parity check FAILED.",
        "Local  : $localTomlPath",
        "Sibling: $($sibling.Source)",
        "",
        $joined,
        "",
        "The [onnxruntime*] sections must match byte-for-byte across both sidecars.",
        "Bump both repos together, or export LINGOPILOT_TTS_PARITY_SIBLING_TOML to point at a staged manifest."
    )
    throw ($lines -join [System.Environment]::NewLine)
}

Write-Host "ORT pin parity OK ($localName ↔ $peerName, source: $($sibling.Source))."
