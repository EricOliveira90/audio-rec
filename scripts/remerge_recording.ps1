<#
.SYNOPSIS
  Merges the kept raw tracks of a failed recording into one stereo file.

.DESCRIPTION
  Use this when a recording fails during post-processing (for example "Disk full" or
  "Not enough disk space"). The recorder keeps <session>_loopback.wav and <session>_mic.wav
  in storage\recordings. This script runs the same FFmpeg merge the recorder runs.

  Take -OffsetMs and -MicGainDb from the failed run's log:
    "Start offset: 1786 ms (loopback started first, delaying loopback)" -> -OffsetMs 1786
    "Mic gain compensation: 6.4 dB"                                     -> -MicGainDb 6.4

  The script checks free space first. It does not delete the raw tracks.

.EXAMPLE
  .\scripts\remerge_recording.ps1 -SessionId rec-20261112_140558 -OffsetMs 1786 -MicGainDb 6.4
#>
param(
    [Parameter(Mandatory = $true)][string]$SessionId,
    [int]$OffsetMs = 0,
    [double]$MicGainDb = 0,
    [string]$RecordingsDir = '',
    [string]$OutputPath = ''
)

$ErrorActionPreference = 'Stop'
# PowerShell 5.1 does not set $PSScriptRoot inside param() defaults, so resolve here.
if (-not $RecordingsDir) { $RecordingsDir = Join-Path $PSScriptRoot '..\storage\recordings' }
if (-not $OutputPath) { $OutputPath = Join-Path (Get-Location) "$SessionId.wav" }
$loopback = Join-Path $RecordingsDir "${SessionId}_loopback.wav"
$mic = Join-Path $RecordingsDir "${SessionId}_mic.wav"
foreach ($track in $loopback, $mic) {
    if (-not (Test-Path $track)) { throw "Raw track not found: $track" }
}

# Output length = longest track + offset. WAV output is 48 kHz 16-bit stereo = 192000 bytes/s.
$durations = foreach ($track in $loopback, $mic) {
    [double](& ffprobe -v error -show_entries format=duration -of csv=p=0 $track)
}
$outputSecs = ($durations | Measure-Object -Maximum).Maximum + [math]::Abs($OffsetMs) / 1000.0
$neededBytes = [int64]($outputSecs * 192000) + 128MB
$drive = (Get-Item $OutputPath -ErrorAction SilentlyContinue).PSDrive
if (-not $drive) { $drive = (Get-Item (Split-Path -Parent ([IO.Path]::GetFullPath($OutputPath)))).PSDrive }
if ($drive.Free -lt $neededBytes) {
    throw ("Not enough disk space: {0} MB free on {1}:, about {2} MB needed." -f `
        [math]::Floor($drive.Free / 1MB), $drive.Name, [math]::Ceiling($neededBytes / 1MB))
}

$whole = '{0:F3}' -f $outputSecs
$gain = if ($MicGainDb -ne 0) { "volume=$($MicGainDb)dB," } else { '' }
$filter = "[0:a]aformat=channel_layouts=stereo,apad=whole_dur=$whole[left];" +
          "[1:a]${gain}aformat=channel_layouts=mono,apad=whole_dur=$whole,asplit=2[ml][mr];" +
          "[left][ml][mr]amerge=inputs=3,pan=stereo|c0<c0+c2|c1<c1+c2[aout]"

$ffmpegArgs = @('-hide_banner', '-loglevel', 'error')
if ($OffsetMs -gt 0) { $ffmpegArgs += @('-itsoffset', ('{0:F3}' -f ($OffsetMs / 1000.0))) }
$ffmpegArgs += @('-i', $loopback)
if ($OffsetMs -lt 0) { $ffmpegArgs += @('-itsoffset', ('{0:F3}' -f (-$OffsetMs / 1000.0))) }
$ffmpegArgs += @('-i', $mic, '-filter_complex', $filter, '-map', '[aout]', '-ar', '48000', '-y', $OutputPath)

& ffmpeg @ffmpegArgs
if ($LASTEXITCODE -ne 0) {
    if (Test-Path $OutputPath) { Remove-Item $OutputPath -Confirm:$false }
    throw "FFmpeg failed with exit code $LASTEXITCODE (-28 means the disk is full)."
}
Write-Host "Saved $OutputPath. The raw tracks are still in $RecordingsDir; delete them when you have checked the file."
