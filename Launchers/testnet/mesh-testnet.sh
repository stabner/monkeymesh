#!/usr/bin/env bash
# MonkeyMesh testnet control script (run on the seed host).
set -euo pipefail

ROOT="${MESH_TESTNET_ROOT:-$HOME/monkeymesh-testnet}"
SRC="${MESH_SRC:-$HOME/src/MonkeyMesh}"
BIN="$ROOT/bin"
DATA="$ROOT/data"
EDGE_DATA="$ROOT/edge"
LOG="$ROOT/logs"

SEED_DNS="${MESH_SEED_DNS:-seednode.hashmonkeys.cloud}"
# Optional operator bind/advertise override. Public advertisement stays on SEED_DNS.
LAN_IP="${MESH_LAN_IP:-}"
BIND_IP="${MESH_BIND_IP:-0.0.0.0}"
P2P_PORT="${MESH_P2P_PORT:-39001}"
RPC_PORT="${MESH_RPC_PORT:-18080}"
ORCH_PORT="${MESH_ORCH_PORT:-18100}"
EDGE_P2P_PORT="${MESH_EDGE_P2P_PORT:-39002}"
EDGE_RPC_PORT="${MESH_EDGE_RPC_PORT:-18081}"
if [ -n "$LAN_IP" ]; then
  EDGE_RPC_URL="http://${LAN_IP}:${EDGE_RPC_PORT}"
else
  EDGE_RPC_URL="http://${SEED_DNS}:${EDGE_RPC_PORT}"
fi
HASHSERVER_EDGE_RPC="${MESH_HASHSERVER_EDGE_RPC:-http://${SEED_DNS}:18083}"
EDGE_RPC_URLS="${EDGE_RPC_URL},http://${SEED_DNS}:${EDGE_RPC_PORT},${HASHSERVER_EDGE_RPC}"
# Seed + NAS edge only. Hashserver :18083 is a mine edge, not a public AI board
# (WAN :18083 is often closed and made miners look like they "pulsed").
AI_SHARDS_MAP="0=http://${SEED_DNS}:${RPC_PORT},1=http://${SEED_DNS}:${EDGE_RPC_PORT}"
AI_SHARD_COUNT="${MESH_AI_SHARD_COUNT:-2}"
# Seed should relay + host AI board; mining steals CPU from verify under load.
# Set MESH_SEED_MINE=1 only for tiny solo labs.
SEED_MINE="${MESH_SEED_MINE:-0}"
# Empty = coinbase pays the miner’s ?address= / X-Mesh-Miner.
# Set MESH_POOL_PAYOUT=mesh01… only if you want every find forced to one wallet.
MESH_POOL_PAYOUT="${MESH_POOL_PAYOUT:-}"
# Seed/edge node-market credits (useful work only).
MESH_OPERATOR_ADDRESS="${MESH_OPERATOR_ADDRESS:-}"
OP_ADDR_FLAG=""
if [[ -n "$MESH_OPERATOR_ADDRESS" ]]; then
  OP_ADDR_FLAG="--operator-address $MESH_OPERATOR_ADDRESS"
fi
MINE_FLAG=""
if [[ "$SEED_MINE" == "1" || "$SEED_MINE" == "true" ]]; then
  MINE_FLAG="--mine"
fi

mkdir -p "$BIN" "$DATA" "$EDGE_DATA" "$LOG"

export PATH="$HOME/.cargo/bin:$PATH"
export RUST_LOG="${RUST_LOG:-info}"

usage() {
  cat <<EOF
Usage: $(basename "$0") {build|install|start|stop|status|restart|wipe-chain}
  build       cargo release build in \$MESH_SRC
  install     copy bins + write systemd user units (seed + edge)
  start|stop|restart|status
  wipe-chain  stop nodes and delete chain stores (PoMC genesis changes)
EOF
}

do_build() {
  cd "$SRC"
  cargo build --release \
    -p mesh-node \
    -p mesh-orchestrator \
    -p mesh-gpu-worker \
    -p mesh-miner-cpu \
    -p mesh-miner-gpu \
    -p mesh-pool
  mkdir -p "$BIN"
  cp -f target/release/mesh-node \
        target/release/mesh-orchestrator \
        target/release/mesh-gpu-worker \
        target/release/mesh-miner-cpu \
        target/release/mesh-miner-gpu \
        target/release/mesh-pool \
        "$BIN/"
  echo "bins -> $BIN"
}

