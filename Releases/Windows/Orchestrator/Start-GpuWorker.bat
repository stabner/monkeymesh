@echo off
cd /d "%~dp0"
set "PATH=%~dp0;%PATH%"
if not defined MESH_ORCH set "MESH_ORCH=http://seednode.hashmonkeys.cloud:18080"
echo MonkeyMesh GPU / AI Worker (protocol + MNIST jobs via node)
echo   orch/node : %MESH_ORCH%
echo   (override MESH_ORCH only if you run a local node)
mesh-gpu-worker.exe --orch %MESH_ORCH% --jobs 0 --poll-ms 400 --keyfile data\gpu-worker.key
if errorlevel 1 pause
