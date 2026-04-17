<#
.SYNOPSIS
    Synthesizes the same Portuguese sentence with every available pt-br voice
    and plays each through speakers so voices can be compared A/B.

.EXAMPLE
    $env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR = (Resolve-Path 'dist/lingopilot-tts-kokoro-v0.1.0-windows-x86_64/espeak-runtime').Path
    $env:KOKORO_TTS_LIVE_MODEL_DIR           = (Resolve-Path 'dist/lingopilot-tts-kokoro-v0.1.0-windows-x86_64/kokoro-model').Path
    $env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL     = (Resolve-Path 'dist/lingopilot-tts-kokoro-v0.1.0-windows-x86_64/onnxruntime.dll').Path
    ./scripts/Play-PortugueseVoices.ps1 -Text 'Olá, bom dia! Como você está hoje?'
#>

param(
    [string] $Text = 'A rápida raposa marrom pula sobre o cachorro preguiçoso.',
    [string[]] $Voices = @('pf_dora', 'pm_alex', 'pm_santa')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$espeakDir = $env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR
$modelDir  = $env:KOKORO_TTS_LIVE_MODEL_DIR
$ortDll    = $env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL

if (-not $espeakDir -or -not $modelDir -or -not $ortDll) {
    throw "Set KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR, KOKORO_TTS_LIVE_MODEL_DIR, KOKORO_TTS_LIVE_ONNXRUNTIME_DLL."
}

$binary = Join-Path $PSScriptRoot '..\target\release\lingopilot-tts-kokoro.exe'
if (-not (Test-Path $binary)) {
    Write-Host "Release binary not found, building..."
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}
$binary = (Resolve-Path $binary).Path

$outDir = Join-Path ([System.IO.Path]::GetTempPath()) 'lingopilot-tts-pt-br'
if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
New-Item -ItemType Directory -Path $outDir | Out-Null

Write-Host "Starting sidecar: $binary"
Write-Host "  text   = `"$Text`""
Write-Host "  voices = $($Voices -join ', ')"

$psi = [System.Diagnostics.ProcessStartInfo]::new()
$psi.FileName = $binary
$psi.Arguments = "--espeak-data-dir `"$espeakDir`""
$psi.Environment['ORT_DYLIB_PATH'] = $ortDll
$psi.Environment['KOKORO_TTS_LOG'] = 'info'
$psi.RedirectStandardInput  = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError  = $true
$psi.UseShellExecute = $false

$proc = [System.Diagnostics.Process]::Start($psi)
if ($null -eq $proc) { throw "Failed to start sidecar process" }
$proc.BeginErrorReadLine()

$rawStdout  = $proc.StandardOutput.BaseStream
$rawStdin   = $proc.StandardInput.BaseStream

function Write-JsonLine([System.IO.Stream] $s, [string] $json) {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json + "`n")
    $s.Write($bytes, 0, $bytes.Length)
    $s.Flush()
}

function Read-Line([System.IO.Stream] $s) {
    $buf = [System.Collections.Generic.List[byte]]::new(4096)
    while ($true) {
        $b = $s.ReadByte()
        if ($b -eq -1) { throw "Unexpected EOF from sidecar stdout" }
        if ($b -eq 10) { break }
        if ($b -eq 13) { continue }
        $buf.Add([byte]$b)
    }
    return [System.Text.Encoding]::UTF8.GetString($buf.ToArray())
}

function Read-Bytes([System.IO.Stream] $s, [int] $count) {
    $bytes = [byte[]]::new($count)
    $off = 0
    while ($off -lt $count) {
        $n = $s.Read($bytes, $off, $count - $off)
        if ($n -le 0) { throw "Unexpected EOF reading $count bytes (got $off)" }
        $off += $n
    }
    return ,$bytes
}

function Write-Wav([string] $path, [byte[]] $pcm) {
    $sr = [uint32]24000; $ch = [uint16]1; $bps = [uint16]16
    $dataLen = [uint32]$pcm.Length
    $fs = [System.IO.FileStream]::new($path, [System.IO.FileMode]::Create)
    $w  = [System.IO.BinaryWriter]::new($fs)
    $w.Write([byte[]]@(0x52,0x49,0x46,0x46))
    $w.Write([uint32](36 + $dataLen))
    $w.Write([byte[]]@(0x57,0x41,0x56,0x45))
    $w.Write([byte[]]@(0x66,0x6D,0x74,0x20))
    $w.Write([uint32]16)
    $w.Write([uint16]1)
    $w.Write($ch); $w.Write($sr)
    $w.Write([uint32]($sr * $ch * ($bps / 8)))
    $w.Write([uint16]($ch * ($bps / 8)))
    $w.Write($bps)
    $w.Write([byte[]]@(0x64,0x61,0x74,0x61))
    $w.Write($dataLen); $w.Write($pcm)
    $w.Close(); $fs.Close()
}

Write-Host "Waiting for sidecar ready..."
$readyLine = Read-Line $rawStdout
$ready = $readyLine | ConvertFrom-Json
if ($ready.type -ne 'ready') { throw "Expected ready, got: $readyLine" }
Write-Host "Sidecar ready (v$($ready.version)).`n"

$player = New-Object System.Media.SoundPlayer

foreach ($voice in $Voices) {
    Write-Host ("[pt-br] {0}: `"{1}`"" -f $voice, $Text)

    $req = @{ text = $Text; voice = $voice; speed = 1.0; model_dir = $modelDir } |
        ConvertTo-Json -Compress

    Write-JsonLine $rawStdin $req

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $respLine = Read-Line $rawStdout
    $resp = $respLine | ConvertFrom-Json

    if ($resp.type -eq 'error') {
        Write-Warning "  ERROR: $($resp.message)"
        continue
    }
    if ($resp.type -ne 'audio') {
        Write-Warning "  Unexpected response type: $($resp.type)"
        continue
    }

    $byteLength = [int]$resp.byte_length
    $pcm = Read-Bytes $rawStdout $byteLength
    $sw.Stop()
    Write-Host ("  Synthesized in {0:N1}s, {1:N0} bytes." -f $sw.Elapsed.TotalSeconds, $byteLength)

    $wavPath = Join-Path $outDir ("{0}.wav" -f $voice)
    Write-Wav $wavPath $pcm

    Write-Host "  Playing $wavPath ..."
    $player.SoundLocation = $wavPath
    $player.PlaySync()
    Write-Host "  Done.`n"
}

$rawStdin.Close()
$proc.WaitForExit(5000) | Out-Null
if (-not $proc.HasExited) { $proc.Kill() }

Write-Host "`nAll $($Voices.Count) pt-br voices played."
Write-Host "WAV files saved in: $outDir"
