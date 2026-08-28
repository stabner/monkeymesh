@echo off
REM GPU-only pack retired — use the Miner GUI (CPU + NVIDIA + AMD)
cd /d "%~dp0..\..\Releases\Windows\Miner"
if not exist "MonkeyMesh-Miner.exe" (
  echo Miner pack is missing.
  echo Run: Launchers\stage-miner-releases.ps1
  pause
  exit /b 1
)
wscript //nologo "%~dp0..\..\Releases\Windows\Miner\_start-gui.vbs" "MonkeyMesh-Miner.exe"
exit /b 0
