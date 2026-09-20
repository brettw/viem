param([switch]$NoBuild, [ValidateRange(1, 20)][int]$Runs = 3, [string]$Label = 'startup', [string]$ProfileDocument, [string]$ConfigFile)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    if (!$NoBuild) { & "$PSScriptRoot/build-win.ps1" -Configuration Release -Offline }
    $testRoot = Join-Path $projectRoot 'target/windows-validation'
    New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
    $oldProfile = $env:VIEM_CONFIG_DIR
    $oldReport = $env:VIEM_STARTUP_REPORT
    $oldExit = $env:VIEM_STARTUP_EXIT
    $oldDocument = $env:VIEM_STARTUP_DOCUMENT
    if ($ProfileDocument) { $env:VIEM_STARTUP_DOCUMENT = (Resolve-Path -LiteralPath $ProfileDocument).Path }
    try {
        for ($run = 1; $run -le $Runs; $run++) {
            $reportPath = Join-Path $testRoot ($Label + '-' + [Guid]::NewGuid().ToString('N') + '.json')
            $env:VIEM_CONFIG_DIR = $reportPath + '.profile'
            if ($ConfigFile) { New-Item -ItemType Directory -Path $env:VIEM_CONFIG_DIR -Force | Out-Null; Copy-Item -LiteralPath $ConfigFile -Destination (Join-Path $env:VIEM_CONFIG_DIR 'config.json') }
            $env:VIEM_STARTUP_REPORT = $reportPath
            $env:VIEM_STARTUP_EXIT = '1'
            $executable = Join-Path $projectRoot 'target/windows/Viem.Windows/bin/x64/Release/net10.0-windows10.0.26100.0/win-x64/Viem.exe'
            $launch = @{ FilePath = $executable; PassThru = $true; WindowStyle = 'Hidden' }
            if ($ProfileDocument) { $launch.ArgumentList = '"' + $env:VIEM_STARTUP_DOCUMENT + '"' }
            $process = Start-Process @launch
            if (!$process.WaitForExit(30000)) { $process.Kill(); throw 'Startup measurement timed out.' }
            if (!(Test-Path -LiteralPath $reportPath)) { throw "App exited $($process.ExitCode) without a startup report." }
            $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
            $first = $report.events | Where-Object name -eq 'editor.firstDraw'
            $activation = $report.events | Where-Object name -eq 'window.activate'
            if (!$first -or !$activation) { throw 'Startup trace is missing activation or the first editor draw.' }
            if ($ProfileDocument) {
                $initial = $report.frames[0].geometry
                foreach ($frame in $report.frames) {
                    foreach ($field in @('width', 'height', 'top', 'firstBaseline', 'canvasY')) {
                        if ([Math]::Abs($frame.geometry.$field - $initial.$field) -gt 0.01) { throw "Initial document geometry shifted: $field. Report: $reportPath" }
                    }
                }
            }
            if (!$ProfileDocument -and ($report.fontPickerListLoaded -ne $false -or $report.fontFaceDescriptionsRead -ne 0)) {
                throw 'Empty-editor startup loaded font picker data or enumerated font faces.'
            }
            [pscustomobject]@{ Run = $run; ProcessMs = [Math]::Round($report.processMilliseconds, 1); FirstDrawMs = [Math]::Round($first.milliseconds, 1); AfterActivationMs = [Math]::Round($first.milliseconds - $activation.milliseconds, 1); Report = $reportPath }
        }
    }
    finally { $env:VIEM_CONFIG_DIR = $oldProfile; $env:VIEM_STARTUP_REPORT = $oldReport; $env:VIEM_STARTUP_EXIT = $oldExit; $env:VIEM_STARTUP_DOCUMENT = $oldDocument }
}
finally { Pop-Location }
