param(
    [ValidateSet('Debug', 'Release')][string]$Configuration = 'Release',
    [switch]$Run,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    $cargoArguments = @('build', '--locked')
    if ($Configuration -eq 'Release') { $cargoArguments += '--release' }
    if ($Offline) { $cargoArguments += '--offline' }
    & cargo @cargoArguments
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
    $dotnetArguments = @('build', 'src/win/Viem.Windows.csproj', '-c', $Configuration, '--nologo', '-p:RestoreLockedMode=true')
    if ($Offline) { $dotnetArguments += '--no-restore' }
    & dotnet @dotnetArguments
    if ($LASTEXITCODE -ne 0) { throw 'Windows frontend build failed.' }
    $executable = Join-Path $projectRoot "target/windows/Viem.Windows/bin/x64/$Configuration/net10.0-windows10.0.26100.0/win-x64/Viem.exe"
    Write-Output $executable
    if ($Run) { & $executable }
}
finally { Pop-Location }
