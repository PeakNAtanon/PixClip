@echo off
setlocal
cd /d "%~dp0..\.."
if not exist "PixClip.exe" (
    echo Build PixClip first: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build.ps1
    pause
    exit /b 1
)
"%CD%\PixClip.exe" %*
set "EXIT_CODE=%ERRORLEVEL%"
if not "%EXIT_CODE%"=="0" pause
exit /b %EXIT_CODE%
