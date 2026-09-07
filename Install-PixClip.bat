@echo off
setlocal EnableExtensions

cd /d "%~dp0"
set "PYTHON_LAUNCHER="

where py.exe >nul 2>&1
if not errorlevel 1 set "PYTHON_LAUNCHER=py"
if defined PYTHON_LAUNCHER goto install_tools

where python.exe >nul 2>&1
if not errorlevel 1 set "PYTHON_LAUNCHER=python"
if defined PYTHON_LAUNCHER goto install_tools

where winget.exe >nul 2>&1
if errorlevel 1 goto python_missing

echo Python 3 was not found. Installing it with winget...
winget install --id Python.Python.3.12 -e --source winget --accept-source-agreements --accept-package-agreements
set "EXIT_CODE=%ERRORLEVEL%"
if not "%EXIT_CODE%"=="0" goto install_failed

where py.exe >nul 2>&1
if not errorlevel 1 set "PYTHON_LAUNCHER=py"
if defined PYTHON_LAUNCHER goto install_tools

where python.exe >nul 2>&1
if not errorlevel 1 set "PYTHON_LAUNCHER=python"
if defined PYTHON_LAUNCHER goto install_tools

echo Python was installed, but this command window cannot find it yet.
echo Close this window, open Install-PixClip.bat again, and try once more.
pause
exit /b 1

:python_missing
echo Python 3 and winget were not found.
echo Install Python 3 from https://www.python.org/downloads/windows/ then run this file again.
pause
exit /b 1

:install_tools
if "%PYTHON_LAUNCHER%"=="py" (
    py -3 "%~dp0Install-Media-Tools.py"
) else (
    python "%~dp0Install-Media-Tools.py"
)
set "EXIT_CODE=%ERRORLEVEL%"
if not "%EXIT_CODE%"=="0" goto install_failed

if exist "%~dp0Create-PixClip-Shortcut.vbs" (
    cscript //nologo "%~dp0Create-PixClip-Shortcut.vbs"
)

echo.
echo PixClip installation is complete.
if exist "%~dp0OPEN-GUI-CMD.bat" call "%~dp0OPEN-GUI-CMD.bat"
exit /b 0

:install_failed
echo.
echo PixClip installation failed with exit code %EXIT_CODE%.
echo Review the message above, then run this file again after fixing the issue.
pause
exit /b %EXIT_CODE%
