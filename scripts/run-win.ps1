$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $projectRoot 'target/windows/Viem.Windows/bin/x64/Release/net10.0-windows10.0.26100.0/win-x64/Viem.exe'
if (!(Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw 'No Release build found. Build it first with: .\scripts\build-win.ps1 -Configuration Release'
}
& $executable
