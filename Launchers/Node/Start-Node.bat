@echo off
setlocal
cd /d "%~dp0"

if exist "%~dp0MonkeyMesh-Node.exe" (
  wscript //nologo "%~dp0_start-gui.vbs" "MonkeyMesh-Node.exe"
  exit /b 0
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start-Node.ps1" %*
endlocal
