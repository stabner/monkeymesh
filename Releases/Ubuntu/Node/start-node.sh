#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
chmod +x ./mesh-node 2>/dev/null || true
LISTEN=$(python3 -c "import json;print(json.load(open('config.json'))['listen'])")
RPC=$(python3 -c "import json;print(json.load(open('config.json'))['rpc'])")
CONNECT=$(python3 -c "import json;print(' '.join('--connect '+p for p in json.load(open('config.json')).get('connect',[])))")
OP_ADDR=$(python3 -c "import json;print(json.load(open('config.json')).get('operator_address','').strip())")
OP_VAULT=$(python3 -c "import json;print(json.load(open('config.json')).get('operator_vault','').strip())")
OP_ARGS=()
if [[ -n "$OP_ADDR" ]]; then
  export MESH_OPERATOR_ADDRESS="$OP_ADDR"
  OP_ARGS+=(--operator-address "$OP_ADDR")
fi
if [[ -n "$OP_VAULT" ]]; then
  VAULT_PATH="$OP_VAULT"
  if [[ "$VAULT_PATH" != /* ]]; then VAULT_PATH="$(pwd)/$VAULT_PATH"; fi
  export MESH_OPERATOR_VAULT="$VAULT_PATH"
  OP_ARGS+=(--operator-vault "$VAULT_PATH")
fi
# shellcheck disable=SC2086
exec ./mesh-node --chain data/chain.bin serve --listen "$LISTEN" --rpc "$RPC" \
  --wallet data/wallet.key --p2p-key data/p2p.key --miner-key data/wallet.key \
  "${OP_ARGS[@]}" $CONNECT