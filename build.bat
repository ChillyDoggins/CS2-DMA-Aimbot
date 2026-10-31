@echo off
cd /d "%~dp0"
echo Building cs2_aimbot...
cargo build --release 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo BUILD FAILED - see errors above
    pause
) else (
    echo.
    echo BUILD SUCCEEDED - binary at target\release\cs2_aimbot.exe
    pause
)
