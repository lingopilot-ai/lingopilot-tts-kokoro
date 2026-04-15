param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$Repository = "lingopilot-ai/lingopilot-tts-kokoro",
    [string]$DownloadRoot = (Join-Path ([System.IO.Path]::GetTempPath()) ("lingopilot-tts-kokoro-published-release-" + [System.Guid]::NewGuid().ToString("N")))
)

$ErrorActionPreference = "Stop"

function Resolve-ReleaseDownloadUrl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Repository,
        [Parameter(Mandatory = $true)]
        [string]$VersionTag,
        [Parameter(Mandatory = $true)]
        [string]$AssetName
    )

    return "https://github.com/$Repository/releases/download/$VersionTag/$AssetName"
}

function Invoke-ReleaseDownload {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Url,
        [Parameter(Mandatory = $true)]
        [string]$DestinationPath
    )

    Invoke-WebRequest -Uri $Url -OutFile $DestinationPath
}

function Assert-ChecksumMatches {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ZipPath,
        [Parameter(Mandatory = $true)]
        [string]$ChecksumPath
    )

    $checksumText = (Get-Content -LiteralPath $ChecksumPath -Raw).Trim()
    $parts = $checksumText -split '\s+', 2
    if ($parts.Count -lt 2) {
        throw "Published checksum manifest '$ChecksumPath' is malformed."
    }

    $expectedHash = $parts[0].Trim().ToLowerInvariant()
    $expectedFileName = $parts[1].Trim()
    if ($expectedFileName -ne (Split-Path -Leaf $ZipPath)) {
        throw "Published checksum manifest references '$expectedFileName', expected '$(Split-Path -Leaf $ZipPath)'."
    }

    $actualHash = (Get-FileHash -LiteralPath $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "Published checksum mismatch for '$ZipPath'. Expected '$expectedHash', got '$actualHash'."
    }
}

$normalizedVersion = $Version.Trim()
if (-not $normalizedVersion.StartsWith("v")) {
    $normalizedVersion = "v$normalizedVersion"
}

$zipName = "lingopilot-tts-kokoro-$normalizedVersion-windows-x86_64.zip"
$checksumName = "lingopilot-tts-kokoro-$normalizedVersion-sha256.txt"
$zipUrl = Resolve-ReleaseDownloadUrl -Repository $Repository -VersionTag $normalizedVersion -AssetName $zipName
$checksumUrl = Resolve-ReleaseDownloadUrl -Repository $Repository -VersionTag $normalizedVersion -AssetName $checksumName

try {
    New-Item -ItemType Directory -Force -Path $DownloadRoot | Out-Null

    $zipPath = Join-Path $DownloadRoot $zipName
    $checksumPath = Join-Path $DownloadRoot $checksumName

    Invoke-ReleaseDownload -Url $zipUrl -DestinationPath $zipPath
    Invoke-ReleaseDownload -Url $checksumUrl -DestinationPath $checksumPath
    Assert-ChecksumMatches -ZipPath $zipPath -ChecksumPath $checksumPath

    & (Join-Path $PSScriptRoot "Test-WindowsReleaseArchive.ps1") -ZipPath $zipPath
}
finally {
    if (Test-Path -LiteralPath $DownloadRoot) {
        Remove-Item -LiteralPath $DownloadRoot -Recurse -Force
    }
}
