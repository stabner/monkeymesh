@echo off
setlocal EnableExtensions
cd /d "%~dp0"
set "PATH=%~dp0;%PATH%"

if not exist "MonkeyMesh-CpuMiner.exe" (
  echo Missing MonkeyMesh-CpuMiner.exe in this folder.
  pause
  exit /b 1
)
if not exist "config.json" (
  echo Missing config.json - set your payout "address"
  pause
  exit /b 1
)
if not exist "vcruntime140.dll" (
  echo Missing vcruntime140.dll - this release pack is incomplete.
  echo Re-run: Launchers\stage-miner-releases.ps1
  pause
  exit /b 1
)

for /f "usebackq delims=" %%A in (`powershell -NoProfile -Command "$c=Get-Content -Raw '.\config.json'|ConvertFrom-Json; if (-not $c.address) { Write-Error 'config.json address is empty'; exit 1 }; Write-Output ($c.rpc.TrimEnd('/') + '|' + $c.address.Trim() + '|' + [string]$c.blocks + '|' + [string]$c.max_nonces)"`) do set "CFG=%%A"
if errorlevel 1 (
  echo Edit config.json and set "address" to your wallet address.
  pause
  exit /b 1
)

for /f "tokens=1-4 delims=|" %%a in ("%CFG%") do (
  set "RPC=%%a"
  set "ADDR=%%b"
  set "BLOCKS=%%c"
  set "NONCES=%%d"
)

echo MonkeyMesh CPU Miner
echo   folder : %CD%
echo   rpc    : %RPC%
echo   address: %ADDR%
echo   blocks : %BLOCKS%  (0 = until Ctrl+C)
echo.
MonkeyMesh-CpuMiner.exe --rpc "%RPC%" --address "%ADDR%" --blocks %BLOCKS% --max-nonces %NONCES%
set ERR=%ERRORLEVEL%
echo.
if not %ERR%==0 pause
exit /b %ERR%
