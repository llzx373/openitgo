# package-windows.ps1 — 构建 Windows 便携 zip 与安装包。
#
# 用法（仓库根目录）:
#   powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1
#   powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1 -NoInstaller
#
# 前置:
#   - 先运行 scripts/setup-windows.ps1 生成 vendor/mpv（mpv.lib + libmpv-2.dll）
#   - VS Build Tools（C++ 工作负载）；本脚本自带 vcvars64 环境
#   - 安装包需要 Inno Setup 6（ISCC.exe）；未安装时跳过并给出警告
#
# 产物:
#   dist/OpenItGo/                                 — 解压即用的便携目录
#   dist/openitgo-windows-x86_64-portable.zip      — 便携 zip
#   dist/OpenItGo-<version>-setup.exe              — Windows 安装包（含卸载器）

param(
    [switch]$NoInstaller
)

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

if ($NoInstaller) { return }

# Inno Setup 6：优先 PATH，其次常见安装位置（含按用户安装目录）。
$iscc = $null
$isccCmd = Get-Command iscc -ErrorAction SilentlyContinue
if ($isccCmd) { $iscc = $isccCmd.Source }
$isccCandidates = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Inno Setup 6\ISCC.exe"
)
foreach ($candidate in $isccCandidates) {
    if (-not $iscc -and (Test-Path $candidate)) { $iscc = $candidate }
}
if (-not $iscc) {
    Write-Warning "未找到 Inno Setup 6（ISCC.exe），跳过安装包；安装后重跑本脚本即可。"
    return
}

$version = (Select-String -Path (Join-Path $root "openitgo-app\Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "构建安装包（Inno Setup，版本 $version）..."
& $iscc "/DMyAppVersion=$version" (Join-Path $root "scripts\openitgo.iss")
if ($LASTEXITCODE -ne 0) { throw "ISCC 失败 ($LASTEXITCODE)" }

$setup = Join-Path $root "dist\OpenItGo-$version-setup.exe"
Write-Host "完成: $setup"
Get-Item $setup | Select-Object FullName, @{N = "MB"; E = { [math]::Round($_.Length / 1MB, 1) } }
