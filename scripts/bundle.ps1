# 打 NSIS 安装包。
# 依赖：.tools/tauri-cli/cargo-tauri.exe（Tauri CLI 预编译版）
# 用法: pwsh -File scripts/bundle.ps1
param([string]$Bundles = 'nsis')

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$cli = Join-Path $root '.tools\tauri-cli\cargo-tauri.exe'
if (-not (Test-Path $cli)) {
    throw "找不到 Tauri CLI：$cli。下载 cargo-tauri-x86_64-pc-windows-msvc.zip 解压到 .tools/tauri-cli/"
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvars64.bat'

$env:CARGO_HOME = Join-Path $root '.tools\cargo'
$env:RUSTUP_HOME = Join-Path $root '.tools\rustup'
$tmp = Join-Path $root '.tools\tmp'
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

$cmd = 'call "{0}" >nul && set "CARGO_HOME={1}" && set "RUSTUP_HOME={2}" && set "TMP={3}" && set "TEMP={3}" && set "PATH={1}\bin;%PATH%" && "{4}" build --bundles {5}' -f `
    $vcvars, $env:CARGO_HOME, $env:RUSTUP_HOME, $tmp, $cli, $Bundles
cmd /c $cmd
exit $LASTEXITCODE
