# Sync soft-AI brain bins from monkeynas seed → hashserver (Build/28 M3 cold standby).
# Usage: .\Launchers\testnet\sync-brains-to-hashserver.ps1
param(
    [string]$SeedHost = $(if ($env:MESH_SEED_SSH) { $env:MESH_SEED_SSH } elseif ($env:MESH_NAS_HOST) { $env:MESH_NAS_HOST } else { "" }),
    [string]$HashHost = $(if ($env:MESH_HASHSERVER_HOST) { $env:MESH_HASHSERVER_HOST } else { "hashserver" })
)
$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($SeedHost)) {
    throw "Set MESH_SEED_SSH or MESH_NAS_HOST (not stored in git)."
}
$files = @(
    "shared_brain.bin",
    "shared_brain_v2.bin",
    "leg_brains.bin",
    "quantum_brains.bin",
    "ai.token"
)
$tmp = Join-Path $env:TEMP "mesh-brain-sync"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
Write-Host "==> pull brains from $SeedHost"
foreach ($f in $files) {
    scp "${SeedHost}:~/monkeymesh-testnet/data/$f" (Join-Path $tmp $f)
}
Write-Host "==> push brains to ${HashHost}:~/monkeymesh-edge2/data"
ssh $HashHost "mkdir -p ~/monkeymesh-edge2/data"
foreach ($f in $files) {
    scp (Join-Path $tmp $f) "${HashHost}:~/monkeymesh-edge2/data/$f"
    Write-Host ("    ok {0}" -f $f)
}
Write-Host "Done. Cold standby brains on hashserver (edge2 does not load them until seed promotion)."
