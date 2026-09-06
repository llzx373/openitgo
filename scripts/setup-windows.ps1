# setup-windows.ps1 — 准备 Windows 构建环境（libmpv 导入库 + 运行时 DLL）。
#
# 用法（仓库根目录）:
#   powershell -ExecutionPolicy Bypass -File scripts/setup-windows.ps1
#
# 可选参数:
#   -Version  mpv 构建版本（默认 20260830-git-e8673660ab，shinchiro SourceForge 构建）
#   -Proxy    下载代理（默认 http://127.0.0.1:7890；传空字符串直连）
#
# 产物（vendor/mpv/，不入库）:
#   mpv.lib       — MSVC 导入库（lib.exe 由 dumpbin 解析出的 mpv.def 生成）
#   libmpv-2.dll  — 运行时 DLL（openitgo-media/build.rs 会拷到 target/<profile>/ 下）
#
# 前置: MSVC 生成工具（VS 2022 / Build Tools，含 C++ 工作负载）。

param(
    [string]$Version = "20260830-git-e8673660ab",
    [string]$Proxy = "http://127.0.0.1:7890"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$vendor = Join-Path $root "vendor\mpv"
$dl = Join-Path $vendor "dl"
New-Item -ItemType Directory -Force $dl, (Join-Path $vendor "dev") | Out-Null

if ($Proxy) {
    $env:HTTPS_PROXY = $Proxy
    $env:HTTP_PROXY = $Proxy
}

$devUrl = "https://downloads.sourceforge.net/project/mpv-player-windows/libmpv/mpv-dev-x86_64-v3-$Version.7z"
$dev7z = Join-Path $dl "mpv-dev.7z"
if (-not (Test-Path $dev7z)) {
    Write-Host "下载 mpv-dev: $devUrl"
    curl.exe -sSL -o $dev7z $devUrl
}

Write-Host "解压 mpv-dev"
tar.exe -xf $dev7z -C (Join-Path $vendor "dev")

$dll = Join-Path $vendor "dev\libmpv-2.dll"
if (-not (Test-Path $dll)) { throw "解压后未找到 libmpv-2.dll" }
Copy-Item $dll (Join-Path $vendor "libmpv-2.dll") -Force

# 定位 MSVC lib.exe / dumpbin.exe（经 vswhere 找最新 VS 实例）
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vsPath) { throw "未找到带 C++ 工具集的 Visual Studio" }
$msvcBin = Get-ChildItem (Join-Path $vsPath "VC\Tools\MSVC") | Sort-Object Name -Descending | Select-Object -First 1
$binDir = Join-Path $msvcBin.FullName "bin\Hostx64\x64"
$libExe = Join-Path $binDir "lib.exe"
$dumpbinExe = Join-Path $binDir "dumpbin.exe"
if (-not (Test-Path $libExe)) { throw "未找到 lib.exe: $libExe" }

# dumpbin /exports -> mpv.def
Write-Host "生成 mpv.def"
$exports = & $dumpbinExe //exports $dll |
    Select-String '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]+\s+([A-Za-z_]\S*)' |
    ForEach-Object { $_.Matches[0].Groups[1].Value } | Sort-Object -Unique
$def = Join-Path $vendor "mpv.def"
@("LIBRARY libmpv-2.dll", "EXPORTS") + $exports | Set-Content $def

# lib.exe -> mpv.lib
Write-Host "生成 mpv.lib"
& $libExe //def:$def //machine:x64 //out:(Join-Path $vendor "mpv.lib")
if ($LASTEXITCODE -ne 0) { throw "lib.exe 失败 ($LASTEXITCODE)" }

Write-Host "完成: $vendor\mpv.lib, $vendor\libmpv-2.dll"
