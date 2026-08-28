@echo off
cd /d "%~dp0"
set "PATH=%~dp0;%PATH%"
if not defined MESH_ORCH_BIND set "MESH_ORCH_BIND=127.0.0.1:18100"
if not defined MESH_NODE_RPC set "MESH_NODE_RPC=http://127.0.0.1:18080"
if not defined MESH_ORCH_REQUIRE_NODE set "MESH_ORCH_REQUIRE_NODE=1"
echo MonkeyMesh Orchestrator
echo   bind : %MESH_ORCH_BIND%
echo   node : %MESH_NODE_RPC%
mesh-orchestrator.exe
if errorlevel 1 pause
