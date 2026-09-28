@echo off
REM ============================================================
REM  Intel I225 / I226 NVM firmware inspector launcher
REM  Drag one or more .bin files onto this file.
REM  Prefers nvm_info.exe; falls back to nvm_info.py if missing.
REM  NOTE: this file is intentionally ASCII-only.
REM ============================================================
chcp 65001 >nul
setlocal enabledelayedexpansion
set "TARGETS="

if not "%~1"=="" goto RUN
set /p TARGETS=Drag files here, or type paths (space separated):
goto RUN

:RUN
pushd "%~dp0"
if exist "nvm_info.exe" (
  nvm_info.exe %TARGETS% %*
) else (
  set PYTHONIOENCODING=utf-8
  set PY=C:\Users\yun_9\.workbuddy\binaries\python\versions\3.13.12\python.exe
  if exist "!PY!" (
    "!PY!" nvm_info.py %TARGETS% %*
  ) else (
    echo [ERROR] nvm_info.exe not found and Python is missing.
    echo         Fix the PY= line in this .bat, or restore nvm_info.exe.
  )
)
echo.
pause
popd
endlocal
