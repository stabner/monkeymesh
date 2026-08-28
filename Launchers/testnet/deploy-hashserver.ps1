# Deploy MonkeyMesh edge2 to hashserver (SSH). Leaves HTN/monkeypool alone.
#
# Usage:
#   .\Launchers\testnet\deploy-hashserver.ps1
#
param()

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$HostName = if ($env:MESH_HASHSERVER_HOST) { $env:MESH_HASHSERVER_HOST } else { "hashserver" }
$RemoteSrc = "~/src/MonkeyMesh"

Write-Host "==> SAFETY: only install/restart mesh-edge2 on hashserver (no HTN/monkeypool)"
Write-Host "==> sync source -> ${HostName}:${RemoteSrc}"
ssh $HostName "mkdir -p $RemoteSrc ~/monkeymesh-edge2/{bin,data,logs}"

$tmp = Join-Path $env:TEMP "monkeymesh-src-hs.tar"
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
ssh $HostName "rm -rf $RemoteSrc/target 2>/dev/null; rm -rf $RemoteSrc; mkdir -p $RemoteSrc && tar -xf ~/monkeymesh-src.tar -C $RemoteSrc && test -f $RemoteSrc/Launchers/testnet/mesh-edge-remote.sh"

Write-Host "==> hop mesh-node NAS -> this PC -> hashserver (hashserver has no NAS key)"
$NasHost = $env:MESH_NAS_HOST
if ([string]::IsNullOrWhiteSpace($NasHost)) {
    throw "Set MESH_NAS_HOST=user@seed-host (not stored in git)."
}
$nodeTmp = Join-Path $env:TEMP "mesh-node-nas"
ssh $HostName "systemctl --user stop mesh-edge2.service || true"
scp "${NasHost}:~/monkeymesh-testnet/bin/mesh-node" $nodeTmp
if (-not (Test-Path $nodeTmp)) { throw "failed to pull mesh-node from $NasHost" }
scp $nodeTmp "${HostName}:~/monkeymesh-edge2/bin/mesh-node"
Remove-Item -Force $nodeTmp -ErrorAction SilentlyContinue

Write-Host "==> install edge2 unit (prebuilt mesh-node already in place)"
ssh $HostName @"
set -e
mkdir -p `$HOME/monkeymesh-edge2/{bin,data,logs}
chmod +x `$HOME/monkeymesh-edge2/bin/mesh-node
echo 'mesh-node from monkeynas (via deploy hop)'
if ssh -o BatchMode=yes -o ConnectTimeout=5 $NasHost 'cat ~/monkeymesh-testnet/data/ai.token' > `$HOME/monkeymesh-edge2/data/ai.token 2>/dev/null; then
  echo 'ai.token from seed'
else
  echo 'warn: ai.token fetch failed'
fi
# Cold-standby brains (M3) — not loaded in EDGE mode; used by promote-hashserver-seed.sh
chmod +x `$HOME/src/MonkeyMesh/Launchers/testnet/mesh-edge-remote.sh `$HOME/src/MonkeyMesh/Launchers/testnet/sync-brains-from-seed.sh 2>/dev/null || true
`$HOME/src/MonkeyMesh/Launchers/testnet/sync-brains-from-seed.sh || echo 'warn: brain sync failed'
# Hourly brain standby timer
mkdir -p `$HOME/.config/systemd/user
cat >`$HOME/.config/systemd/user/mesh-brain-sync.service <<'UNIT'
[Unit]
Description=MonkeyMesh cold brain sync from monkeynas seed
After=network-online.target

[Service]
Type=oneshot
ExecStart=%h/src/MonkeyMesh/Launchers/testnet/sync-brains-from-seed.sh
UNIT
cat >`$HOME/.config/systemd/user/mesh-brain-sync.timer <<'UNIT'
[Unit]
Description=Hourly MonkeyMesh brain cold standby sync

[Timer]
OnBootSec=2min
OnUnitActiveSec=1h
Persistent=true

[Install]
WantedBy=timers.target
UNIT
systemctl --user daemon-reload || true
systemctl --user enable --now mesh-brain-sync.timer || true
export MESH_SRC=`$HOME/src/MonkeyMesh
export MESH_TESTNET_ROOT=`$HOME/monkeymesh-edge2
export MESH_BIND_IP=0.0.0.0
export MESH_SEED_HOST=`${MESH_SEED_HOST:-seednode.hashmonkeys.cloud}
$(if ($env:MESH_LAN_IP) { "export MESH_LAN_IP=$($env:MESH_LAN_IP)" } else { "" })
export MESH_EDGE_P2P_PORT=39002
# Only build if binary missing
if [[ ! -x `$HOME/monkeymesh-edge2/bin/mesh-node ]]; then
  `$HOME/src/MonkeyMesh/Launchers/testnet/mesh-edge-remote.sh build
fi
`$HOME/src/MonkeyMesh/Launchers/testnet/mesh-edge-remote.sh install
systemctl --user daemon-reload || true
systemctl --user restart mesh-edge2.service
systemctl --user disable --now mesh-pool.service 2>/dev/null || true
sleep 5
systemctl --user is-active mesh-edge2.service
curl -fsS http://127.0.0.1:18083/v1/getnodeinfo | head -c 500 || true
echo
"@

Write-Host "Done. Edge2 P2P :39002. Pool: https://eu.hashmonkeys.cloud (HTN untouched)."
Write-Host "Brains: cold standby synced + hourly timer on edge2 (Build/28 M3)."
