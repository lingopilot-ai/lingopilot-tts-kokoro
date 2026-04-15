function Get-PackageVersion {
    param([string]$CargoTomlPath)

    $versionMatch = Select-String -Path $CargoTomlPath -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $versionMatch) {
        throw "Could not determine the package version from Cargo.toml."
    }

    return $versionMatch.Matches[0].Groups[1].Value
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
        throw "$Name must point to an existing $Kind path."
    }

    if (-not (Test-Path -LiteralPath $Value)) {
        throw "$Name is missing: $Value"
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

function Resolve-RequiredBundleFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ModelDir,
        [Parameter(Mandatory = $true)]
        [string]$Pattern,
        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    $matches = @(
        Get-ChildItem -LiteralPath $ModelDir -File |
            Where-Object { $_.Name -like $Pattern } |
            Sort-Object -Property Name
    )

    if ($matches.Count -eq 1) {
        return $matches[0].FullName
    }

    if ($matches.Count -eq 0) {
        throw "Cannot use model_dir '$ModelDir': expected exactly one $Description"
    }

    $names = ($matches | ForEach-Object { "'$($_.Name)'" }) -join ", "
    throw "Cannot use model_dir '$ModelDir': expected exactly one $Description, found $names"
}

function Update-ChecksumManifest {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ChecksumPath,
        [Parameter(Mandatory = $true)]
        [string]$AssetPath
    )

    $assetName = Split-Path -Leaf $AssetPath
    $hash = (Get-FileHash -LiteralPath $AssetPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $checksumLine = "{0}  {1}" -f $hash, $assetName
    $lines = @()

    if (Test-Path -LiteralPath $ChecksumPath) {
        $lines = @(
            Get-Content -LiteralPath $ChecksumPath |
                Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
                Where-Object {
                    $parts = $_ -split '\s+', 2
                    $parts.Count -lt 2 -or $parts[1].Trim() -ne $assetName
                }
        )
    }

    $lines += $checksumLine
    Set-Content -LiteralPath $ChecksumPath -Value $lines
}
