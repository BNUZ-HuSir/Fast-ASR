param([switch]$PrepareAssets)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    $env:CARGO_HOME = Join-Path $projectRoot '.build\cargo'
    $env:CARGO_TARGET_DIR = Join-Path $projectRoot '.build\target'
    $env:TEMP = Join-Path $projectRoot '.build'
    $env:TMP = $env:TEMP
    $env:PYTHONDONTWRITEBYTECODE = '1'
    New-Item -ItemType Directory -Force -Path $env:TEMP | Out-Null
    if ($PrepareAssets) {
        python scripts/prepare_assets.py
        if ($LASTEXITCODE -ne 0) { throw 'Asset preparation failed' }
    }
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
    $metadata = cargo metadata --locked --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Dependency metadata failed' }
    $metadata | Set-Content -LiteralPath (Join-Path $projectRoot '.build\cargo-metadata.json') -Encoding utf8
    Write-Host "Executable: $env:CARGO_TARGET_DIR\release\fast-asr.exe"
} finally { Pop-Location }
