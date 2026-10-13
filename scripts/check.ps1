$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$cargoDirectory = Join-Path $env:USERPROFILE '.cargo\bin'
if (Test-Path -LiteralPath $cargoDirectory) { $env:PATH = "$cargoDirectory;$env:PATH" }
Push-Location $projectRoot
try {
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed.' }
    cargo clippy --locked --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed.' }
    cargo test --locked
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
} finally { Pop-Location }
