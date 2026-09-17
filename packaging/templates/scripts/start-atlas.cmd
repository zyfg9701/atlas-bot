@echo off
REM S1 one-click wrapper: bypass ExecutionPolicy for double-click / Start Menu.
REM Forwards all args to start-atlas.ps1 in this directory.
setlocal
set "SCRIPT_DIR=%~dp0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT_DIR%start-atlas.ps1" %*
exit /b %ERRORLEVEL%
