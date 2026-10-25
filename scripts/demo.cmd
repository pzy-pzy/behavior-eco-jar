@echo off
rem 一键:增量构建 + 演示模式(约 2 分钟一个世代,可与正式实例同时运行)
rem 数据隔离在 %TEMP%\ecojar-demo,图鉴记录跨次累计
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0cargo.ps1" build --release
if errorlevel 1 goto :fail
set "ECOJAR_DATA_DIR=%TEMP%\ecojar-demo"
start "" "%~dp0..\target\release\ecojar.exe" --demo
exit /b 0
:fail
echo.
echo 构建失败,请检查上方输出
pause
