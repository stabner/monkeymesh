#!/usr/bin/env bash
# MonkeyMesh remote edge (second mine RPC + P2P peer).
# Does NOT touch HTN monkeypool / miningcore / HTND.
set -euo pipefail

ROOT="${MESH_TESTNET_ROOT:-$HOME/monkeymesh-edge2}"
SRC="${MESH_SRC:-$HOME/src/MonkeyMesh}"
BIN="$ROOT/bin"
DATA="$ROOT/data"
LOG="$ROOT/logs"

SEED_DNS="${MESH_SEED_DNS:-seednode.hashmonkeys.cloud}"
SEED_HOST="${MESH_SEED_HOST:-$SEED_DNS}"
SEED_P2P_PORT="${MESH_SEED_P2P_PORT:-39001}"
SEED_RPC_PORT="${MESH_SEED_RPC_PORT:-18080}"
LAN_IP="${MESH_LAN_IP:-}"
BIND_IP="${MESH_BIND_IP:-0.0.0.0}"
EDGE_P2P_PORT="${MESH_EDGE_P2P_PORT:-39002}"
EDGE_RPC_PORT="${MESH_EDGE_RPC_PORT:-18083}"
AI_SHARDS_MAP="0=http://${SEED_DNS}:${SEED_RPC_PORT},1=http://${SEED_DNS}:18081,2=http://${SEED_DNS}:${EDGE_RPC_PORT}"
AI_SHARD_COUNT=3

mkdir -p "$BIN" "$DATA" "$LOG"
export PATH="$HOME/.cargo/bin:$PATH"
export RUST_LOG="${RUST_LOG:-info}"

usage() {
  cat <<EOF
Usage: $(basename "$0") {build|install|start|stop|status|restart}
  Remote edge: P2P :${EDGE_P2P_PORT} RPC :${EDGE_RPC_PORT} → seed ${SEED_HOST}:${SEED_P2P_PORT}
EOF
}

do_build() {
  cd "$SRC"
  cargo build --release -p mesh-node -p mesh-pool
  cp -f "$SRC/target/release/mesh-node" "$BIN/mesh-node"
  cp -f "$SRC/target/release/mesh-pool" "$BIN/mesh-pool"
  echo "bins -> $BIN"
}

write_units() {
  local unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
  mkdir -p "$unit_dir"
  local AI_TOKEN_LINE=""
  if [[ -f "$DATA/ai.token" ]]; then
    AI_TOKEN_LINE="Environment=MESH_AI_TOKEN=$(tr -d '\n\r' < "$DATA/ai.token")"
  elif [[ -f "$HOME/monkeymesh-testnet/data/ai.token" ]]; then
    cp -f "$HOME/monkeymesh-testnet/data/ai.token" "$DATA/ai.token"
    AI_TOKEN_LINE="Environment=MESH_AI_TOKEN=$(tr -d '\n\r' < "$DATA/ai.token")"
  fi
  local RPC_TOKEN_LINE=""
  if [[ ! -f "$DATA/rpc.token" ]]; then
    if command -v openssl >/dev/null 2>&1; then
      openssl rand -hex 32 >"$DATA/rpc.token"
    else
      head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$DATA/rpc.token"
    fi
    chmod 600 "$DATA/rpc.token"
  fi
  RPC_TOKEN_LINE="Environment=MESH_RPC_TOKEN=$(tr -d '\n\r' < "$DATA/rpc.token")"

  cat >"$unit_dir/mesh-edge2.service" <<EOF
[Unit]
Description=MonkeyMesh edge2 on hashserver (templates/submit; syncs from monkeynas seed)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=RUST_LOG=info
Environment=MESH_EDGE_MODE=1
Environment=MESH_ADVERTISE_HOST=${LAN_IP}
Environment=MESH_AI_UPSTREAM=http://${SEED_HOST}:${SEED_RPC_PORT}
Environment=MESH_EDGE_AI_LOCAL=1
Environment=MESH_AI_SHARD_ID=2
Environment=MESH_AI_SHARD_COUNT=${AI_SHARD_COUNT}
Environment=MESH_AI_SHARDS=${AI_SHARDS_MAP}
Environment=MESH_RPC_EDGES=http://${SEED_HOST}:18081,http://${LAN_IP}:${EDGE_RPC_PORT}
Environment=MESH_POW_V2_HEIGHT=${MESH_POW_V2_HEIGHT:-53000}
Environment=MESH_POW_EVO_HEIGHT=${MESH_POW_EVO_HEIGHT:-1}
Environment=MESH_POW_FUSION_HEIGHT=${MESH_POW_FUSION_HEIGHT:-80}
# F2 stays off (Build/36). Do not export MESH_FINALITY_HEIGHT on this host.
Environment=MESH_FORCE_RETARGET_INTERVAL=${MESH_FORCE_RETARGET_INTERVAL:-15}
${AI_TOKEN_LINE}
${RPC_TOKEN_LINE}
ExecStart=$BIN/mesh-node --chain $DATA/chain.bin serve --listen ${BIND_IP}:${EDGE_P2P_PORT} --connect ${SEED_HOST}:${SEED_P2P_PORT} --rpc ${BIND_IP}:${EDGE_RPC_PORT} --wallet $DATA/wallet.key --p2p-key $DATA/p2p.key --miner-key $DATA/wallet.key
Restart=on-failure
RestartSec=3
StandardOutput=append:$LOG/edge2.log
StandardError=append:$LOG/edge2.log

[Install]
WantedBy=default.target
EOF
  systemctl --user daemon-reload
  systemctl --user enable mesh-edge2.service
  echo "mesh-edge2 unit: P2P :${EDGE_P2P_PORT} RPC :${EDGE_RPC_PORT} → ${SEED_HOST}:${SEED_P2P_PORT}"
}

