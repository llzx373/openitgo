@echo off
REM cargo-win.bat — 在 MSVC 生成环境（vcvars64）内运行 cargo。
REM 用法: scripts\cargo-win.bat <cargo 参数...>   例: scripts\cargo-win.bat check --workspace
REM 本机 VS2022 Community 的 MSVC 库目录不完整（缺 msvcrt.lib），
REM 因此固定使用 VS2019 BuildTools 的 v142 工具集 + Windows SDK 10.0.26100。
setlocal
if not defined LIB (
    call "C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >NUL
)
if not defined HTTPS_PROXY set HTTPS_PROXY=http://127.0.0.1:7890
if not defined HTTP_PROXY set HTTP_PROXY=http://127.0.0.1:7890
set PATH=%USERPROFILE%\.cargo\bin;%PATH%
cargo %*
