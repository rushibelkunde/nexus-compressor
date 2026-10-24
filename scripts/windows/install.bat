@echo off
setlocal
cd /d "%~dp0"
title NEXUS Archiver - 1-Click Installer
echo ==========================================================
echo          NEXUS Archiver - Windows Installer
echo ==========================================================
echo.
echo Installing NEXUS to your computer...
echo (Installs to %%LOCALAPPDATA%%\Programs\NEXUS - No Admin Rights Needed)
echo.

powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& { & '%~dp0install.ps1' }"

echo.
if %ERRORLEVEL% equ 0 (
    echo [SUCCESS] NEXUS is ready to use!
    echo You can now right-click any file or folder to compress or extract.
) else (
    echo [ERROR] Installation encountered an issue. See details above.
)
echo.
pause
