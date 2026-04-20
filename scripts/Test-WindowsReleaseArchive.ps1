param(
    [Parameter(Mandatory = $true)]
    [string]$ZipPath
)

$ErrorActionPreference = "Stop"

function Read-ProtocolLine {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.Stream]$Stream
    )

    $buffer = New-Object System.Collections.Generic.List[byte]
    while ($true) {
        $nextByte = $Stream.ReadByte()
        if ($nextByte -lt 0) {
            if ($buffer.Count -eq 0) {
                return $null
            }
            throw "Unexpected EOF while reading a protocol JSON line."
        }

        if ($nextByte -eq 10) {
            break
        }

        if ($nextByte -ne 13) {
            [void]$buffer.Add([byte]$nextByte)
        }
    }

    return [System.Text.Encoding]::UTF8.GetString($buffer.ToArray())
}

function Read-ExactBytes {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.Stream]$Stream,
        [Parameter(Mandatory = $true)]
        [int]$ByteLength
    )

    $bytes = New-Object byte[] $ByteLength
    $offset = 0
    while ($offset -lt $ByteLength) {
        $read = $Stream.Read($bytes, $offset, $ByteLength - $offset)
        if ($read -le 0) {
            throw "Expected $ByteLength PCM bytes but stdout closed after $offset bytes."
        }
        $offset += $read
    }

    return $bytes
}

function Read-RemainingBytes {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.Stream]$Stream
    )

    $buffer = New-Object byte[] 4096
    $allBytes = New-Object System.Collections.Generic.List[byte]
    while ($true) {
        $read = $Stream.Read($buffer, 0, $buffer.Length)
        if ($read -le 0) {
            break
        }

        for ($index = 0; $index -lt $read; $index++) {
            [void]$allBytes.Add($buffer[$index])
        }
    }

    return $allBytes.ToArray()
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
        throw "Packaged model_dir '$ModelDir' is invalid: expected exactly one $Description."
    }

    $names = ($matches | ForEach-Object { "'$($_.Name)'" }) -join ", "
    throw "Packaged model_dir '$ModelDir' is invalid: expected exactly one $Description, found $names."
}

function Assert-StderrIsPlainText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Stderr
    )

    if ($Stderr.Contains([char]0)) {
        throw "Smoke test failed: stderr contained NUL bytes."
    }

    if ($Stderr.Contains('{"type"')) {
        throw "Smoke test failed: stderr contained protocol JSON."
    }
}

$resolvedZipPath = (Resolve-Path $ZipPath).Path
$extractRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("lingopilot-tts-kokoro-release-smoke-" + [System.Guid]::NewGuid().ToString("N"))

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
    $runtimeDir = Join-Path $packageRoot "espeak-runtime"
    $onnxRuntimeDll = Join-Path $packageRoot "onnxruntime.dll"
    $modelDir = Join-Path $packageRoot "kokoro-model"
    $espeakLibraryPath = Join-Path $runtimeDir "espeak-ng.dll"

    foreach ($requiredPath in @(
        $binaryPath,
        $runtimeDir,
        (Join-Path $runtimeDir "espeak-ng-data"),
        $espeakLibraryPath,
        $onnxRuntimeDll,
        $modelDir
    )) {
        if (-not (Test-Path $requiredPath)) {
            throw "Smoke test input is missing: $requiredPath"
        }
    }

    $null = Resolve-RequiredBundleFile -ModelDir $modelDir -Pattern "*.onnx" -Description "Kokoro model (*.onnx)"
    $null = Resolve-RequiredBundleFile -ModelDir $modelDir -Pattern "voices*.bin" -Description "Kokoro voices bundle (voices*.bin)"

    $defenderExclusionAdded = $false
    try {
        Add-MpPreference -ExclusionPath $packageRoot -ErrorAction Stop
        $defenderExclusionAdded = $true
        Write-Host "[smoke] registered Windows Defender exclusion for $packageRoot"
    } catch {
        Write-Host "[smoke] Add-MpPreference unavailable or failed; continuing without Defender exclusion ($($_.Exception.Message))"
    }

    Write-Host "[smoke] onnxruntime.dll probe starting at $(Get-Date -Format o)"
    try {
        $dllHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $onnxRuntimeDll).Hash
        $dllItem = Get-Item -LiteralPath $onnxRuntimeDll
        $dllVersion = $dllItem.VersionInfo.FileVersion
        $dllLength = $dllItem.Length
        $dllSig = (Get-AuthenticodeSignature -LiteralPath $onnxRuntimeDll)
        Write-Host "[smoke] dll metadata length=$dllLength version=$dllVersion sha256=$dllHash sig_status=$($dllSig.Status) sig_signer=$($dllSig.SignerCertificate.Subject)"
    } catch {
        Write-Host "[smoke] dll metadata probe failed: $($_.Exception.Message)"
    }

    try {
        Add-Type -Namespace LpTtsProbe -Name Kernel32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError=true, CharSet=System.Runtime.InteropServices.CharSet.Unicode)]