write_units() {
  local unit_dir="$HOME/.config/systemd/user"
  mkdir -p "$unit_dir"

  # Optional default AI board token (Build/27 B4). Sticky file; set MESH_AI_TOKEN_AUTO=0 to keep open.
  local AI_TOKEN_LINE=""
  local AI_TOKEN_VAL=""
  if [[ "${MESH_AI_TOKEN_AUTO:-1}" == "1" || "${MESH_AI_TOKEN_AUTO}" == "true" ]]; then
    if [[ -n "${MESH_AI_TOKEN:-}" ]]; then
      AI_TOKEN_VAL="$MESH_AI_TOKEN"
    else
      if [[ ! -f "$DATA/ai.token" ]]; then
        if command -v openssl >/dev/null 2>&1; then
          openssl rand -hex 24 >"$DATA/ai.token"
        else
          head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$DATA/ai.token"
        fi
        chmod 600 "$DATA/ai.token"
        echo "generated AI board token -> $DATA/ai.token"
      fi
      AI_TOKEN_VAL="$(tr -d '\n\r' <"$DATA/ai.token")"
    fi
    AI_TOKEN_LINE="Environment=MESH_AI_TOKEN=${AI_TOKEN_VAL}"
    echo "mesh-node/gpu-worker: MESH_AI_TOKEN armed (MESH_AI_TOKEN_AUTO=1)"
  else
    echo "mesh-node: AI board open (MESH_AI_TOKEN_AUTO=0)"
  fi

  # Bitcoin Core–style wallet RPC cookie. Wallet/gov routes fail closed.
  local RPC_TOKEN_LINE=""
  local RPC_TOKEN_VAL=""
  if [[ -n "${MESH_RPC_TOKEN:-}" ]]; then
    RPC_TOKEN_VAL="$MESH_RPC_TOKEN"
  else
    if [[ ! -f "$DATA/rpc.token" ]]; then
      if command -v openssl >/dev/null 2>&1; then
        openssl rand -hex 32 >"$DATA/rpc.token"
      else
        head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$DATA/rpc.token"
      fi
      chmod 600 "$DATA/rpc.token"
      echo "generated wallet RPC cookie -> $DATA/rpc.token"
    fi
    RPC_TOKEN_VAL="$(tr -d '\n\r' <"$DATA/rpc.token")"
  fi
  RPC_TOKEN_LINE="Environment=MESH_RPC_TOKEN=${RPC_TOKEN_VAL}"
  echo "mesh-node: MESH_RPC_TOKEN armed (fail-closed wallet/gov RPC)"

  local EDGE_RPC_TOKEN_LINE=""
  if [[ ! -f "$EDGE_DATA/rpc.token" ]]; then
    if command -v openssl >/dev/null 2>&1; then
      openssl rand -hex 32 >"$EDGE_DATA/rpc.token"
    else
      head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$EDGE_DATA/rpc.token"
    fi
    chmod 600 "$EDGE_DATA/rpc.token"
  fi
  EDGE_RPC_TOKEN_LINE="Environment=MESH_RPC_TOKEN=$(tr -d '\n\r' <"$EDGE_DATA/rpc.token")"

  cat >"$unit_dir/mesh-node.service" <<EOF
[Unit]
Description=MonkeyMesh official seed node (seednode.hashmonkeys.cloud)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=RUST_LOG=info,mesh_p2p=warn
Environment=MESH_ADVERTISE_HOST=${LAN_IP}
Environment=MESH_RPC_EDGES=${EDGE_RPC_URLS}
Environment=MESH_AI_SHARD_ID=0
Environment=MESH_AI_SHARD_COUNT=${AI_SHARD_COUNT}
Environment=MESH_AI_SHARDS=${AI_SHARDS_MAP}
${AI_TOKEN_LINE}
${RPC_TOKEN_LINE}
Environment=MESH_BRAIN_VERIFY_MAX_STEPS=${MESH_BRAIN_VERIFY_MAX_STEPS:-512}
Environment=MESH_BRAIN_VERIFY_MAX_SAMPLES=${MESH_BRAIN_VERIFY_MAX_SAMPLES:-1024}
Environment=MESH_BRAIN_VERIFY_BATCH_MAX=${MESH_BRAIN_VERIFY_BATCH_MAX:-4}
Environment=MESH_POW_V2_HEIGHT=${MESH_POW_V2_HEIGHT:-53000}
Environment=MESH_POW_EVO_HEIGHT=${MESH_POW_EVO_HEIGHT:-1}
Environment=MESH_POW_FUSION_HEIGHT=${MESH_POW_FUSION_HEIGHT:-80}
# F2 stays off (Build/36). Do not export MESH_FINALITY_HEIGHT on this host.
Environment=MESH_AI_GLOBAL_LIMIT=${MESH_AI_GLOBAL_LIMIT:-800}
Environment=MESH_AI_IP_LIMIT=${MESH_AI_IP_LIMIT:-120}
Environment=MESH_AI_JOB_LIMIT=${MESH_AI_JOB_LIMIT:-30}
Environment=MESH_AI_RES_LIMIT=${MESH_AI_RES_LIMIT:-45}
Environment=MESH_FORCE_RETARGET_INTERVAL=${MESH_FORCE_RETARGET_INTERVAL:-15}
Environment=TOKIO_WORKER_THREADS=${TOKIO_WORKER_THREADS:-6}
ExecStart=$BIN/mesh-node --chain $DATA/chain.bin serve --listen ${BIND_IP}:${P2P_PORT} --rpc ${BIND_IP}:${RPC_PORT} --wallet $DATA/wallet.key --p2p-key $DATA/p2p.key --miner-key $DATA/wallet.key $MINE_FLAG $OP_ADDR_FLAG
Restart=on-failure
RestartSec=3
StandardOutput=append:$LOG/node.log
StandardError=append:$LOG/node.log

[Install]
WantedBy=default.target
EOF
  if [[ -n "$MINE_FLAG" ]]; then
    echo "mesh-node unit: mining ENABLED (MESH_SEED_MINE=$SEED_MINE)"
  else
    echo "mesh-node unit: mining OFF (relay + AI board only; set MESH_SEED_MINE=1 to enable)"
  fi
  echo "mesh-node advertises edge RPC: ${EDGE_RPC_URL}"

  cat >"$unit_dir/mesh-edge.service" <<EOF
[Unit]
Description=MonkeyMesh edge RPC (templates/submit; syncs from seed)
After=network-online.target mesh-node.service
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=RUST_LOG=info,mesh_p2p=warn
Environment=MESH_EDGE_MODE=1
Environment=MESH_ADVERTISE_HOST=${LAN_IP}
Environment=MESH_AI_UPSTREAM=http://127.0.0.1:${RPC_PORT}
Environment=MESH_EDGE_AI_LOCAL=${MESH_EDGE_AI_LOCAL:-0}
Environment=MESH_AI_SHARD_ID=1
Environment=MESH_AI_SHARD_COUNT=${AI_SHARD_COUNT}
Environment=MESH_AI_SHARDS=${AI_SHARDS_MAP}
${AI_TOKEN_LINE}
${EDGE_RPC_TOKEN_LINE}
Environment=MESH_POW_V2_HEIGHT=${MESH_POW_V2_HEIGHT:-53000}
Environment=MESH_POW_EVO_HEIGHT=${MESH_POW_EVO_HEIGHT:-1}
Environment=MESH_POW_FUSION_HEIGHT=${MESH_POW_FUSION_HEIGHT:-80}
# F2 stays off (Build/36). Do not export MESH_FINALITY_HEIGHT on this host.
Environment=MESH_FORCE_RETARGET_INTERVAL=${MESH_FORCE_RETARGET_INTERVAL:-15}
ExecStart=$BIN/mesh-node --chain $EDGE_DATA/chain.bin serve --listen ${BIND_IP}:${EDGE_P2P_PORT} --connect 127.0.0.1:${P2P_PORT} --rpc ${BIND_IP}:${EDGE_RPC_PORT} --wallet $EDGE_DATA/wallet.key --p2p-key $EDGE_DATA/p2p.key --miner-key $EDGE_DATA/wallet.key $OP_ADDR_FLAG
Restart=on-failure
RestartSec=3
StandardOutput=append:$LOG/edge.log
StandardError=append:$LOG/edge.log

[Install]
WantedBy=default.target
EOF
  echo "mesh-edge unit: P2P :${EDGE_P2P_PORT} RPC :${EDGE_RPC_PORT} (MESH_EDGE_MODE=1)"

  cat >"$unit_dir/mesh-orchestrator.service" <<EOF
[Unit]
Description=MonkeyMesh AI orchestrator + marketplace
After=mesh-node.service
Wants=mesh-node.service

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=RUST_LOG=info
Environment=MESH_ORCH_BIND=${BIND_IP}:${ORCH_PORT}
Environment=MESH_NODE_RPC=http://127.0.0.1:${RPC_PORT}
Environment=MESH_ORCH_REQUIRE_NODE=1
Environment=MESH_SETTLE=1
Environment=MESH_SETTLE_BASE_ATOMIC=100000
${AI_TOKEN_LINE}
${RPC_TOKEN_LINE}
ExecStart=$BIN/mesh-orchestrator
Restart=on-failure
RestartSec=3
StandardOutput=append:$LOG/orch.log
StandardError=append:$LOG/orch.log

[Install]
WantedBy=default.target
EOF

  cat >"$unit_dir/mesh-gpu-worker.service" <<EOF
[Unit]
Description=MonkeyMesh GPU AI worker (talks to node embedded AI board)
After=mesh-node.service
Wants=mesh-node.service

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=RUST_LOG=info
${AI_TOKEN_LINE}
ExecStart=$BIN/mesh-gpu-worker --orch http://127.0.0.1:${RPC_PORT} --keyfile $DATA/gpu-worker.key --poll-ms 500
Restart=on-failure
RestartSec=5
StandardOutput=append:$LOG/worker.log
StandardError=append:$LOG/worker.log

[Install]
WantedBy=default.target
EOF

  # HTTP GBT pool on seed host (public via firewall https://eu.hashmonkeys.cloud → :12500).
  # Upstream = local edge templates/submit so seed AI/verify stays off the hot mine path.
  # Coinbase = miner ?address= / X-Mesh-Miner (not pool.key).
  # MESH_POOL_PAYOUT overrides every template (operator wallet).
  cat >"$unit_dir/mesh-pool.service" <<EOF
[Unit]
Description=MonkeyMesh HTTP GBT pool (:12500 → local edge :${EDGE_RPC_PORT})
After=network-online.target mesh-edge.service
Wants=network-online.target mesh-edge.service

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=RUST_LOG=info
ExecStart=$BIN/mesh-pool --bind ${BIND_IP}:12500 --upstream http://127.0.0.1:${EDGE_RPC_PORT} --keyfile $DATA/pool.key --credits $DATA/pool_credits.json --blocks $DATA/pool_blocks.json ${MESH_POOL_PAYOUT:+--payout-address $MESH_POOL_PAYOUT}
Restart=on-failure
RestartSec=3
StandardOutput=append:$LOG/pool.log
StandardError=append:$LOG/pool.log

[Install]
WantedBy=default.target
EOF
  echo "mesh-pool unit: :12500 → upstream http://127.0.0.1:${EDGE_RPC_PORT}"

  systemctl --user daemon-reload
  # Seed + edge + pool by default — local GPU worker / orch burn NAS CPU; opt in with MESH_SEED_LOCAL_AI=1
  systemctl --user enable mesh-node.service mesh-edge.service mesh-pool.service
  if [[ "${MESH_SEED_LOCAL_AI:-0}" == "1" || "${MESH_SEED_LOCAL_AI:-}" == "true" ]]; then
    systemctl --user enable mesh-orchestrator.service mesh-gpu-worker.service
    echo "systemd user units enabled (seed + edge + pool + local AI worker/orch)"
  else
    systemctl --user disable mesh-orchestrator.service mesh-gpu-worker.service 2>/dev/null || true
    echo "systemd user units enabled (seed + edge + pool; set MESH_SEED_LOCAL_AI=1 for local gpu-worker)"
  fi
}

