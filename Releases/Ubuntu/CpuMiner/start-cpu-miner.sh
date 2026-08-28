#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
chmod +x ./mesh-miner-cpu 2>/dev/null || true
RPC=$(python3 -c "import json;print(json.load(open('config.json'))['rpc'].rstrip('/'))")
ADDR=$(python3 -c "import json;print(json.load(open('config.json')).get('address','').strip())")
BLOCKS=$(python3 -c "import json;print(json.load(open('config.json')).get('blocks',0))")
MAX_NONCES=$(python3 -c "import json;print(json.load(open('config.json')).get('max_nonces',5000000))")
if [[ -z "$ADDR" ]]; then
  echo "Set address in config.json to your wallet payout address."
  exit 1
fi
exec ./mesh-miner-cpu --rpc "$RPC" --address "$ADDR" --blocks "$BLOCKS" --max-nonces "$MAX_NONCES"