fetch_ai_token() {
  if [[ -f "$DATA/ai.token" ]]; then
    return 0
  fi
  SEED_SSH="${MESH_SEED_SSH:-${MESH_SEED_USER:+${MESH_SEED_USER}@}${SEED_HOST}}"
  if ssh -o BatchMode=yes -o ConnectTimeout=5 "$SEED_SSH" "cat ~/monkeymesh-testnet/data/ai.token" >"$DATA/ai.token.tmp" 2>/dev/null; then
    mv "$DATA/ai.token.tmp" "$DATA/ai.token"
    echo "fetched ai.token from seed"
  else
    rm -f "$DATA/ai.token.tmp"
    echo "warn: could not fetch ai.token (AI proxy may 401 until set)"
  fi
}

cmd_install() {
  if [[ ! -x "$BIN/mesh-node" ]]; then
    do_build
  else
    echo "using existing $BIN/mesh-node (skip cargo)"
  fi
  fetch_ai_token
  write_units
  # Optional pool unit — default OFF (pool runs on monkeynas). MESH_HASHSERVER_POOL=1 to keep it here.
  if [[ -x "$BIN/mesh-pool" ]] || [[ -f "$SRC/target/release/mesh-pool" ]]; then
    cp -f "$SRC/target/release/mesh-pool" "$BIN/mesh-pool" 2>/dev/null || true
  fi
  if [[ -f "$BIN/mesh-pool" ]]; then
    local unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
    if [[ "${MESH_HASHSERVER_POOL:-0}" == "1" || "${MESH_HASHSERVER_POOL:-}" == "true" ]]; then
      cat >"$unit_dir/mesh-pool.service" <<EOF
[Unit]
Description=MonkeyMesh HTTP mining pool (GBT proxy)
After=network-online.target mesh-edge2.service
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=RUST_LOG=info
ExecStart=$BIN/mesh-pool --bind 0.0.0.0:12500 --upstream http://127.0.0.1:${EDGE_RPC_PORT} --keyfile $DATA/pool.key --credits $DATA/pool_credits.json --blocks $DATA/pool_blocks.json
Restart=on-failure
RestartSec=3
StandardOutput=append:$LOG/pool.log
StandardError=append:$LOG/pool.log

[Install]
WantedBy=default.target
EOF
      systemctl --user daemon-reload
      systemctl --user enable mesh-pool.service
      echo "mesh-pool unit: :12500 (scheme DFPPS+/suffix 500) → upstream :${EDGE_RPC_PORT}"
    else
      systemctl --user disable --now mesh-pool.service 2>/dev/null || true
      echo "mesh-pool skipped on hashserver (canonical pool is monkeynas; set MESH_HASHSERVER_POOL=1 to override)"
    fi
  fi
  cat >"$ROOT/ENDPOINTS.txt" <<EOF
MonkeyMesh hashserver edge2
P2P:     ${LAN_IP}:${EDGE_P2P_PORT}
RPC:     http://${LAN_IP}:${EDGE_RPC_PORT}/v1/getnodeinfo
Pool:    on monkeynas :12500 (HTTPS https://eu.hashmonkeys.cloud)
Seed:    ${SEED_HOST}:${SEED_P2P_PORT} / http://${SEED_HOST}:${SEED_RPC_PORT}
AI up:   http://${SEED_HOST}:${SEED_RPC_PORT}
EOF
  cat "$ROOT/ENDPOINTS.txt"
}

cmd_start() {
  systemctl --user start mesh-edge2.service
  systemctl --user start mesh-pool.service 2>/dev/null || true
  sleep 2
  systemctl --user --no-pager status mesh-edge2.service || true
  curl -fsS "http://127.0.0.1:${EDGE_RPC_PORT}/v1/getnodeinfo" | head -c 400 || echo "edge2 RPC not up yet"
  echo
  curl -fsS "http://127.0.0.1:12500/v1/poolstats" | head -c 300 || true
  echo
}

cmd_stop() {
  systemctl --user stop mesh-pool.service 2>/dev/null || true
  systemctl --user stop mesh-edge2.service || true
}

cmd_status() {
  systemctl --user --no-pager status mesh-edge2.service mesh-pool.service || true
  curl -fsS "http://127.0.0.1:${EDGE_RPC_PORT}/v1/getnodeinfo" | head -c 500 || true
  echo
  curl -fsS "http://127.0.0.1:${EDGE_RPC_PORT}/v1/ai/health" | head -c 400 || true
  echo
  curl -fsS "http://127.0.0.1:12500/v1/poolstats" | head -c 400 || true
  echo
}

cmd_restart() {
  cmd_stop
  sleep 1
  cmd_start
}

case "${1:-}" in
  build) do_build ;;
  install) cmd_install ;;
  start) cmd_start ;;
  stop) cmd_stop ;;
  status) cmd_status ;;
  restart) cmd_restart ;;
  *) usage; exit 1 ;;
esac
