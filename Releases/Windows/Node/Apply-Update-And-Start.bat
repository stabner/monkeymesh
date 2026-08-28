@echo off
cd /d "%~dp0"
echo Stopping MonkeyMesh Node / mesh-node if running...
taskkill /IM MonkeyMesh-Node.exe /F >nul 2>&1
taskkill /IM mesh-node.exe /F >nul 2>&1
timeout /t 2 /nobreak >nul
if exist "MonkeyMesh-Node.exe.new" (
  move /Y "MonkeyMesh-Node.exe.new" "MonkeyMesh-Node.exe" >nul
  echo Updated MonkeyMesh-Node.exe
)
echo.
echo IMPORTANT: Your local chain height is ahead of the public seed with 0 peers.
echo That usually means Auto-mine built a private tip. To follow the public testnet:
echo   1. Keep wallet.key / p2p.key
echo   2. Delete data\chain.*  (or the whole data folder except *.key and ai.token)
echo   3. Start with Auto-mine OFF
echo.
pause
start "" "%~dp0MonkeyMesh-Node.exe"
