@echo off
cd /d "%~dp0"
cargo build --release --workspace || exit /b 1
if not exist dist mkdir dist
copy /y target\release\api.exe dist\ >nul
copy /y target\release\client.exe dist\Rustify.exe >nul
set ISCC=%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe
if not exist "%ISCC%" set ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe
if exist "%ISCC%" "%ISCC%" /Q installer\rustify.iss || exit /b 1
echo Built to %~dp0dist
