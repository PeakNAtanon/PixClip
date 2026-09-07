@echo off
rem Launch PixClip without a console window.
setlocal

cd /d "%~dp0"

where pyw.exe >nul 2>&1
if not errorlevel 1 (
    start "" /b pyw.exe "%~dp0media_toolkit.py" %*
    exit /b 0
)

where pythonw.exe >nul 2>&1
if not errorlevel 1 (
    start "" /b pythonw.exe "%~dp0media_toolkit.py" %*
    exit /b 0
)

echo Python 3 was not found.
echo Install Python 3 for Windows, then double-click this file again.
pause
exit /b 1
