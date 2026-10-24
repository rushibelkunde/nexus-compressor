@echo off
setlocal
cd /d "%~dp0"
title NEXUS Archiver - Uninstaller
echo ==========================================================
echo          NEXUS Archiver - Windows Uninstaller
echo ==========================================================
echo.
echo Removing NEXUS from your computer...
echo.

powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& { & '%~dp0uninstall.ps1' }"

echo.
pause
