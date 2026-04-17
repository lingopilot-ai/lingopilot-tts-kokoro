<#
.SYNOPSIS
    Synthesizes one phrase per enabled language and plays each through speakers.

.EXAMPLE
    $env:KOKORO_TTS_LIVE_ESPEAK_RUNTIME_DIR = (Resolve-Path 'target/release/espeak-runtime').Path
    $env:KOKORO_TTS_LIVE_MODEL_DIR = (Resolve-Path 'packaging/windows/kokoro-model').Path
    $env:KOKORO_TTS_LIVE_ONNXRUNTIME_DLL = (Resolve-Path 'packaging/windows/onnxruntime.dll').Path
    ./scripts/Play-AllLanguages.ps1
#>

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

$phrases = @(
    @{ voice = 'af_heart'; lang = 'en-us'; text = 'The quick brown fox jumps over the lazy dog.' }
    @{ voice = 'bf_emma';  lang = 'en-gb'; text = 'The quick brown fox jumps over the lazy dog.' }
    @{ voice = 'ef_dora';  lang = 'es';    text = 'El veloz zorro marron salta sobre el perro perezoso.' }
    @{ voice = 'ff_siwis'; lang = 'fr';    text = 'Le renard brun rapide saute par-dessus le chien paresseux.' }
    @{ voice = 'hf_alpha'; lang = 'hi';    text = 'Namaste, aaj ka din bahut accha hai.' }
    @{ voice = 'if_sara';  lang = 'it';    text = 'La volpe marrone veloce salta sopra il cane pigro.' }
    @{ voice = 'pf_dora';  lang = 'pt-br'; text = 'A rapida raposa marrom pula sobre o cachorro preguicoso.' }
    @{ voice = 'pm_santa'; lang = 'pt-br'; text = 'Ola, eu sou o Papai Noel e desejo a todos um Feliz Natal.' }
)

$outDir = Join-Path ([System.IO.Path]::GetTempPath()) 'lingopilot-tts-play-all'
if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
New-Item -ItemType Directory -Path $outDir | Out-Null

# ---- launch sidecar with raw byte streams (no StreamReader wrapping) ----
Write-Host "Starting sidecar: $binary"
Write-Host "  --espeak-data-dir $espeakDir"
Write-Host "  ORT_DYLIB_PATH=$ortDll"

$psi = [System.Diagnostics.ProcessStartInfo]::new()
$psi.FileName = $binary
$psi.Arguments = "--espeak-data-dir `"$espeakDir`""
$psi.Environment['ORT_DYLIB_PATH'] = $ortDll
$psi.Environment['KOKORO_TTS_LOG'] = 'info'
$psi.RedirectStandardInput  = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError  = $true
$psi.UseShellExecute = $false
# Critical: do not let .NET wrap stdout in a StreamReader with encoding detection.
# We need raw bytes. StandardOutputEncoding = null would be ideal but .NET still
# wraps it. Instead we will grab the BaseStream immediately before any ReadLine.

$proc = [System.Diagnostics.Process]::Start($psi)
if ($null -eq $proc) { throw "Failed to start sidecar process" }

# Drain stderr asynchronously so it doesn't block the process
$proc.BeginErrorReadLine()

# Get the raw stdout stream BEFORE anyone calls ReadLine on StandardOutput.
# .NET's Process.StandardOutput is a StreamReader; accessing .BaseStream gives
# the underlying pipe. Since we haven't called ReadLine yet, the internal buffer
# is empty, so no data is lost.
$rawStdout = $proc.StandardOutput.BaseStream
$stdinWriter = $proc.StandardInput

# ---- helper functions operating on raw byte stream ----
function Read-Line([System.IO.Stream] $s) {
    $buf = [System.Collections.Generic.List[byte]]::new(4096)
    while ($true) {
        $b = $s.ReadByte()
        if ($b -eq -1) { throw "Unexpected EOF from sidecar stdout" }
        if ($b -eq 10) { break }   # LF
        if ($b -eq 13) { continue } # CR
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
    $w.Write([byte[]]@(0x52,0x49,0x46,0x46)) # RIFF
    $w.Write([uint32](36 + $dataLen))
    $w.Write([byte[]]@(0x57,0x41,0x56,0x45)) # WAVE
    $w.Write([byte[]]@(0x66,0x6D,0x74,0x20)) # fmt
    $w.Write([uint32]16)
    $w.Write([uint16]1)   # PCM
    $w.Write($ch)
    $w.Write($sr)
    $w.Write([uint32]($sr * $ch * ($bps / 8))) # byte rate
    $w.Write([uint16]($ch * ($bps / 8)))       # block align
    $w.Write($bps)
    $w.Write([byte[]]@(0x64,0x61,0x74,0x61)) # data
    $w.Write($dataLen)
    $w.Write($pcm)
    $w.Close(); $fs.Close()
}

# ---- wait for ready ----
Write-Host "Waiting for sidecar ready..."
$readyLine = Read-Line $rawStdout
Write-Host "  <- $readyLine"
$ready = $readyLine | ConvertFrom-Json
if ($ready.type -ne 'ready') {
    throw "Expected ready, got: $readyLine"
}
Write-Host "Sidecar ready (v$($ready.version)).`n"

$player = New-Object System.Media.SoundPlayer

foreach ($p in $phrases) {
    $voice = $p.voice
    $lang  = $p.lang
    $text  = $p.text

    Write-Host ("[{0}] {1}: `"{2}`"" -f $lang, $voice, $text)

    $req = @{ text = $text; voice = $voice; speed = 1.0; model_dir = $modelDir } |
        ConvertTo-Json -Compress

    $stdinWriter.WriteLine($req)
    $stdinWriter.Flush()

    Write-Host "  Sent request, waiting for response..."
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
    Write-Host ("  Audio header received: {0:N0} bytes, {1} Hz. Reading PCM..." -f $byteLength, $resp.sample_rate)

    $pcm = Read-Bytes $rawStdout $byteLength
    $sw.Stop()
    Write-Host ("  Synthesized in {0:N1}s. Writing WAV..." -f $sw.Elapsed.TotalSeconds)

    $wavPath = Join-Path $outDir ("{0}_{1}.wav" -f $voice, $lang)
    Write-Wav $wavPath $pcm

    Write-Host "  Playing $wavPath ..."
    $player.SoundLocation = $wavPath
    $player.PlaySync()
    Write-Host "  Done.`n"
}

# shutdown
$stdinWriter.Close()
$proc.WaitForExit(5000) | Out-Null
if (-not $proc.HasExited) { $proc.Kill() }

Write-Host "`nAll $($phrases.Count) languages played."
Write-Host "WAV files saved in: $outDir"
