param(
    [Parameter(Mandatory = $true)]
    [datetime]$CopyAt,

    [Parameter(Mandatory = $true)]
    [string]$LoopbackSource,

    [Parameter(Mandatory = $true)]
    [string]$MicSource,

    [Parameter(Mandatory = $true)]
    [string]$DestinationDirectory
)

$ErrorActionPreference = "Stop"
$logPath = Join-Path $DestinationDirectory "snapshot.log"

try {
    $delay = $CopyAt - (Get-Date)
    if ($delay.TotalSeconds -gt 0) {
        Start-Sleep -Seconds ([math]::Ceiling($delay.TotalSeconds))
    }

    Copy-Item -LiteralPath $LoopbackSource `
        -Destination (Join-Path $DestinationDirectory "loopback-raw.wav") `
        -Force
    Copy-Item -LiteralPath $MicSource `
        -Destination (Join-Path $DestinationDirectory "mic-raw.wav") `
        -Force

    $loopbackLength = (Get-Item -LiteralPath (Join-Path $DestinationDirectory "loopback-raw.wav")).Length
    $micLength = (Get-Item -LiteralPath (Join-Path $DestinationDirectory "mic-raw.wav")).Length
    "$(Get-Date -Format o) snapshot complete loopback_bytes=$loopbackLength mic_bytes=$micLength" |
        Set-Content -LiteralPath $logPath
}
catch {
    "$(Get-Date -Format o) snapshot failed: $_" | Set-Content -LiteralPath $logPath
    exit 1
}
