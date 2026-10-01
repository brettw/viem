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
    $executableDirectory = Join-Path $projectRoot "target/windows/Viem.Windows/bin/x64/$Configuration/net10.0-windows10.0.26100.0/win-x64"
    $dotnetArguments = @('build', 'src/win/Viem.Windows.csproj', '-c', $Configuration, '--nologo', '-p:RestoreLockedMode=true')
    if ($Configuration -eq 'Release') {
        # ReadyToRun is a publish step. Keep the existing executable location so
        # launch scripts and user shortcuts run the precompiled Release build.
        $dotnetArguments[0] = 'publish'
        $dotnetArguments += @('--output', $executableDirectory)
    }
    if ($Offline) { $dotnetArguments += '--no-restore' }
    & dotnet @dotnetArguments
    if ($LASTEXITCODE -ne 0) { throw 'Windows frontend build failed.' }
    $executable = Join-Path $executableDirectory 'Viem.exe'
    Write-Output $executable
}
finally { Pop-Location }
# Launch after restoring the caller's directory rather than the build directory.
if ($Run) { & $executable }
