# ==============================================================================
# NEXUS Windows Shell Integration Uninstaller
# Cleanly removes context menus, file associations, PATH entries, and files
# No Administrator Privileges Required
# ==============================================================================

[CmdletBinding()]
param()

$ErrorActionPreference = "SilentlyContinue"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "       NEXUS Archiver Uninstaller for Windows             " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$installDir = Join-Path $env:LOCALAPPDATA "Programs\NEXUS"

# 1. Remove Registry Context Menus using .NET Registry API
Write-Host "[*] Removing Windows Explorer context menus and associations..." -ForegroundColor Yellow

$classes = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Classes", $true)
if ($classes) {
    # Remove from *
    $starShell = $classes.OpenSubKey("*\shell", $true)
    if ($starShell) {
        $starShell.DeleteSubKeyTree("NexusCompress", $false)
        $starShell.DeleteSubKeyTree("NexusPresets", $false)
        $starShell.Close()
    }

    # Remove from Directory
    $dirShell = $classes.OpenSubKey("Directory\shell", $true)
    if ($dirShell) {
        $dirShell.DeleteSubKeyTree("NexusCompress", $false)
        $dirShell.DeleteSubKeyTree("NexusPresets", $false)
        $dirShell.Close()
    }

    # Remove from SystemFileAssociations (.nxar, .nxs)
    $assocKeys = @(".nxar", ".nxs")
    foreach ($ext in $assocKeys) {
        $extShell = $classes.OpenSubKey("SystemFileAssociations\$ext\shell", $true)
        if ($extShell) {
            $extShell.DeleteSubKeyTree("NexusExtractGui", $false)
            $extShell.DeleteSubKeyTree("NexusExtractHere", $false)
            $extShell.Close()
        }
        $classes.DeleteSubKeyTree("SystemFileAssociations\$ext", $false)
    }

    # Remove file extension and ProgID
    $classes.DeleteSubKeyTree(".nxar", $false)
    $classes.DeleteSubKeyTree("Nexus.Archive", $false)
    $classes.Close()
}

# 2. Remove from Windows Add/Remove Programs
$uninstallKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Microsoft\Windows\CurrentVersion\Uninstall", $true)
if ($uninstallKey) {
    $uninstallKey.DeleteSubKeyTree("NexusArchiver", $false)
    $uninstallKey.Close()
}

# 3. Remove Start Menu Shortcut
$startMenuShortcut = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\NEXUS Archiver.lnk"
if (Test-Path $startMenuShortcut) {
    Remove-Item -Path $startMenuShortcut -Force -ErrorAction SilentlyContinue
    Write-Host "[*] Removed Start Menu Shortcut." -ForegroundColor Yellow
}

# 3. Remove from User PATH
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath) {
    $pathParts = $userPath -split ';' | Where-Object { $_ -ne $installDir -and $_ -ne "" }
    $newPath = $pathParts -join ';'
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "[*] Removed $installDir from User PATH." -ForegroundColor Yellow
}

# 4. Remove Installation Directory
Get-Process -Name "nexus-gui", "nexus" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 250

if (Test-Path $installDir) {
    Write-Host "[*] Removing installation files at: $installDir" -ForegroundColor Yellow
    Remove-Item -Path $installDir -Recurse -Force -ErrorAction SilentlyContinue
}

# 5. Refresh Windows Explorer Icon Cache
try {
    & ie4uinit.exe -show 2>$null
} catch {}

Write-Host "==========================================================" -ForegroundColor Green
Write-Host " [SUCCESS] NEXUS has been completely uninstalled.        " -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green
