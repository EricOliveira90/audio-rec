<#
.SYNOPSIS
  Loads the MSVC build environment, then runs a command (for example cargo).

.DESCRIPTION
  Rust on Windows (x86_64-pc-windows-msvc) needs the MSVC linker, link.exe.
  This script finds Visual Studio or the VS Build Tools with vswhere.exe, loads
  the x64 developer environment into the current PowerShell process, checks that
  link.exe is the MSVC linker, then runs your command.

  If you run it without a command, it only loads the environment. The environment
  stays loaded in this PowerShell window, so later cargo commands work too.

  If the C++ build tools are not installed, the script stops and prints the
  install command. The install needs about 7 GB of free disk space.

.EXAMPLE
  .\scripts\with-msvc.ps1 cargo test -p audio-rec-core --lib

.EXAMPLE
  .\scripts\with-msvc.ps1
  cargo build --release
#>

$ErrorActionPreference = 'Stop'

$installHint = @'
The MSVC C++ build tools are not installed. Install them, then run this script again:

  winget install Microsoft.VisualStudio.2022.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

The install needs about 7 GB of free disk space on C:.
'@

function Test-MsvcLinker {
    $link = Get-Command link.exe -ErrorAction SilentlyContinue
    return ($link -and $link.Source -match '\\VC\\Tools\\MSVC\\')
}

if (-not (Test-MsvcLinker)) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { throw $installHint }

    $vsPath = & $vswhere -latest -products * `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath
    if (-not $vsPath) { throw $installHint }

    $devShell = Join-Path $vsPath 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll'
    if (-not (Test-Path $devShell)) {
        throw "Found Visual Studio at $vsPath, but not its DevShell module ($devShell). Repair the install."
    }

    Import-Module $devShell
    Enter-VsDevShell -VsInstallPath $vsPath -SkipAutomaticLocation `
        -DevCmdArguments '-arch=x64 -host_arch=x64' | Out-Null

    if (-not (Test-MsvcLinker)) {
        $found = (Get-Command link.exe -ErrorAction SilentlyContinue).Source
        throw "Loaded the developer environment, but link.exe is still not the MSVC linker (found: $found)."
    }
}

Write-Host "MSVC linker: $((Get-Command link.exe).Source)"

if ($args.Count -gt 0) {
    $command = $args[0]
    $rest = @($args | Select-Object -Skip 1)
    & $command @rest
    exit $LASTEXITCODE
}
