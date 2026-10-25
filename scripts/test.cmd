@echo off
chcp 65001 >nul
rem 一键:跑全部单元测试(Release 配置,复用编译缓存,秒级)
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0cargo.ps1" test --release
echo.
pause
