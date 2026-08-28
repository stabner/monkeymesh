@echo off
REM Jump to the portable CPU miner release pack
cd /d "%~dp0..\..\Releases\CpuMiner"
if not exist "MonkeyMesh-CpuMiner.exe" (
  echo CPU miner release pack is missing.
  echo Run: Launchers\stage-miner-releases.ps1
  pause
  exit /b 1
)
call Start-CpuMiner.bat
exit /b %ERRORLEVEL%
