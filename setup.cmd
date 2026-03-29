@echo off
setlocal enabledelayedexpansion

echo [+] Stargazer Windows Setup
echo.

:: Check for Rust
where rustc >nul 2>nul
if %errorlevel% neq 0 goto :norust
echo [+] Rust found:
rustc --version
echo.
goto :checkbuild

:norust
echo [!] Rust not found. Installing via rustup...
echo.
echo     Download and run rustup-init.exe from https://rustup.rs
echo     Choose option 1 (default installation).
echo     Then re-run setup.cmd
echo.
start https://rustup.rs
pause
exit /b 1

:checkbuild
echo [+] Building stargazer (release)...
echo.
cargo build --release
if %errorlevel% neq 0 goto :buildfail
echo.
echo [+] Done! Run with:
echo     target\release\stargazer.exe
echo.
echo     Controls: ESC or Q to quit
echo     Logging:  stargazer.exe --log
echo.
pause
exit /b 0

:buildfail
echo.
echo [x] Build failed.
echo.
echo     If you see "linker not found", install Visual Studio Build Tools:
echo     https://visualstudio.microsoft.com/visual-cpp-build-tools/
echo     Select "Desktop development with C++" workload.
echo.
echo     Then re-run setup.cmd from "x64 Native Tools Command Prompt"
echo     or a fresh terminal after installation.
echo.
pause
exit /b 1
