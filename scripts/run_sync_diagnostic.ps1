param(
    [ValidateRange(20, 7200)]
    [int]$DurationSeconds = 45,

    [ValidateRange(3, 300)]
    [int]$MarkerIntervalSeconds = 5,

    [int]$InputDeviceIndex = 1,
    [int]$OutputDeviceIndex = 0,
    [string]$OutputRoot = (Join-Path (Get-Location) "sync-diagnostics")
)

$ErrorActionPreference = "Stop"

function Require-Command([string]$Name) {
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if (-not $command) {
        throw "Required command not found: $Name"
    }
    return $command.Source
}

$audioRec = Require-Command "audio-rec.exe"
$ffmpeg = Require-Command "ffmpeg.exe"
$ffplay = Require-Command "ffplay.exe"
$python = Require-Command "python.exe"

$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$runDirectory = Join-Path $OutputRoot "sync-$timestamp"
New-Item -ItemType Directory -Force -Path $runDirectory | Out-Null

$recordingPath = Join-Path $runDirectory "recording.wav"
$reportPath = Join-Path $runDirectory "report.json"
$recorderStdout = Join-Path $runDirectory "recorder.stdout.log"
$recorderStderr = Join-Path $runDirectory "recorder.stderr.log"
$analyzer = Join-Path $PSScriptRoot "analyze_sync.py"

$playbackDuration = $DurationSeconds - 3
$source = "aevalsrc=if(gte(t\,3)*lt(mod(t-3\,$MarkerIntervalSeconds)\,0.4)\,0.95*sin(2*PI*1000*t)\,0)|0:s=48000:d=$playbackDuration"

$markerGate = "gte(t\,3)*if(lt(t\,60)\,lt(mod(t-3\,5)\,0.4)\,lt(mod(t-3\,$MarkerIntervalSeconds)\,0.4))"
$source = "aevalsrc=if($markerGate\,0.95*sin(2*PI*1000*t)\,0)|0:s=48000:d=$playbackDuration"

$recordArguments = @(
    "record",
    "--duration", $DurationSeconds,
    "--format", "wav",
    "--output", $recordingPath,
    "--input-device", $InputDeviceIndex,
    "--output-device", $OutputDeviceIndex,
    "--no-trim",
    "--no-log-file"
)

Write-Host "Starting recorder. Keep the notebook stationary and the room quiet."
$recorder = Start-Process -FilePath $audioRec `
    -ArgumentList $recordArguments `
    -RedirectStandardOutput $recorderStdout `
    -RedirectStandardError $recorderStderr `
    -WindowStyle Hidden `
    -PassThru

Start-Sleep -Seconds 2
if ($recorder.HasExited) {
    throw "Recorder exited before playback started. See $recorderStderr"
}

Write-Host "Playing audible markers for $playbackDuration seconds..."
& $ffplay -nodisp -autoexit -loglevel error -volume 100 -f lavfi -i $source
$playerExitCode = $LASTEXITCODE
$recorder.WaitForExit()
$recorder.Refresh()
$recorderExitCode = $recorder.ExitCode

if ($playerExitCode -ne 0) {
    throw "FFplay failed with exit code $playerExitCode."
}
if ($null -ne $recorderExitCode -and $recorderExitCode -ne 0) {
    throw "Recorder failed with exit code $recorderExitCode. See $recorderStderr"
}
if (-not (Test-Path -LiteralPath $recordingPath)) {
    throw "Recorder completed without creating $recordingPath"
}

$mergedPath = Join-Path $runDirectory "recording_merged.wav"
if (Test-Path -LiteralPath $mergedPath) {
    $officialHash = (Get-FileHash -LiteralPath $recordingPath -Algorithm SHA256).Hash
    $mergedHash = (Get-FileHash -LiteralPath $mergedPath -Algorithm SHA256).Hash
    if ($officialHash -eq $mergedHash) {
        Remove-Item -LiteralPath $mergedPath
    } else {
        Write-Warning "The diagnostic _merged file differs from the official output and was retained."
    }
}

$recorderEvents = Get-Content -LiteralPath $recorderStdout -Raw
if ($recorderEvents -notmatch '"mic_has_audio":true') {
    throw "Microphone did not detect the speaker markers. Unplug headphones, raise speaker volume, and disable echo cancellation before retrying."
}

Write-Host "Analyzing marker alignment..."
& $python $analyzer $recordingPath `
    --report $reportPath `
    --min-markers 4
$analysisExitCode = $LASTEXITCODE

Write-Host "Artifacts: $runDirectory"
exit $analysisExitCode
