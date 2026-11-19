@echo off
setlocal
cd /d "%~dp0"
if not exist "PixClip.exe" (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build.ps1"
    if errorlevel 1 goto failed
)
"%~dp0PixClip.exe" --install-tools
if errorlevel 1 goto failed
cscript //nologo "%~dp0scripts\windows\Create-PixClip-Shortcut.vbs"
call "%~dp0PixClip.bat"
exit /b 0
:failed
echo PixClip installation failed. Review the message above.
pause
exit /b 1
