@echo off
setlocal
title GameDockPanel — setup and build

cd /d "%~dp0"

echo.
echo  GameDockPanel Windows setup
echo  Running PowerShell script...
echo.

powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0windows-setup-and-build.ps1" %*

if errorlevel 1 (
    echo.
    echo  Script failed. See errors above.
    pause
    exit /b 1
)

echo.
pause
