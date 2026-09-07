@echo off
setlocal

cd /d "%~dp0"

where py.exe >nul 2>&1
if not errorlevel 1 goto run_with_py

where python.exe >nul 2>&1
if not errorlevel 1 goto run_with_python

echo Python 3 was not found.
echo Install Python 3 for Windows, then run this file again.
pause
exit /b 1

:run_with_py
py -3 "%~dp0media_toolkit.py" %*
goto finish

:run_with_python
python "%~dp0media_toolkit.py" %*

:finish
set "EXIT_CODE=%ERRORLEVEL%"
if not "%EXIT_CODE%"=="0" (
    echo.
    echo GUI exited with code %EXIT_CODE%.
    pause
)
exit /b %EXIT_CODE%
