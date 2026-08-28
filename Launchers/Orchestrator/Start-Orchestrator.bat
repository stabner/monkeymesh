@echo off
setlocal
cd /d "%~dp0"
if exist "%~dp0bin\mesh-orchestrator.exe" (
  start "MonkeyMesh Orchestrator" "%~dp0bin\mesh-orchestrator.exe"
  exit /b 0
)
echo mesh-orchestrator.exe not found. Build with:
echo   .\Launchers\build-release.ps1
pause
endlocal
