param(
    [string]$Configuration = "release"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "Building tilik-ui ($Configuration)..."
cargo build --$Configuration

$targetDir = Join-Path $root "target\$Configuration"
$exe = Join-Path $targetDir "tilik-ui.exe"
if (-not (Test-Path $exe)) {
    throw "Release binary not found: $exe"
}

$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null

$zip = Join-Path $dist "tilik-win-rs-portable.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $exe -DestinationPath $zip
Write-Host "Portable package: $zip"

$iscc = "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
$iss = Join-Path $root "installer\setup.iss"
if ((Test-Path $iscc) -and (Test-Path $iss)) {
    Write-Host "Building installer..."
    & $iscc $iss
} else {
    Write-Host "Inno Setup not found or setup.iss missing; skipping installer."
}
Write-Host "Done."