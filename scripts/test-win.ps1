param([switch]$NoBuild, [switch]$PointerInput, [switch]$Optimized, [switch]$NativeAot, [string]$ProfileDocument, [ValidateSet('drag', 'resize', 'page', 'scroll', 'roundtrip')][string]$ProfileScenario = 'drag', [switch]$DisablePrelayout, [ValidateRange(1, 1000)][int]$ProfileIntervalMs = 16, [string]$ConfigFile, [string]$Executable)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$optimizedOutput = Join-Path $projectRoot $(if ($NativeAot) { 'target/windows-native-tests' } else { 'target/windows-profile' })
Push-Location $projectRoot
try {
    & python src/win/tools/generate_bindings.py --check
    if ($LASTEXITCODE -ne 0) { throw 'Generated ABI declarations differ from the C header.' }
    if (!$NoBuild) {
        if ($Optimized -or $NativeAot) {
            & cargo build --release --locked
            if ($LASTEXITCODE -ne 0) { throw 'Optimized Rust build failed.' }
            # Keep diagnostics enabled, with the same Rust and C# optimization
            # as shipping builds, in a separate output from the running editor.
            $testBuild = @('build', 'src/win/Viem.Windows.csproj', '-c', 'Debug', '--nologo', '-p:RestoreLockedMode=true', '-p:Optimize=true', '-p:RustProfile=release')
            if ($NativeAot) {
                $testBuild[0] = 'publish'
                $testBuild += @('-p:PublishAot=true', '--output', $optimizedOutput,
                    "-p:BaseIntermediateOutputPath=$projectRoot/target/windows-native-tests-obj/",
                    "-p:BaseOutputPath=$projectRoot/target/windows-native-tests-build/")
            } else { $testBuild += "-p:OutDir=$optimizedOutput/" }
            & dotnet @testBuild
            if ($LASTEXITCODE -ne 0) { throw 'Optimized Windows test build failed.' }
            & dotnet publish src/win/Blocking/BlockingViem.csproj -c Debug --nologo --output $optimizedOutput
            if ($LASTEXITCODE -ne 0) { throw 'Blocking launcher test build failed.' }
        } else { & "$PSScriptRoot/build-win.ps1" -Configuration Debug }
    }
    $testRoot = Join-Path $projectRoot 'target/windows-validation'
    New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
    $reportPath = Join-Path $testRoot ('result-' + [Guid]::NewGuid().ToString('N') + '.json')
    $oldProfile = $env:VIEM_CONFIG_DIR
    $oldPerformanceDocument = $env:VIEM_PERF_DOCUMENT
    $oldPerformanceScenario = $env:VIEM_PERF_SCENARIO
    $oldPointerInput = $env:VIEM_TEST_POINTER_INPUT
    $oldPrelayout = $env:VIEM_TEST_DISABLE_PRELAYOUT
    $oldInterval = $env:VIEM_PERF_INTERVAL_MS
    $env:VIEM_TEST_DISABLE_PRELAYOUT = if ($DisablePrelayout) { '1' } else { $null }
    $env:VIEM_PERF_INTERVAL_MS = "$ProfileIntervalMs"
    $env:VIEM_TEST_POINTER_INPUT = if ($PointerInput) { '1' } else { $null }
    $env:VIEM_PERF_SCENARIO = $ProfileScenario
    if ($ProfileDocument) { $env:VIEM_PERF_DOCUMENT = (Resolve-Path -LiteralPath $ProfileDocument).Path }
    $env:VIEM_CONFIG_DIR = $reportPath + '.profile'
    if ($ConfigFile) { New-Item -ItemType Directory -Path $env:VIEM_CONFIG_DIR -Force | Out-Null; Copy-Item -LiteralPath $ConfigFile -Destination (Join-Path $env:VIEM_CONFIG_DIR 'config.json') }
    try {
        if (!$Executable) {
            $Executable = Join-Path $projectRoot 'target/windows/Viem.Windows/bin/x64/Debug/net10.0-windows10.0.26100.0/win-x64/Viem.exe'
            if ($Optimized -or $NativeAot) { $Executable = Join-Path $optimizedOutput 'Viem.exe' }
        }
        $process = Start-Process -FilePath $executable -ArgumentList @('--self-test', ('"' + $reportPath + '"')) -PassThru -WindowStyle Hidden
        if (!$process.WaitForExit(120000)) { $process.Kill(); throw 'Windows integration tests timed out.' }
        if (!(Test-Path -LiteralPath $reportPath)) { throw "Windows app exited $($process.ExitCode) before writing a test report." }
        $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
        if (!$report.passed) { throw $report.error }
        if ($NativeAot -and !$ProfileDocument -and $report.nativeAot -ne $true) {
            throw 'The requested native UI checks ran in a managed executable.'
        }
        if ($ProfileDocument) { Write-Output "$ProfileScenario CPU benchmark completed. Report: $reportPath" }
        else { Write-Output "$($report.count) Windows integration checks passed. Report: $reportPath" }
    }
    finally { $env:VIEM_CONFIG_DIR = $oldProfile; $env:VIEM_PERF_DOCUMENT = $oldPerformanceDocument; $env:VIEM_PERF_SCENARIO = $oldPerformanceScenario; $env:VIEM_TEST_POINTER_INPUT = $oldPointerInput; $env:VIEM_TEST_DISABLE_PRELAYOUT = $oldPrelayout; $env:VIEM_PERF_INTERVAL_MS = $oldInterval }
}
finally { Pop-Location }