cmd_install() {
  do_build
  write_units
  cat >"$ROOT/ENDPOINTS.txt" <<EOF
MonkeyMesh official seed @ ${SEED_DNS}
DNS:          ${SEED_DNS}
Explorer:     http://${SEED_DNS}:${RPC_PORT}/
              http://${LAN_IP}:${RPC_PORT}/
RPC:          http://${SEED_DNS}:${RPC_PORT}/v1/getnodeinfo
Edge RPC:     ${EDGE_RPC_URL}/v1/getnodeinfo  (templates/submit; AI on seed)
Pool:         http://${LAN_IP}:12500/  (HTTPS front: https://eu.hashmonkeys.cloud)
Marketplace:  http://${SEED_DNS}:${ORCH_PORT}/marketplace  (legacy standalone orch)
AI board:     http://${SEED_DNS}:${RPC_PORT}/v1/ai/health  (embedded in seed)
P2P seed:     ${SEED_DNS}:${P2P_PORT}
LAN P2P:      ${LAN_IP}:${P2P_PORT}
Edge P2P:     ${LAN_IP}:${EDGE_P2P_PORT}
Research:     http://${LAN_IP}:${RPC_PORT}/v1/research/scenarios
Note: router must forward 18080/tcp 18081/tcp 18100/tcp 39001/udp(+tcp) 39002/udp(+tcp) to ${LAN_IP}
      Public pool uses firewall HTTPS → ${LAN_IP}:12500 (WAN :12500 optional)
Workers use seed or edge RPC (edge proxies AI to seed when MESH_AI_UPSTREAM is set)
AI token:     \$DATA/ai.token (MESH_AI_TOKEN_AUTO=1 default; MESH_AI_TOKEN_AUTO=0 keeps board open)
Services:     http://${SEED_DNS}:${RPC_PORT}/v1/nodeservices
EOF
  cat "$ROOT/ENDPOINTS.txt"
}

