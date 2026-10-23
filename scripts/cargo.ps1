# 在 MSVC 环境下调用 cargo。
# 用法: pwsh -File scripts/cargo.ps1 build
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere)) { throw "vswhere not found: $vswhere" }
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw "No Visual Studio C++ toolchain found" }
$vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path $vcvars)) { throw "vcvars64.bat not found: $vcvars" }

$env:CARGO_HOME = Join-Path $root '.tools\cargo'
$env:RUSTUP_HOME = Join-Path $root '.tools\rustup'
$tmp = Join-Path $root '.tools\tmp'
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$env:TMP = $tmp
$env:TEMP = $tmp
$env:Path = "$env:CARGO_HOME\bin;$env:Path"

$cmd = 'call "{0}" >nul && set "CARGO_HOME={1}" && set "RUSTUP_HOME={2}" && set "TMP={3}" && set "TEMP={3}" && cargo {4}' -f `
    $vcvars, $env:CARGO_HOME, $env:RUSTUP_HOME, $tmp, ($args -join ' ')
cmd /c $cmd
exit $LASTEXITCODE
