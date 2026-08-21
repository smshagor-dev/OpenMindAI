@echo off
setlocal EnableExtensions

rem OpenMindAI universal Windows bootstrap launcher.
rem Version: 2.0.0
rem Kept intentionally small -- all real logic lives in
rem bootstrap\windows\bootstrap.ps1 (PowerShell). This file just locates
rem itself, finds/fetches that script, and hands off to it.

set "SCRIPT_DIR=%~dp0"
set "BOOTSTRAP_PS1=%SCRIPT_DIR%bootstrap\windows\bootstrap.ps1"

where powershell >nul 2>nul
if errorlevel 1 (
    where pwsh >nul 2>nul
    if errorlevel 1 (
        echo.
        echo [OpenMindAI Setup] PowerShell was not found on this system.
        echo OpenMindAI Setup requires Windows PowerShell 5.1+ or PowerShell 7+,
        echo which ships with Windows 10/11 by default.
        echo.
        pause
        exit /b 1
    )
    set "PS_EXE=pwsh"
) else (
    set "PS_EXE=powershell"
)

if not exist "%BOOTSTRAP_PS1%" (
    echo.
    echo [OpenMindAI Setup] bootstrap\windows\bootstrap.ps1 was not found next to this file.
    echo This looks like a standalone copy of OpenMindAI-Setup.bat -- fetching
    echo the bootstrap script from the official repository...
    echo.
    if not exist "%SCRIPT_DIR%bootstrap\windows" mkdir "%SCRIPT_DIR%bootstrap\windows" >nul 2>nul
    "%PS_EXE%" -NoProfile -ExecutionPolicy Bypass -Command ^
        "$ProgressPreference='SilentlyContinue'; try { Invoke-WebRequest -UseBasicParsing -Uri 'https://raw.githubusercontent.com/smshagor-dev/OpenMindAI/main/bootstrap/windows/bootstrap.ps1' -OutFile '%BOOTSTRAP_PS1%' } catch { exit 1 }"
    if errorlevel 1 (
        echo [OpenMindAI Setup] Could not download the bootstrap script. Check your
        echo internet connection, or download the full OpenMindAI repository instead:
        echo https://github.com/smshagor-dev/OpenMindAI
        echo.
        pause
        exit /b 1
    )
)

rem %SCRIPT_DIR% always ends in a backslash (from %~dp0) -- passed straight
rem into a quoted argument, a trailing \" is parsed by the C runtime as an
rem escaped literal quote, corrupting the argument. Appending "." keeps the
rem path pointing at the same directory while avoiding that trap.
"%PS_EXE%" -NoProfile -ExecutionPolicy Bypass -File "%BOOTSTRAP_PS1%" -LauncherRoot "%SCRIPT_DIR%." %*
set "EXITCODE=%ERRORLEVEL%"

if not "%EXITCODE%"=="0" (
    echo.
    echo [OpenMindAI Setup] Setup did not complete successfully ^(exit code %EXITCODE%^).
    echo See the messages above for details.
    echo.
    pause
)

exit /b %EXITCODE%
