@echo off
rem 一键:跑全部单元测试(Release 配置,复用编译缓存)
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0cargo.ps1" test --release
echo.
pause
