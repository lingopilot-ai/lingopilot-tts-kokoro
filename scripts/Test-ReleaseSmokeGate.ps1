param(
    [Parameter(Mandatory = $true)]
    [string]$ZipPath
)

$ErrorActionPreference = "Stop"

# Implements the "zero-arg + empty-stdin" readiness assertion from the host
# directive (kokoro.md §Smoke gate). Keep this script strictly aligned with
# that directive — any drift here will hide real conformance regressions.

$resolvedZipPath = (Resolve-Path $ZipPath).Path
$extractRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("lingopilot-tts-kokoro-smokegate-" + [System.Guid]::NewGuid().ToString("N"))
$expectedVersion = (Get-Content -LiteralPath (Join-Path $PSScriptRoot "..\Cargo.toml")) |
    Where-Object { $_ -match '^version\s*=\s*"(?<v>[^"]+)"' } |
    Select-Object -First 1 |
    ForEach-Object { ([regex]::Match($_, '^version\s*=\s*"(?<v>[^"]+)"').Groups['v'].Value) }

if ([string]::IsNullOrWhiteSpace($expectedVersion)) {
    throw "Could not determine expected version from Cargo.toml"
}

try {
    New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
    Expand-Archive -LiteralPath $resolvedZipPath -DestinationPath $extractRoot -Force

    $packageRoot = Join-Path $extractRoot ([System.IO.Path]::GetFileNameWithoutExtension($resolvedZipPath))
    if (-not (Test-Path $packageRoot)) {
        $directories = Get-ChildItem -LiteralPath $extractRoot -Directory
        if ($directories.Count -ne 1) {
            throw "Could not determine the extracted package root in $extractRoot."
        }
        $packageRoot = $directories[0].FullName
    }

    $binaryPath = Join-Path $packageRoot "lingopilot-tts-kokoro.exe"
    if (-not (Test-Path $binaryPath)) {
        throw "Smoke gate input is missing: $binaryPath"
    }

    # Feed a single empty line via stdin. No CLI args, no env vars.
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $binaryPath
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $psi
    [void]$process.Start()

    $process.StandardInput.WriteLine("")
    $process.StandardInput.Close()

    if (-not $process.WaitForExit(30000)) {
        $process.Kill()
        throw "Smoke gate failed: process did not exit within 30 seconds."
    }

    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()

    if ($process.ExitCode -ne 0) {
        throw "Smoke gate failed: exit code=$($process.ExitCode). stderr=`n$stderr"
    }

    if ($stderr.Contains("Missing required startup argument")) {
        throw "Smoke gate failed: stderr contained forbidden 'Missing required startup argument' string."
    }

    if ([string]::IsNullOrEmpty($stdout)) {
        throw "Smoke gate failed: stdout was empty."
    }

    $lines = $stdout -split "`r?`n"
    $nonEmptyLines = @($lines | Where-Object { $_.Length -gt 0 })
    if ($nonEmptyLines.Count -ne 1) {
        throw "Smoke gate failed: expected exactly one stdout line, got $($nonEmptyLines.Count). stdout=$stdout"
    }

    $readyLine = $nonEmptyLines[0]
    if ($readyLine -notmatch '^\{"op":"ready",') {
        throw "Smoke gate failed: ready line did not start with `"{`"op`":`"ready`",`". Got: $readyLine"
    }

    $ready = $readyLine | ConvertFrom-Json
    if ($ready.op -ne "ready") { throw "Smoke gate failed: op != ready" }
    if ([int]$ready.sample_rate -ne 24000) { throw "Smoke gate failed: sample_rate != 24000 (got $($ready.sample_rate))" }
    if ([int]$ready.channels -ne 1) { throw "Smoke gate failed: channels != 1 (got $($ready.channels))" }
    if ($ready.encoding -ne "pcm16le") { throw "Smoke gate failed: encoding != pcm16le (got $($ready.encoding))" }
    if ($ready.version -ne $expectedVersion) {
        throw "Smoke gate failed: version mismatch. Expected=$expectedVersion Got=$($ready.version)"
    }

    Write-Host "Smoke gate passed for $resolvedZipPath (version=$($ready.version))" -ForegroundColor Green
}
finally {
    if (Test-Path $extractRoot) {
        Remove-Item -LiteralPath $extractRoot -Recurse -Force
    }
}
