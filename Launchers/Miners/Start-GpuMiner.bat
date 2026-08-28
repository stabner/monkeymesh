@echo off
REM Jump to the portable GPU miner release pack
cd /d "%~dp0..\..\Releases\Windows\GpuMiner"
if not exist "MonkeyMesh-GpuMiner.exe" (
  echo GPU miner release pack is missing.
  echo Run: Launchers\stage-miner-releases.ps1
  pause
  exit /b 1
)
wscript //nologo "%~dp0..\..\Releases\Windows\GpuMiner\_start-gui.vbs" "MonkeyMesh-GpuMiner.exe"
exit /b 0
