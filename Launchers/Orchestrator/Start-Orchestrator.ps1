# MonkeyMesh AI orchestrator launcher
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$exe = Join-Path $PSScriptRoot "bin\mesh-orchestrator.exe"
if (-not (Test-Path $exe)) {
    Write-Host "Missing bin\mesh-orchestrator.exe — run .\Launchers\build-release.ps1"
    Read-Host "Press Enter to exit"
    exit 1
}

$env:MESH_ORCH_BIND = if ($env:MESH_ORCH_BIND) { $env:MESH_ORCH_BIND } else { "127.0.0.1:18100" }
$env:MESH_NODE_RPC = if ($env:MESH_NODE_RPC) { $env:MESH_NODE_RPC } else { "http://127.0.0.1:18080" }

Write-Host "MonkeyMesh Orchestrator"
Write-Host "  bind : $($env:MESH_ORCH_BIND)"
Write-Host "  node : $($env:MESH_NODE_RPC)"
Write-Host ""
& $exe
