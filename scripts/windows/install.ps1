# ==============================================================================
# NEXUS Windows Shell Integration Installer
# Installs nexus.exe (CLI), nexus-gui.exe (GUI), and custom branded .ico icon
# Adds Context Menus with GUI progress dialogs & 1-click presets in Windows File Explorer
# No Administrator Privileges Required (Uses CurrentUser Registry & LocalAppData)
# ==============================================================================

[CmdletBinding()]
param(
    [string]$SourceExe = "",
    [string]$SourceGui = ""
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   NEXUS Shell Integration & GUI Installer for Windows    " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# 1. Locate release binaries
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$projectRoot = Split-Path -Parent (Split-Path -Parent $scriptDir)

if (-not $SourceExe) {
    $possiblePaths = @(
        (Join-Path $projectRoot "target\release\nexus.exe"),
        (Join-Path $projectRoot "target\debug\nexus.exe"),
        (Join-Path $scriptDir "nexus.exe")
    )
    foreach ($p in $possiblePaths) {
        if (Test-Path $p) {
            $SourceExe = $p
            break
        }
    }
}

if (-not $SourceGui) {
    $possibleGuiPaths = @(
        (Join-Path $projectRoot "target\release\nexus-gui.exe"),
        (Join-Path $projectRoot "target\debug\nexus-gui.exe"),
        (Join-Path $scriptDir "nexus-gui.exe")
    )
    foreach ($p in $possibleGuiPaths) {
        if (Test-Path $p) {
            $SourceGui = $p
            break
        }
    }
}

if (-not $SourceExe -or -not (Test-Path $SourceExe) -or -not $SourceGui -or -not (Test-Path $SourceGui)) {
    Write-Host "[*] Building release binaries..." -ForegroundColor Yellow
    $env:PATH += ";$env:USERPROFILE\.cargo\bin"
    Push-Location $projectRoot
    cargo build --release --bin nexus --bin nexus-gui
    Pop-Location
    $SourceExe = Join-Path $projectRoot "target\release\nexus.exe"
    $SourceGui = Join-Path $projectRoot "target\release\nexus-gui.exe"
}

if (-not (Test-Path $SourceExe)) {
    Write-Error "Could not find or build nexus.exe at $SourceExe"
    exit 1
}
if (-not (Test-Path $SourceGui)) {
    Write-Error "Could not find or build nexus-gui.exe at $SourceGui"
    exit 1
}

# 2. Target installation directory
$installDir = Join-Path $env:LOCALAPPDATA "Programs\NEXUS"
if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

$destExe = Join-Path $installDir "nexus.exe"
$destGui = Join-Path $installDir "nexus-gui.exe"
$destIco = Join-Path $installDir "nexus.ico"
$destLogo = Join-Path $installDir "nexus_logo.png"

# Close any running instances before updating binaries
Get-Process -Name "nexus-gui", "nexus" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 250

Write-Host "[+] Installing NEXUS CLI to: $destExe" -ForegroundColor Green
Copy-Item -Path $SourceExe -Destination $destExe -Force

Write-Host "[+] Installing NEXUS GUI to: $destGui" -ForegroundColor Green
Copy-Item -Path $SourceGui -Destination $destGui -Force

$sourceIco = if (Test-Path (Join-Path $scriptDir "nexus.ico")) {
    Join-Path $scriptDir "nexus.ico"
} elseif (Test-Path (Join-Path $projectRoot "assets\nexus.ico")) {
    Join-Path $projectRoot "assets\nexus.ico"
} else { "" }

$sourceLogo = if (Test-Path (Join-Path $scriptDir "nexus_logo.png")) {
    Join-Path $scriptDir "nexus_logo.png"
} elseif (Test-Path (Join-Path $projectRoot "assets\nexus_logo.png")) {
    Join-Path $projectRoot "assets\nexus_logo.png"
} else { "" }

if ($sourceIco -and (Test-Path $sourceIco)) {
    Write-Host "[+] Installing NEXUS Logo Icon to: $destIco" -ForegroundColor Green
    Copy-Item -Path $sourceIco -Destination $destIco -Force
}
if ($sourceLogo -and (Test-Path $sourceLogo)) {
    Copy-Item -Path $sourceLogo -Destination $destLogo -Force
}

# Copy uninstall scripts to install directory
$uninstPs1 = Join-Path $scriptDir "uninstall.ps1"
if (Test-Path $uninstPs1) {
    Copy-Item -Path $uninstPs1 -Destination (Join-Path $installDir "uninstall.ps1") -Force
}
$uninstBat = Join-Path $scriptDir "uninstall.bat"
if (Test-Path $uninstBat) {
    Copy-Item -Path $uninstBat -Destination (Join-Path $installDir "uninstall.bat") -Force
}

# 3. Add to User PATH if not already present
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -split ';' -notcontains $installDir) {
    Write-Host "[+] Adding $installDir to User PATH..." -ForegroundColor Green
    $newPath = if ($userPath) { "$userPath;$installDir" } else { $installDir }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    $env:PATH = "$env:PATH;$installDir"
} else {
    Write-Host "[=] $installDir is already in User PATH." -ForegroundColor Gray
}