public static extern System.IntPtr LoadLibraryExW(string lpLibFileName, System.IntPtr hFile, uint dwFlags);
[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError=true)]
public static extern bool FreeLibrary(System.IntPtr hModule);
'@ -ErrorAction Stop
        $probeStart = Get-Date
        Write-Host "[smoke] LoadLibraryExW call starting at $(Get-Date -Format o)"
        $handle = [LpTtsProbe.Kernel32]::LoadLibraryExW($onnxRuntimeDll, [System.IntPtr]::Zero, 0)
        $probeEnd = Get-Date
        $elapsedMs = [int]($probeEnd - $probeStart).TotalMilliseconds
        if ($handle -eq [System.IntPtr]::Zero) {
            $err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
            Write-Host "[smoke] LoadLibraryExW FAILED elapsed_ms=$elapsedMs win32_error=$err"
        } else {
            Write-Host "[smoke] LoadLibraryExW OK elapsed_ms=$elapsedMs handle=$handle"
            [void][LpTtsProbe.Kernel32]::FreeLibrary($handle)
        }
    } catch {
        Write-Host "[smoke] LoadLibraryExW probe threw: $($_.Exception.Message)"
    }

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $binaryPath
    $startInfo.ArgumentList.Add("--espeak-data-dir")
    $startInfo.ArgumentList.Add($runtimeDir)
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $false
    $startInfo.Environment["KOKORO_TTS_LOG"] = "debug"

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo

    $null = $process.Start()
    $stdout = $process.StandardOutput.BaseStream

    Write-Host "[smoke] process started (pid=$($process.Id)), awaiting ready line..."
    $readyLine = Read-ProtocolLine -Stream $stdout
    Write-Host "[smoke] ready line received"
    if ([string]::IsNullOrWhiteSpace($readyLine)) {
        throw "Smoke test failed: the packaged binary did not emit a ready line."
    }

    $ready = $readyLine | ConvertFrom-Json
    if ($ready.type -ne "ready") {
        throw "Smoke test failed: expected a ready response, got '$readyLine'."
    }

    $request = @{
        text = "Hello from the packaged Kokoro sidecar"
        voice = "af_heart"
        speed = 1.0
        model_dir = $modelDir
    } | ConvertTo-Json -Compress
    Write-Host "[smoke] writing synthesis request to stdin..."
    $process.StandardInput.WriteLine($request)
    $process.StandardInput.Flush()
    Write-Host "[smoke] request flushed at $(Get-Date -Format o), awaiting audio response line..."

    $audioLine = Read-ProtocolLine -Stream $stdout
    Write-Host "[smoke] audio response line received at $(Get-Date -Format o)"
    if ([string]::IsNullOrWhiteSpace($audioLine)) {
        throw "Smoke test failed: the packaged binary did not emit an audio response."
    }

    $audio = $audioLine | ConvertFrom-Json
    if ($audio.type -ne "audio") {
        throw "Smoke test failed: expected an audio response, got '$audioLine'."
    }

    $byteLength = [int]$audio.byte_length
    if ($byteLength -le 0) {
        throw "Smoke test failed: byte_length must be greater than zero."
    }
    if (($byteLength % 2) -ne 0) {
        throw "Smoke test failed: byte_length must be even for PCM16 mono output."
    }
    if ([int]$audio.sample_rate -ne 24000) {
        throw "Smoke test failed: expected sample_rate=24000, got '$($audio.sample_rate)'."
    }
    if ([int]$audio.channels -ne 1) {
        throw "Smoke test failed: expected channels=1, got '$($audio.channels)'."
    }

    Write-Host "[smoke] reading $byteLength PCM bytes from stdout..."
    $audioBytes = Read-ExactBytes -Stream $stdout -ByteLength $byteLength
    if ($audioBytes.Length -ne $byteLength) {
        throw "Smoke test failed: expected $byteLength PCM bytes, got $($audioBytes.Length)."
    }

    Write-Host "[smoke] audio bytes read, closing stdin..."
    $process.StandardInput.Close()
    $process.WaitForExit()
    Write-Host "[smoke] process exited, code=$($process.ExitCode)"
    $remainingStdout = Read-RemainingBytes -Stream $stdout

    if ($process.ExitCode -ne 0) {
        throw "Smoke test failed: packaged binary exited with code $($process.ExitCode)."
    }

    if ($remainingStdout.Length -ne 0) {
        throw "Smoke test failed: stdout contained extra output after the audio bytes."
    }

    Write-Host "Smoke test passed for $resolvedZipPath" -ForegroundColor Green
}
finally {
    if ($defenderExclusionAdded) {
        try { Remove-MpPreference -ExclusionPath $packageRoot -ErrorAction SilentlyContinue } catch {}
    }
    if (Test-Path $extractRoot) {
        Remove-Item -LiteralPath $extractRoot -Recurse -Force
    }
}
