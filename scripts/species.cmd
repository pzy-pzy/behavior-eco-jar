@echo off
chcp 65001 >nul
rem 一键:直接预览某个物种的完满形态(不自动收集,可与正式实例同时运行)
rem 用法: species.cmd <deep_glow|tide|school|sunken_jade|dawn|night>
if "%~1"=="" (
  echo 用法: species.cmd ^<物种 id^>
  echo   deep_glow    深光种(浮伞型,聚焦时的稳定辉光)
  echo   tide         潮汐种(浮伞型,随节律开合)
  echo   school       群游种(群游型,6 条小鱼,点击惊散)
  echo   sunken_jade  沉玉种(辐射型,底栖海星,点击卷腕)
  echo   dawn         晨光种(流线型,金色侧线,点击急冲)
  echo   night        夜洄种(流线型,蓝紫节段,点击急冲)
  pause
  exit /b 1
)
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0cargo.ps1" build --release
if errorlevel 1 goto :fail
set "ECOJAR_DATA_DIR=%TEMP%\ecojar-preview-%~1"
start "" "%~dp0..\target\release\ecojar.exe" --demo --preview-species %~1
exit /b 0
:fail
echo.
echo 构建失败,请检查上方输出
pause