# 4. Configure Registry Context Menus using .NET Registry API (Fast, no wildcard issues)
Write-Host "[+] Registering Windows Explorer Right-Click Context Menus..." -ForegroundColor Green

$iconRef = if (Test-Path $destIco) { "`"$destIco`"" } else { "`"$destGui`",0" }

$classes = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey("Software\Classes")

# Helper function to register compression context menu
function Set-CompressionKey {
    param([Microsoft.Win32.RegistryKey]$parentKey)

    $shellKey = $parentKey.CreateSubKey("shell")

    # 1. Primary "Compress with NEXUS..." (Opens GUI dialog, NO console window)
    $guiCompress = $shellKey.CreateSubKey("NexusCompress")
    $guiCompress.SetValue("", "Compress with NEXUS...")
    $guiCompress.SetValue("Icon", $iconRef)
    $guiCompress.CreateSubKey("command").SetValue("", "`"$destGui`" compress `"%1`"")

    # 2. Cascading "NEXUS Quick Compress" Submenu with 3 Presets
    $presetsKey = $shellKey.CreateSubKey("NexusPresets")
    $presetsKey.SetValue("MUIVerb", "NEXUS Quick Compress")
    $presetsKey.SetValue("SubCommands", "")
    $presetsKey.SetValue("Icon", $iconRef)

    $subShell = $presetsKey.CreateSubKey("shell")

    # Low Preset
    $fastKey = $subShell.CreateSubKey("01Fast")
    $fastKey.SetValue("", "⚡ Fast (Low, Level 2)")
    $fastKey.CreateSubKey("command").SetValue("", "`"$destGui`" compress `"%1`" --preset low --auto-start")

    # Medium Preset (Default)
    $medKey = $subShell.CreateSubKey("02Medium")
    $medKey.SetValue("", "⚖️ Balanced (Medium, Level 6)")
    $medKey.CreateSubKey("command").SetValue("", "`"$destGui`" compress `"%1`" --preset medium --auto-start")

    # High Preset
    $highKey = $subShell.CreateSubKey("03High")
    $highKey.SetValue("", "💎 Maximum (High, Level 9)")
    $highKey.CreateSubKey("command").SetValue("", "`"$destGui`" compress `"%1`" --preset high --auto-start")
}

# A. Register for Any File (*)
$starKey = $classes.CreateSubKey("*")
Set-CompressionKey -parentKey $starKey

# B. Register for Any Directory
$dirKey = $classes.CreateSubKey("Directory")
Set-CompressionKey -parentKey $dirKey

# C. Register for .nxar and .nxs Archives (Extract with NEXUS GUI & Extract Here)
$assocExts = @(".nxar", ".nxs")
foreach ($ext in $assocExts) {
    $assocShell = $classes.CreateSubKey("SystemFileAssociations\$ext\shell")

    # 1. Extract with NEXUS (GUI dialog with destination picker & progress bar)
    $guiExtract = $assocShell.CreateSubKey("NexusExtractGui")
    $guiExtract.SetValue("", "Extract with NEXUS...")
    $guiExtract.SetValue("Icon", $iconRef)
    $guiExtract.CreateSubKey("command").SetValue("", "`"$destGui`" decompress `"%1`"")

    # 2. Extract Here (GUI with auto-start, NO terminal window!)
    $cliExtract = $assocShell.CreateSubKey("NexusExtractHere")
    $cliExtract.SetValue("", "Extract Here")
    $cliExtract.SetValue("Icon", $iconRef)
    $cliExtract.CreateSubKey("command").SetValue("", "`"$destGui`" decompress `"%1`" --auto-start")
}

# D. Register .nxar file type association and custom logo icon
$extKey = $classes.CreateSubKey(".nxar")
$extKey.SetValue("", "Nexus.Archive")

$typeKey = $classes.CreateSubKey("Nexus.Archive")
$typeKey.SetValue("", "NEXUS Compressed Archive")

# Branded logo icon for all .nxar files in Windows Explorer
$iconKey = $typeKey.CreateSubKey("DefaultIcon")
$iconKey.SetValue("", "`"$destIco`",0")

# Double-click action opens GUI extractor
$openCmdKey = $typeKey.CreateSubKey("shell\open\command")
$openCmdKey.SetValue("", "`"$destGui`" decompress `"%1`"")

# 5. Register in Windows "Installed Apps" / "Add or Remove Programs"
try {
    $uninstallRoot = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey("Software\Microsoft\Windows\CurrentVersion\Uninstall")
    $appUninstall = $uninstallRoot.CreateSubKey("NexusArchiver")
    $appUninstall.SetValue("DisplayName", "NEXUS Archiver")
    $appUninstall.SetValue("DisplayVersion", "0.1.0")
    $appUninstall.SetValue("Publisher", "NEXUS Core Team")
    $appUninstall.SetValue("DisplayIcon", $destIco)
    $appUninstall.SetValue("InstallLocation", $installDir)
    $destUninstallBat = Join-Path $installDir "uninstall.bat"
    $appUninstall.SetValue("UninstallString", "`"$destUninstallBat`"")
    $appUninstall.SetValue("NoModify", 1, [Microsoft.Win32.RegistryValueKind]::DWord)
    $appUninstall.SetValue("NoRepair", 1, [Microsoft.Win32.RegistryValueKind]::DWord)
    Write-Host "[+] Registered in Windows Settings (Installed Apps)." -ForegroundColor Green
} catch {}

# 6. Create Start Menu Shortcut
try {
    $startMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    if (Test-Path $startMenuDir) {
        $shortcutPath = Join-Path $startMenuDir "NEXUS Archiver.lnk"
        $wshShell = New-Object -ComObject WScript.Shell
        $shortcut = $wshShell.CreateShortcut($shortcutPath)
        $shortcut.TargetPath = $destGui
        $shortcut.IconLocation = if (Test-Path $destIco) { "$destIco,0" } else { "$destGui,0" }
        $shortcut.Description = "NEXUS Archiver - High-Performance Next-Gen Compression"
        $shortcut.WorkingDirectory = $installDir
        $shortcut.Save()
        Write-Host "[+] Created Start Menu Shortcut: $shortcutPath" -ForegroundColor Green
    }
} catch {}

# 7. Refresh Windows Explorer Icon Cache
try {
    & ie4uinit.exe -show 2>$null
    Write-Host "[+] Refreshed Windows Explorer Icon Cache." -ForegroundColor Green
} catch {}

Write-Host "==========================================================" -ForegroundColor Green
Write-Host " [SUCCESS] NEXUS installed successfully!" -ForegroundColor Green
Write-Host " - GUI Archiver: $destGui" -ForegroundColor White
Write-Host " - CLI Engine  : $destExe" -ForegroundColor White
Write-Host " - Brand Icon  : $destIco" -ForegroundColor White
Write-Host " - Double-click any .nxar file -> Opens GUI Extractor" -ForegroundColor White
Write-Host " - Right-click any file/folder -> 'Compress with NEXUS...' (GUI)" -ForegroundColor White
Write-Host " - Right-click any file/folder -> 'NEXUS Quick Compress' (Low / Medium / High)" -ForegroundColor White
Write-Host " - Right-click any .nxar file  -> 'Extract with NEXUS...'" -ForegroundColor White
Write-Host "==========================================================" -ForegroundColor Green
