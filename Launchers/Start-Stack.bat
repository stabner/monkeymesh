@echo off
setlocal
cd /d "%~dp0"

echo Starting MonkeyMesh stack...
echo.

if not exist "%~dp0Node\bin\mesh-node.exe" (
  echo Binaries missing. Run build-release.ps1 first.
  pause
  exit /b 1
)

echo NOTE: PoMC 40/40/20 changed genesis — wipe Node\data\chain.bin if upgrading.
echo.

start "MonkeyMesh Node" "%~dp0Node\Start-Node.bat"
timeout /t 4 /nobreak >nul

if exist "%~dp0Orchestrator\bin\mesh-orchestrator.exe" (
  start "MonkeyMesh Orchestrator" "%~dp0Orchestrator\Start-Orchestrator.bat"
  timeout /t 2 /nobreak >nul
)

start "MonkeyMesh Wallet" "%~dp0Wallet\Start-Wallet.bat"

echo.
echo Launched: Node, Orchestrator (if staged), Wallet
echo GPU AI worker: Launchers\Orchestrator\bin\mesh-gpu-worker.exe
echo Explorer:     http://127.0.0.1:18080/
echo Marketplace:  http://127.0.0.1:18100/marketplace
echo.
endlocal
