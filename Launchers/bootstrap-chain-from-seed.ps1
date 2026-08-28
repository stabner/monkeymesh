# Bootstrap local node chain from the seed (fast catch-up).
# Keeps wallet.key / p2p.key. Replaces chain.* only.
#
# Usage (PowerShell, from repo or anywhere):
#   .\Launchers\bootstrap-chain-from-seed.ps1
#   .\Launchers\bootstrap-chain-from-seed.ps1 -NodeData ".\Releases\Windows\Node\data"
#
# Requires: SSH access to monkeynas (same as deploy). Stops local Node briefly.

param(
    [string]$NasHost = $(if ($env:MESH_NAS_HOST) { $env:MESH_NAS_HOST } else { "" }),
    [string]$RemoteData = "~/monkeymesh-testnet/data",
    [string]$NodeData = ""
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($NasHost)) {
    throw "Set MESH_NAS_HOST=user@seed-host (not stored in git)."
}
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
if (-not $NodeData) {
    $NodeData = Join-Path $Root "Releases\Windows\Node\data"
}
if (-not (Test-Path $NodeData)) {
    throw "Node data dir missing: $NodeData"
}

Write-Host "==> Stopping local MonkeyMesh node (if running)"
Get-Process MonkeyMesh-Node, mesh-node -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

Write-Host "==> Pausing seed for consistent chain copy"
ssh $NasHost "systemctl --user stop mesh-node; sleep 1"

try {
    Write-Host "==> Copying chain files from $NasHost"
    Remove-Item -Force (Join-Path $NodeData "chain.blocks.wal"),
        (Join-Path $NodeData "chain.meta.bin"),
        (Join-Path $NodeData "chain.snap.json") -ErrorAction SilentlyContinue
    scp "${NasHost}:${RemoteData}/chain.blocks.wal" $NodeData
    scp "${NasHost}:${RemoteData}/chain.meta.bin" $NodeData
    scp "${NasHost}:${RemoteData}/chain.snap.json" $NodeData
}
finally {
    Write-Host "==> Restarting seed"
    ssh $NasHost "systemctl --user start mesh-node"
}

Write-Host "==> Starting local node"
$start = Join-Path (Split-Path $NodeData -Parent) "Start-Node.bat"
if (Test-Path $start) {
    Start-Process $start -WorkingDirectory (Split-Path $start)
} else {
    Write-Host "Start manually: Releases\Windows\Node\Start-Node.bat"
}
Write-Host "Done. Keys preserved under $NodeData"
