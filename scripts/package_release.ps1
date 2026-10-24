# ==============================================================================
# NEXUS Release Packager
# Builds optimized release binaries and creates a portable 1-click distribution
# Creates:
#   dist\NEXUS-v0.1.0-Windows-x64\
#   dist\NEXUS-v0.1.0-Windows-x64.zip
# ==============================================================================

[CmdletBinding()]
param(
    [string]$Version = "0.1.0"
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "         NEXUS Archiver Release Packager                  " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$projectRoot = Split-Path -Parent $scriptDir

# 1. Build optimized release binaries
Write-Host "[*] Building release binaries (nexus.exe, nexus-gui.exe)..." -ForegroundColor Yellow
$env:PATH += ";$env:USERPROFILE\.cargo\bin"
Push-Location $projectRoot
cargo build --release --bin nexus --bin nexus-gui
Pop-Location

$sourceExe = Join-Path $projectRoot "target\release\nexus.exe"
$sourceGui = Join-Path $projectRoot "target\release\nexus-gui.exe"
$sourceIco = Join-Path $projectRoot "assets\nexus.ico"
$sourceLogo = Join-Path $projectRoot "assets\nexus_logo.png"

if (-not (Test-Path $sourceExe)) {
    Write-Error "nexus.exe not found at $sourceExe"
    exit 1
}
if (-not (Test-Path $sourceGui)) {
    Write-Error "nexus-gui.exe not found at $sourceGui"
    exit 1
}

# 2. Prepare Distribution Directory
$distRoot = Join-Path $projectRoot "dist"
$pkgName = "NEXUS-v$Version-Windows-x64"
$pkgDir = Join-Path $distRoot $pkgName
$zipFile = Join-Path $distRoot "$pkgName.zip"

if (Test-Path $pkgDir) {
    Remove-Item -Path $pkgDir -Recurse -Force
}
if (Test-Path $zipFile) {
    Remove-Item -Path $zipFile -Force
}

New-Item -ItemType Directory -Path $pkgDir -Force | Out-Null
Write-Host "[+] Created package directory: $pkgDir" -ForegroundColor Green

# 3. Copy binaries and assets
Copy-Item -Path $sourceExe -Destination (Join-Path $pkgDir "nexus.exe") -Force
Copy-Item -Path $sourceGui -Destination (Join-Path $pkgDir "nexus-gui.exe") -Force

if (Test-Path $sourceIco) {
    Copy-Item -Path $sourceIco -Destination (Join-Path $pkgDir "nexus.ico") -Force
}
if (Test-Path $sourceLogo) {
    Copy-Item -Path $sourceLogo -Destination (Join-Path $pkgDir "nexus_logo.png") -Force
}

# 4. Copy 1-click installer and uninstaller scripts
$scriptsDir = Join-Path $projectRoot "scripts\windows"
Copy-Item -Path (Join-Path $scriptsDir "install.bat") -Destination (Join-Path $pkgDir "install.bat") -Force
Copy-Item -Path (Join-Path $scriptsDir "install.ps1") -Destination (Join-Path $pkgDir "install.ps1") -Force
Copy-Item -Path (Join-Path $scriptsDir "uninstall.bat") -Destination (Join-Path $pkgDir "uninstall.bat") -Force
Copy-Item -Path (Join-Path $scriptsDir "uninstall.ps1") -Destination (Join-Path $pkgDir "uninstall.ps1") -Force

# 5. Create user README.txt
$readmeContent = @"
==============================================================================
                              NEXUS ARCHIVER v$Version
                 Neural EXtreme Unified compresSion for Windows
==============================================================================

Thank you for downloading NEXUS Archiver!

------------------------------------------------------------------------------
HOW TO INSTALL (1-Click, No Administrator Privileges Required):
------------------------------------------------------------------------------
1. Double-click "install.bat".
2. That's it! 
   - Adds "Compress with NEXUS..." & Quick Presets to Windows Explorer right-click menus.
   - Associates .nxar archive files with the custom NEXUS logo.
   - Adds "NEXUS Archiver" to your Windows Start Menu.
   - Adds the 'nexus' CLI command to your User PATH.

------------------------------------------------------------------------------
PORTABLE USE (No Installation Required):
------------------------------------------------------------------------------
- You can simply double-click "nexus-gui.exe" to run the visual archiver portably!
- You can run "nexus.exe" in PowerShell or CMD for command-line compression.

------------------------------------------------------------------------------
HOW TO UNINSTALL:
------------------------------------------------------------------------------
- Double-click "uninstall.bat" OR uninstall anytime via:
  Windows Settings -> Apps -> Installed Apps -> NEXUS Archiver -> Uninstall.

------------------------------------------------------------------------------
KEY FEATURES:
------------------------------------------------------------------------------
* 3 Presets: Fast (Low), Balanced (Medium), Maximum (High)
* Custom Levels 1 to 19 (including Context Mixing and Neural Prediction)
* Pre-flight size estimator: shows estimated output size before compressing
* Modern dark-mode GUI with real-time speed (MB/s) and ETA progress bars
==============================================================================
"@

Set-Content -Path (Join-Path $pkgDir "README.txt") -Value $readmeContent -Encoding UTF8
Write-Host "[+] Created README.txt" -ForegroundColor Green

# 6. Compress into .zip archive
Write-Host "[*] Packaging into $zipFile..." -ForegroundColor Yellow
Compress-Archive -Path "$pkgDir\*" -DestinationPath $zipFile -CompressionLevel Optimal

$zipSize = (Get-Item $zipFile).Length
$zipSizeMb = [math]::Round($zipSize / 1MB, 2)

Write-Host "==========================================================" -ForegroundColor Green
Write-Host " [SUCCESS] Release package created successfully!" -ForegroundColor Green
Write-Host " - Folder : $pkgDir" -ForegroundColor White
Write-Host " - ZIP    : $zipFile ($zipSizeMb MB)" -ForegroundColor White
Write-Host "==========================================================" -ForegroundColor Green
