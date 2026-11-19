@echo off
setlocal
cd /d "%~dp0"
if not exist "%~dp0PixClip.exe" (
    echo Build PixClip first: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build.ps1
    pause
    exit /b 1
)
if "%~1"=="" (
    start "" "%~dp0PixClip.exe" --gui
) else (
    "%~dp0PixClip.exe" %*
)
