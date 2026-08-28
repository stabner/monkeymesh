@echo off
setlocal
cd /d "%~dp0"
set ORCH=http://127.0.0.1:18100
if not "%MESH_ORCH_URL%"=="" set ORCH=%MESH_ORCH_URL%

if exist "%~dp0bin\mesh-gpu-worker.exe" (
  start "MonkeyMesh GPU Worker" "%~dp0bin\mesh-gpu-worker.exe" --orch %ORCH%
  exit /b 0
)
echo mesh-gpu-worker.exe not found. Build with Launchers\build-release.ps1
pause
endlocal
