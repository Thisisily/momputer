# Builds the mom CLI and the momputer installer.
#   .\scripts\build.ps1            -> target\release\bundle\ (NSIS + MSI)
#   .\scripts\build.ps1 -NoBundle  -> just target\release\momputer.exe and mom.exe
param([switch]$NoBundle)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

cargo build --release -p mom-cli
if ($LASTEXITCODE -ne 0) { throw 'building the mom CLI failed' }

# Tauri bundles mom.exe next to the app; it wants the target triple in the file name.
$triple = ((rustc -vV) | Select-String '^host: (.+)$').Matches[0].Groups[1].Value
New-Item -ItemType Directory -Force 'app\src-tauri\binaries' | Out-Null
Copy-Item 'target\release\mom.exe' "app\src-tauri\binaries\mom-$triple.exe" -Force

if ($NoBundle) {
    cargo build --release -p momputer
    if ($LASTEXITCODE -ne 0) { throw 'building the app failed' }
    Write-Host 'Built target\release\momputer.exe and target\release\mom.exe'
    exit 0
}

if (-not (Get-Command cargo-tauri -ErrorAction SilentlyContinue)) {
    Write-Host 'Installing the Tauri CLI (one time)...'
    cargo install tauri-cli --version '^2' --locked
}
Push-Location 'app\src-tauri'
try {
    cargo tauri build
    if ($LASTEXITCODE -ne 0) { throw 'bundling failed' }
} finally {
    Pop-Location
}
Write-Host 'Installers are in target\release\bundle\'
