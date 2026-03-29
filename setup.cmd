@echo off
setlocal

echo [+] Stargazer Windows Setup
echo.

:: Check for Rust
where rustc >nul 2>nul
if %errorlevel% neq 0 (
    echo [!] Rust not found. Installing via rustup...
    echo.
    echo     If a browser opens, download and run rustup-init.exe
    echo     Choose option 1 (default installation) when prompted.
    echo     Then close this window and re-run setup.cmd
    echo.
    start https://rustup.rs
    pause
    exit /b 1
)

echo [+] Rust found:
rustc --version
echo.

:: Check for Visual Studio Build Tools (cl.exe)
where cl >nul 2>nul
if %errorlevel% neq 0 (
    :: Try to find it via vswhere
    set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
    if not exist "%VSWHERE%" (
        echo [!] Visual Studio Build Tools not found.
        echo.
        echo     Install "Desktop development with C++" from:
        echo     https://visualstudio.microsoft.com/visual-cpp-build-tools/
        echo.
        echo     Or if already installed, run this from
        echo     "x64 Native Tools Command Prompt for VS"
        echo.
        pause
        exit /b 1
    )
)

echo [+] Building stargazer (release)...
echo.
cargo build --release
if %errorlevel% neq 0 (
    echo.
    echo [x] Build failed. Make sure Visual Studio Build Tools are installed
    echo     with the "Desktop development with C++" workload.
    pause
    exit /b 1
)

echo.
echo [+] Done! Run with:
echo     target\release\stargazer.exe
echo.
echo     Controls: ESC or Q to quit
echo     Logging:  stargazer.exe --log
echo.
pause
