param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    & python src/win/tools/generate_bindings.py --check
    if ($LASTEXITCODE -ne 0) { throw 'Generated ABI declarations differ from the C header.' }
    if (!$NoBuild) { & "$PSScriptRoot/build-win.ps1" -Configuration Debug }
    $testRoot = Join-Path $projectRoot 'target/windows-validation'
    New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
    $reportPath = Join-Path $testRoot ('result-' + [Guid]::NewGuid().ToString('N') + '.json')
    $oldProfile = $env:VIEM_CONFIG_DIR
    $env:VIEM_CONFIG_DIR = $reportPath + '.profile'
    try {
        $executable = Join-Path $projectRoot 'target/windows/Viem.Windows/bin/x64/Debug/net10.0-windows10.0.26100.0/win-x64/Viem.exe'
        $process = Start-Process -FilePath $executable -ArgumentList @('--self-test', ('"' + $reportPath + '"')) -PassThru -WindowStyle Hidden
        if (!$process.WaitForExit(120000)) { $process.Kill(); throw 'Windows integration tests timed out.' }
        if (!(Test-Path -LiteralPath $reportPath)) { throw "Windows app exited $($process.ExitCode) before writing a test report." }
        $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
        if (!$report.passed) { throw $report.error }
        Write-Output "$($report.count) Windows integration checks passed. Report: $reportPath"
    }
    finally { $env:VIEM_CONFIG_DIR = $oldProfile }
}
finally { Pop-Location }
