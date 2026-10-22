# Remove every build output, like `make clean`: the Rust and Windows builds,
# test output and consistency baselines (all under target/), and the fuzz
# builds. Fuzz corpora and findings are kept.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    $paths = @('.build', 'target') +
        @(Get-ChildItem -LiteralPath 'fuzz' -Directory -Filter 'target*' | ForEach-Object { $_.FullName })
    foreach ($path in $paths) {
        if (!(Test-Path -LiteralPath $path)) { continue }
        try { Remove-Item -LiteralPath $path -Recurse -Force }
        catch { throw "Could not remove $path; close Viem and any running build or test first. $_" }
    }
    # Forget the consistency baseline worktrees deleted with target/.
    & git worktree prune
    if ($LASTEXITCODE -ne 0) { throw 'git worktree prune failed.' }
}
finally { Pop-Location }