cmd_start() {
  systemctl --user start mesh-node.service
  sleep 3
  systemctl --user start mesh-edge.service
  sleep 2
  systemctl --user start mesh-pool.service
  sleep 1
  if [[ "${MESH_SEED_LOCAL_AI:-0}" == "1" || "${MESH_SEED_LOCAL_AI:-}" == "true" ]]; then
    systemctl --user start mesh-orchestrator.service
    sleep 1
    systemctl --user start mesh-gpu-worker.service
  fi
  cmd_status
}

cmd_stop() {
  systemctl --user stop mesh-gpu-worker.service mesh-orchestrator.service mesh-pool.service mesh-edge.service mesh-node.service || true
}

cmd_status() {
  systemctl --user --no-pager status mesh-node.service mesh-edge.service mesh-pool.service mesh-orchestrator.service mesh-gpu-worker.service || true
  echo "---"
  curl -fsS "http://${LAN_IP}:${RPC_PORT}/v1/getnodeinfo" 2>/dev/null | head -c 400 || echo "seed RPC not up yet"
  echo
  curl -fsS "http://${LAN_IP}:${EDGE_RPC_PORT}/v1/getnodeinfo" 2>/dev/null | head -c 400 || echo "edge RPC not up yet"
  echo
  curl -fsS "http://${LAN_IP}:12500/health" 2>/dev/null || echo "pool not up yet"
  echo
  curl -fsS "http://${LAN_IP}:${ORCH_PORT}/v1/health" 2>/dev/null || echo "orch not up yet"
  echo
}

cmd_wipe() {
  cmd_stop
  rm -f "$DATA/chain.bin" "$DATA/chain.bin.tmp"
  rm -f "$DATA/chain.blocks.wal" "$DATA/chain.meta.bin" "$DATA/chain.snap.json"
  rm -f "$DATA/chain.bin.monolithic-bak"
  rm -f "$EDGE_DATA/chain.bin" "$EDGE_DATA/chain.bin.tmp"
  rm -f "$EDGE_DATA/chain.blocks.wal" "$EDGE_DATA/chain.meta.bin" "$EDGE_DATA/chain.snap.json"
  rm -f "$EDGE_DATA/chain.bin.monolithic-bak"
  echo "wiped seed + edge chain stores"
}

case "${1:-}" in
  build) do_build ;;
  install) cmd_install ;;
  start) cmd_start ;;
  stop) cmd_stop ;;
  restart) cmd_stop; sleep 1; cmd_start ;;
  status) cmd_status ;;
  wipe-chain) cmd_wipe ;;
  *) usage; exit 1 ;;
esac
