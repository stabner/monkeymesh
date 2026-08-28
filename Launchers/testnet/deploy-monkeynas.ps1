# Deploy MonkeyMesh testnet to the seed host (SSH). Set MESH_NAS_HOST=user@host.
# SAFETY: only restarts mesh-* user systemd units. Never touches miningcore / cereblix / other pools.
#
# Usage:
#   .\Launchers\testnet\deploy-monkeynas.ps1              # default: NO wipe
#   .\Launchers\testnet\deploy-monkeynas.ps1 -Wipe        # delete chain + brains (explicit)
#
param(
    # DANGEROUS: deletes public tip + brains. Requires MESH_ALLOW_WIPE=1 and typed confirmation.
    [switch]$Wipe,
    [string]$WipeConfirm = ""
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$HostName = $env:MESH_NAS_HOST
if ([string]::IsNullOrWhiteSpace($HostName)) {
    throw "Set MESH_NAS_HOST=user@seed-host (not stored in git)."
}
$RemoteSrc = "~/src/MonkeyMesh"

Write-Host "==> SAFETY: will only restart mesh-node / mesh-orchestrator / mesh-gpu-worker"
if ($Wipe) {
    if ($env:MESH_ALLOW_WIPE -ne "1") {
        throw @"
REFUSED: -Wipe blocked (Build/28 M2).
Public tip must not be casually deleted.
To wipe intentionally set `$env:MESH_ALLOW_WIPE='1' and pass -WipeConfirm DELETE_PUBLIC_TIP
"@
    }
    if ($WipeConfirm -ne "DELETE_PUBLIC_TIP") {
        throw "REFUSED: -Wipe requires -WipeConfirm DELETE_PUBLIC_TIP (and MESH_ALLOW_WIPE=1)."
    }
    Write-Host "==> WIPE CONFIRMED: will delete monkeymesh-testnet chain + brains"
} else {
    Write-Host "==> NO WIPE: keeping chain + shared_brain / leg_brains"
}
Write-Host "==> sync source -> ${HostName}:${RemoteSrc}"
ssh $HostName "mkdir -p $RemoteSrc ~/monkeymesh-testnet/{bin,data,logs}"

$tmp = Join-Path $env:TEMP "monkeymesh-src.tar"
if (Test-Path $tmp) { Remove-Item $tmp -Force }
Push-Location $Root
try {
    & tar.exe -cf $tmp `
      Cargo.toml Cargo.lock README.md `
      crates apps Launchers Build assets 2>$null
} finally {
    Pop-Location
}
scp $tmp "${HostName}:~/monkeymesh-src.tar"
# Avoid sticky `target/` leftovers blocking rm -rf of the source tree.
ssh $HostName "rm -rf $RemoteSrc/target 2>/dev/null; rm -rf $RemoteSrc; mkdir -p $RemoteSrc && tar -xf ~/monkeymesh-src.tar -C $RemoteSrc && test -f $RemoteSrc/Launchers/testnet/mesh-testnet.sh"

$wipeBlock = if ($Wipe) {
    @"
systemctl --user stop mesh-node.service mesh-edge.service mesh-orchestrator.service mesh-gpu-worker.service || true
rm -f `$HOME/monkeymesh-testnet/data/chain.bin `$HOME/monkeymesh-testnet/data/chain.bin.tmp
rm -f `$HOME/monkeymesh-testnet/data/chain.blocks.wal `$HOME/monkeymesh-testnet/data/chain.meta.bin
rm -f `$HOME/monkeymesh-testnet/data/chain.snap.json `$HOME/monkeymesh-testnet/data/chain.bin.monolithic-bak
rm -f `$HOME/monkeymesh-testnet/data/chain.utxo.ckpt `$HOME/monkeymesh-testnet/data/chain.utxo.ckpt.tmp
rm -f `$HOME/monkeymesh-testnet/data/shared_brain.bin
rm -f `$HOME/monkeymesh-testnet/data/shared_brain_v2.bin
rm -f `$HOME/monkeymesh-testnet/data/leg_brains.bin
rm -f `$HOME/monkeymesh-testnet/data/quantum_brains.bin
rm -f `$HOME/monkeymesh-testnet/data/pool_credits.json `$HOME/monkeymesh-testnet/data/pool_blocks.json
rm -f `$HOME/monkeymesh-testnet/data/*.wal 2>/dev/null || true
rm -f `$HOME/monkeymesh-testnet/data/ai_inbound.cursor `$HOME/monkeymesh-testnet/data/ai_queue.snap 2>/dev/null || true
rm -rf `$HOME/monkeymesh-testnet/edge
ls -la `$HOME/monkeymesh-testnet/data || true
"@
} else {
    @"
systemctl --user stop mesh-node.service mesh-edge.service mesh-orchestrator.service mesh-gpu-worker.service || true
# Clear AI queue snaps only (not chain / brains) — stale pending blocks new research ticks.
rm -f `$HOME/monkeymesh-testnet/data/ai_inbound.wal `$HOME/monkeymesh-testnet/data/ai_inbound.cursor `$HOME/monkeymesh-testnet/data/ai_queue.snap 2>/dev/null || true
rm -f `$HOME/monkeymesh-testnet/edge/ai_inbound.wal `$HOME/monkeymesh-testnet/edge/ai_inbound.cursor `$HOME/monkeymesh-testnet/edge/ai_queue.snap 2>/dev/null || true
ls -la `$HOME/monkeymesh-testnet/data || true
"@
}

Write-Host "==> build + install mesh units only (other pools untouched)"
ssh $HostName @"
set -e
source `$HOME/.cargo/env
chmod +x `$HOME/src/MonkeyMesh/Launchers/testnet/mesh-testnet.sh
export MESH_SRC=`$HOME/src/MonkeyMesh
export MESH_TESTNET_ROOT=`$HOME/monkeymesh-testnet
export MESH_BIND_IP=0.0.0.0
$(if ($env:MESH_LAN_IP) { "export MESH_LAN_IP=$($env:MESH_LAN_IP)" } else { "" })
export MESH_SEED_MINE=0
$wipeBlock
`$HOME/src/MonkeyMesh/Launchers/testnet/mesh-testnet.sh build
`$HOME/src/MonkeyMesh/Launchers/testnet/mesh-testnet.sh install
systemctl --user daemon-reload || true
systemctl --user restart mesh-node.service
sleep 3
systemctl --user restart mesh-edge.service
sleep 2
systemctl --user restart mesh-pool.service
sleep 2
# Local AI worker/orch optional — keep NAS cooler by default
systemctl --user stop mesh-orchestrator.service mesh-gpu-worker.service 2>/dev/null || true
systemctl --user disable mesh-orchestrator.service mesh-gpu-worker.service 2>/dev/null || true
sleep 1
systemctl --user --no-pager is-active mesh-node mesh-edge mesh-pool
systemctl --user --no-pager is-active mesh-orchestrator mesh-gpu-worker || true
systemctl is-active miningcore-crb.service cereblixd.service || true
# Edge keeps its own datadir; copy seed tip so :18081 cannot stay on a pre-wipe fork.
chmod +x `$HOME/src/MonkeyMesh/Launchers/testnet/realign-edge-from-seed.sh
`$HOME/src/MonkeyMesh/Launchers/testnet/realign-edge-from-seed.sh || true
curl -fsS http://127.0.0.1:18080/v1/getnodeinfo | head -c 320; echo
curl -fsS http://127.0.0.1:18081/v1/getnodeinfo | head -c 320; echo
curl -fsS http://127.0.0.1:12500/health; echo
curl -fsS http://127.0.0.1:18080/v1/ai/health | head -c 320; echo
curl -fsS http://127.0.0.1:18080/v1/trilemma | head -c 400; echo
curl -fsS http://127.0.0.1:18080/v1/quantum | head -c 500; echo
"@

Write-Host ""
Write-Host "Explorer:    http://seednode.hashmonkeys.cloud:18080/"
Write-Host "Edge RPC:    http://seednode.hashmonkeys.cloud:18081/  (templates/submit)"
Write-Host "Pool:        https://eu.hashmonkeys.cloud"
Write-Host "AI board:    http://seednode.hashmonkeys.cloud:18080/v1/ai/health"
Write-Host "Trilemma:    http://seednode.hashmonkeys.cloud:18080/v1/trilemma"
Write-Host "Quantum:     http://seednode.hashmonkeys.cloud:18080/v1/quantum"
Write-Host "Seed mine:   OFF (MESH_SEED_MINE=0). Local gpu-worker/orch: OFF."
Write-Host "Wipe only with -Wipe."
Write-Host "Done (miningcore/cereblix left running)."
