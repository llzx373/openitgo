# package-windows.ps1 — 构建 Windows 便携 zip 发布包。
#
# 用法（仓库根目录）:
#   powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1
#
# 前置:
#   - 先运行 scripts/setup-windows.ps1 生成 vendor/mpv（mpv.lib + libmpv-2.dll）
#   - VS Build Tools（C++ 工作负载）；本脚本自带 vcvars64 环境
#
# 产物:
#   dist/OpenItGo/            — 解压即用的便携目录
#   dist/openitgo-windows-x86_64-portable.zip

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$dll = Join-Path $root "vendor\mpv\libmpv-2.dll"
if (-not (Test-Path $dll)) {
    throw "未找到 vendor/mpv/libmpv-2.dll，请先运行 scripts/setup-windows.ps1"
}

# MSVC 环境（用 vswhere 找最新的带 C++ 工具集的 VS 实例，本机为 VS2022
# Community；若已处于 vcvars 环境（LIB 已设置）则跳过）
if (-not $env:LIB) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vsPath) { throw "未找到带 C++ 工具集的 Visual Studio" }
    $vcvars = Join-Path $vsPath "VC\Auxiliary\Build\vcvars64.bat"
    # 在 cmd 里跑 vcvars 再把环境导回当前进程
    $envLines = cmd /c "`"$vcvars`" >NUL 2>&1 && set"
    foreach ($line in $envLines) {
        if ($line -match '^([^=]+)=(.*)$') {
            [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], "Process")
        }
    }
}
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

Write-Host "构建 release（openitgo-app）..."
cargo build --release -p openitgo-app
if ($LASTEXITCODE -ne 0) { throw "cargo build 失败 ($LASTEXITCODE)" }

$stage = Join-Path $root "dist\OpenItGo"
$zip = Join-Path $root "dist\openitgo-windows-x86_64-portable.zip"
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null

Copy-Item (Join-Path $root "target\release\openitgo-app.exe") (Join-Path $stage "OpenItGo.exe")
Copy-Item $dll $stage
Copy-Item (Join-Path $root "README.md"), (Join-Path $root "LICENSE"), (Join-Path $root "CHANGELOG.md") $stage

Remove-Item $zip -Force -ErrorAction SilentlyContinue
Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal

Write-Host "完成: $zip"
Get-Item $zip | Select-Object FullName, @{N = "MB"; E = { [math]::Round($_.Length / 1MB, 1) } }
