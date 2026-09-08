@echo off
REM cargo-win.bat — 在 MSVC 生成环境（vcvars64）内运行 cargo。
REM 用法: scripts\cargo-win.bat <cargo 参数...>   例: scripts\cargo-win.bat check --workspace
REM 用 vswhere 定位最新的带 C++(x64) 工具的 VS 实例（本机为 VS2022 Community）。
setlocal enabledelayedexpansion
if not defined LIB (
    for /f "usebackq tokens=*" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VS_INSTALL=%%i"
    if defined VS_INSTALL call "!VS_INSTALL!\VC\Auxiliary\Build\vcvars64.bat" >NUL
)
if not defined HTTPS_PROXY set HTTPS_PROXY=http://127.0.0.1:7890
if not defined HTTP_PROXY set HTTP_PROXY=http://127.0.0.1:7890
set PATH=%USERPROFILE%\.cargo\bin;%PATH%
cargo %*
