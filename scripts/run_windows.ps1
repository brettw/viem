param([switch]$Offline)

$ErrorActionPreference = 'Stop'
& "$PSScriptRoot/build-win.ps1" -Configuration Release -Run -Offline:$Offline
