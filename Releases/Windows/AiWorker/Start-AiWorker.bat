@echo off
cd /d "%~dp0"
set "PATH=%~dp0;%PATH%"
if not defined MESH_ORCH set "MESH_ORCH=http://seednode.hashmonkeys.cloud:18080"
echo.
echo  MonkeyMesh AI Worker
echo  ====================
echo  Pulls blockchain self-improvement + MNIST jobs from the seed NODE.
echo  This is the GPU-market worker ??? not the PoW block miner.
echo.
echo  orch/node : %MESH_ORCH%
echo  payout key: data\gpu-worker.key  (created on first run)
echo.
echo  Press Ctrl+C to stop.
echo.
mesh-gpu-worker.exe --orch %MESH_ORCH% --jobs 0 --poll-ms 400 --keyfile data\gpu-worker.key
echo.
echo Worker exited.
pause
