param([switch]$NoRustBuild)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    & python -B -m unittest discover -s scripts/tests -p test_vim_runtime.py
    if ($LASTEXITCODE -ne 0) { throw 'Runtime packaging tests failed.' }
    if (!$NoRustBuild) {
        & "$PSScriptRoot/build-win.ps1" -Configuration Debug -Offline
    }
    $build = Join-Path $projectRoot 'target/windows/Viem.Windows/bin/x64/Debug/net10.0-windows10.0.26100.0/win-x64'
    $testRoot = Join-Path $projectRoot ('target/windows-validation/vim-' + [Guid]::NewGuid().ToString('N'))
    # Exercise Unicode paths even when this script is read by Windows PowerShell 5.
    $publish = Join-Path $testRoot ('publish ' + [char]0x65E5 + [char]0x672C)
    $relocated = Join-Path $testRoot ('relocated app ' + [char]0x65E5 + [char]0x672C)
    function Seed-StaleRuntime([string]$OutputDirectory) {
        $owned = Join-Path $OutputDirectory 'Resources/vim/runtime/syntax'
        New-Item -ItemType Directory -Path $owned -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $owned 'obsolete.viem-test') -Value 'obsolete'
        Set-Content -LiteralPath (Join-Path $OutputDirectory 'Resources/keep.viem-test') -Value 'keep'
    }
    function Verify-Output([string]$OutputDirectory) {
        & python scripts/vim-runtime.py verify (Join-Path $OutputDirectory 'Resources/vim')
        if ($LASTEXITCODE -ne 0) { throw 'Packaged runtime verification failed.' }
        if (Test-Path -LiteralPath (Join-Path $OutputDirectory 'Resources/vim/runtime/syntax/obsolete.viem-test')) { throw 'Stale runtime file survived packaging.' }
        if ((Get-Content -LiteralPath (Join-Path $OutputDirectory 'Resources/keep.viem-test')) -ne 'keep') { throw 'Packaging changed neighboring resources.' }
        foreach ($name in @('README.md', 'manifest.json', 'runtime/LICENSE', 'runtime/syntax/shared/debarchitectures.vim')) {
            if ((Get-FileHash -LiteralPath (Join-Path $OutputDirectory "Resources/vim/$name")).Hash -ne
                (Get-FileHash -LiteralPath (Join-Path $projectRoot "assets/vim/$name")).Hash) { throw "Packaged bytes differ: $name" }
        }
    }
    Seed-StaleRuntime $build
    & dotnet build src/win/Viem.Windows.csproj -c Debug --no-restore --nologo
    if ($LASTEXITCODE -ne 0) { throw 'Direct Windows build failed.' }
    Verify-Output $build
    Seed-StaleRuntime $publish
    & dotnet publish src/win/Viem.Windows.csproj -c Debug --no-build --no-restore --nologo -o $publish
    if ($LASTEXITCODE -ne 0) { throw 'Windows publish failed.' }
    Verify-Output $publish

    $savedEnvironment = @{}
    foreach ($key in @('VIEM_CONFIG_DIR', 'VIEM_TEST_SYNTAX_ONLY', 'VIEM_TEST_VIM_MISSING')) {
        $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
    }
    function Test-Syntax([string]$OutputDirectory, [string]$Name, [bool]$Missing = $false) {
        $env:VIEM_CONFIG_DIR = Join-Path $testRoot "$Name-profile"
        $env:VIEM_TEST_SYNTAX_ONLY = '1'
        $env:VIEM_TEST_VIM_MISSING = if ($Missing) { '1' } else { $null }
        $report = Join-Path $testRoot "$Name.json"
        $process = Start-Process -FilePath (Join-Path $OutputDirectory 'Viem.exe') -WorkingDirectory $testRoot `
            -ArgumentList @('--self-test', ('"' + $report + '"')) -PassThru -WindowStyle Hidden
        if (!$process.WaitForExit(60000)) { $process.Kill(); throw 'Vim syntax checks timed out.' }
        if (!(Test-Path -LiteralPath $report)) { throw "App exited $($process.ExitCode) without a syntax report." }
        $result = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
        if (!$result.passed -or $process.ExitCode -ne 0) { throw $result.error }
        Write-Output "$Name`: $($result.count) syntax checks passed. Report: $report"
    }
    try {
        Test-Syntax $build 'build'
        Test-Syntax $publish 'publish'
        # Check absolute containment before moving either generated directory.
        $rootPrefix = [IO.Path]::GetFullPath($testRoot) + [IO.Path]::DirectorySeparatorChar
        foreach ($path in @($publish, $relocated)) {
            if (![IO.Path]::GetFullPath($path).StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Relocation escaped test output.' }
        }
        Move-Item -LiteralPath $publish -Destination $relocated
        Test-Syntax $relocated 'relocated'
        $resources = Join-Path $relocated 'Resources/vim'
        $disabled = Join-Path $relocated 'Resources/vim-unavailable'
        foreach ($path in @($resources, $disabled)) {
            if (!(Resolve-Path -LiteralPath (Split-Path -Parent $path)).Path.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Missing-resource fixture escaped test output.' }
        }
        Move-Item -LiteralPath $resources -Destination $disabled
        try { Test-Syntax $relocated 'missing-runtime' $true }
        finally { Move-Item -LiteralPath $disabled -Destination $resources }
    }
    finally {
        foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key]) }
    }
}
finally { Pop-Location }
