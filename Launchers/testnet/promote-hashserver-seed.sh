#!/usr/bin/env bash
# EMERGENCY: promote edge2 → temporary seed (AI board local) using cold-standby brains.
# Use only if the public seed is down. Leaves HTN/monkeypool alone.
#
# Usage (on hashserver):
#   ~/src/MonkeyMesh/Launchers/testnet/promote-hashserver-seed.sh
set -euo pipefail
ROOT="${MESH_TESTNET_ROOT:-$HOME/monkeymesh-edge2}"
DATA="$ROOT/data"
BIN="$ROOT/bin"
UNIT_DIR="$HOME/.config/systemd/user"

if [[ ! -x "$BIN/mesh-node" ]]; then
  echo "missing $BIN/mesh-node" >&2
  exit 1
fi
if [[ ! -f "$DATA/shared_brain.bin" ]]; then
  echo "missing cold brains — run sync-brains-from-seed.sh first while seed is up" >&2
  exit 1
fi

systemctl --user stop mesh-edge2.service || true
mkdir -p "$UNIT_DIR"
cat >"$UNIT_DIR/mesh-seed-failover.service" <<EOF
[Unit]
Description=MonkeyMesh TEMP seed failover on edge2 (AI board local)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=RUST_LOG=info,mesh_p2p=warn
Environment=MESH_ADVERTISE_HOST=${MESH_ADVERTISE_HOST:-seednode.hashmonkeys.cloud}
Environment=MESH_AI_SHARD_ID=0
Environment=MESH_AI_SHARD_COUNT=3
Environment=MESH_RPC_EDGES=${MESH_RPC_EDGES:-http://seednode.hashmonkeys.cloud:18081}
$( [[ -f $DATA/ai.token ]] && echo "Environment=MESH_AI_TOKEN=$(tr -d '\n\r' < "$DATA/ai.token")" )
$( [[ -f $DATA/rpc.token ]] && echo "Environment=MESH_RPC_TOKEN=$(tr -d '\n\r' < "$DATA/rpc.token")" )
ExecStart=$BIN/mesh-node --chain $DATA/chain.bin serve --listen 0.0.0.0:39002 --rpc 0.0.0.0:18083 --wallet $DATA/wallet.key --p2p-key $DATA/p2p.key --miner-key $DATA/wallet.key
Restart=on-failure
RestartSec=3
StandardOutput=append:$ROOT/logs/seed-failover.log
StandardError=append:$ROOT/logs/seed-failover.log

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user disable --now mesh-edge2.service 2>/dev/null || true
systemctl --user enable --now mesh-seed-failover.service
sleep 3
systemctl --user is-active mesh-seed-failover.service
curl -fsS -m 10 http://127.0.0.1:18083/v1/getnodeinfo | head -c 240
echo
curl -fsS -m 10 http://127.0.0.1:18083/v1/ai/health | head -c 240 || true
echo
echo "FAILOVER ACTIVE on :18083 / P2P :39002 — point clients here until monkeynas recovers."
echo "To revert: systemctl --user disable --now mesh-seed-failover; systemctl --user enable --now mesh-edge2"
