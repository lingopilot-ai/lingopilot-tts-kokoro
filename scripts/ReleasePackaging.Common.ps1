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

function ConvertFrom-ReleaseSourcesToml {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $result = @{}
    $currentSection = $null

    foreach ($rawLine in Get-Content -LiteralPath $Path) {
        $line = $rawLine.Trim()
        if ([string]::IsNullOrEmpty($line)) { continue }
        if ($line.StartsWith('#')) { continue }

        if ($line.StartsWith('[') -and $line.EndsWith(']')) {
            $currentSection = $line.Substring(1, $line.Length - 2).Trim()
            if (-not $result.ContainsKey($currentSection)) {
                $result[$currentSection] = @{}
            }
            continue
        }

        if ($null -eq $currentSection) {
            throw "release-sources.toml: key '$line' is not under any [section]."
        }

        $eqIndex = $line.IndexOf('=')
        if ($eqIndex -lt 1) {
            throw "release-sources.toml: unrecognized line '$rawLine'."
        }

        $key = $line.Substring(0, $eqIndex).Trim()
        $value = $line.Substring($eqIndex + 1).Trim()

        $hashIndex = -1
        $inString = $false
        for ($i = 0; $i -lt $value.Length; $i++) {
            $ch = $value[$i]
            if ($ch -eq '"') { $inString = -not $inString }
            elseif ($ch -eq '#' -and -not $inString) { $hashIndex = $i; break }
        }
        if ($hashIndex -ge 0) {
            $value = $value.Substring(0, $hashIndex).Trim()
        }

        if ($value.StartsWith('"') -and $value.EndsWith('"') -and $value.Length -ge 2) {
            $value = $value.Substring(1, $value.Length - 2)
        } else {
            throw "release-sources.toml: value for '$key' must be a double-quoted string."
        }

        $result[$currentSection][$key] = $value
    }

    return $result
}

function Get-ReleaseSourcesConfig {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    $path = Join-Path $RepoRoot "release-sources.toml"
    $empty = @{
        kokoro_model             = @{ url = $null; sha256 = $null }
        onnxruntime              = @{ url = $null; sha256 = $null }
        piper_windows            = @{ url = $null; sha256 = $null }
        onnxruntime_linux_x64    = @{ url = $null; sha256 = $null }
        onnxruntime_macos_arm64  = @{ url = $null; sha256 = $null }
    }

    if (-not (Test-Path -LiteralPath $path)) {
        return $empty
    }

    $parsed = ConvertFrom-ReleaseSourcesToml -Path $path
    foreach ($section in @('kokoro_model', 'onnxruntime', 'piper_windows', 'onnxruntime_linux_x64', 'onnxruntime_macos_arm64')) {
        if (-not $parsed.ContainsKey($section)) {
            $parsed[$section] = @{}
        }
        foreach ($field in @('url', 'sha256')) {
            $value = $parsed[$section][$field]
            if ([string]::IsNullOrWhiteSpace($value)) {
                $parsed[$section][$field] = $null
            } else {
                $parsed[$section][$field] = $value.Trim()
            }
        }
    }

    return $parsed
}

function Resolve-ReleaseAssetUrl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [string]$EnvOverride,
        [string]$LegacyOverride,
        [string]$ConfigValue
    )

    if (-not [string]::IsNullOrWhiteSpace($EnvOverride)) {
        return [pscustomobject]@{ Value = $EnvOverride.Trim(); Source = 'env' }
    }
    if (-not [string]::IsNullOrWhiteSpace($LegacyOverride)) {
        return [pscustomobject]@{ Value = $LegacyOverride.Trim(); Source = 'legacy' }
    }
    if (-not [string]::IsNullOrWhiteSpace($ConfigValue)) {
        return [pscustomobject]@{ Value = $ConfigValue.Trim(); Source = 'toml' }
    }
    return [pscustomobject]@{ Value = $null; Source = 'none' }
}

function Assert-FileSha256 {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [string]$Expected
    )

    if ([string]::IsNullOrWhiteSpace($Expected)) {
        return $null
    }

    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    $expectedLower = $Expected.Trim().ToLowerInvariant()
    if ($actual -ne $expectedLower) {
        $fileName = Split-Path -Leaf $Path
        throw "SHA-256 mismatch for '$fileName': expected $expectedLower, got $actual."
    }
    return $actual
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
