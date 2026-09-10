@echo off
REM Mock Cursor/Atlas Agent CLI for Windows (no real Cursor login).
REM Compatible surface: accepts flags + a prompt; prints a NON-ECHO reply.
REM Reply MUST contain atlas-mock-reply (NOT Hub echo: stub).
REM Simpler than the .sh mock: ignores unknown flags; chars= is prompt length.
setlocal EnableExtensions EnableDelayedExpansion

set "PROMPT_TEXT="
set "AGENT_ID=unknown"
if defined ATLAS_AGENT_ID set "AGENT_ID=%ATLAS_AGENT_ID%"

:parse
if "%~1"=="" goto done_parse
if /I "%~1"=="-p" (
  shift
  goto parse
)
if /I "%~1"=="--print" (
  shift
  goto parse
)
if /I "%~1"=="--output-format" (
  shift
  if not "%~1"=="" shift
  goto parse
)
set "ARG=%~1"
if "!ARG:~0,1!"=="-" (
  shift
  goto parse
)
if not defined PROMPT_TEXT (
  set "PROMPT_TEXT=%~1"
) else (
  set "PROMPT_TEXT=!PROMPT_TEXT! %~1"
)
shift
goto parse

:done_parse
if defined MOCK_CLI_SLEEP_MS (
  set /A "SLEEP_S=(%MOCK_CLI_SLEEP_MS%+999)/1000" 2>nul
  if defined SLEEP_S if !SLEEP_S! GTR 0 timeout /T !SLEEP_S! /NOBREAK >nul 2>nul
)

set "PLEN=0"
call :strlen PLEN PROMPT_TEXT
echo atlas-mock-reply agent=!AGENT_ID! chars=!PLEN!
exit /b 0

:strlen
set "strlen_s=!%~2!"
set /A "%~1=0"
:strlen_loop
if not defined strlen_s goto :eof
set "strlen_s=!strlen_s:~1!"
set /A "%~1+=1"
goto strlen_loop
