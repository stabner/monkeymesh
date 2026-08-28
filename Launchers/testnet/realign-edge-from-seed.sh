#!/usr/bin/env bash
# Copy the *higher* local tip between seed and edge (monkeynas).
# Never clobber a longer chain with a shorter one.
# Usage: ~/src/MonkeyMesh/Launchers/testnet/realign-edge-from-seed.sh
set -euo pipefail
ROOT="${MESH_TESTNET_ROOT:-$HOME/monkeymesh-testnet}"
DATA="$ROOT/data"
EDGE="$ROOT/edge"

snap_h() {
  python3 -c "import json,sys; print(int(json.load(open(sys.argv[1])).get('height') or 0))" "$1" 2>/dev/null || echo 0
}

SEED_SNAP="$DATA/chain.snap.json"
EDGE_SNAP="$EDGE/chain.snap.json"
SEED_H=0
EDGE_H=0
[[ -f "$SEED_SNAP" ]] && SEED_H="$(snap_h "$SEED_SNAP")"
[[ -f "$EDGE_SNAP" ]] && EDGE_H="$(snap_h "$EDGE_SNAP")"

copy_chain() {
  local src="$1" dst="$2"
  rm -f "$dst/chain.blocks.wal" "$dst/chain.meta.bin" "$dst/chain.snap.json"
  cp -a "$src/chain.blocks.wal" "$src/chain.meta.bin" "$src/chain.snap.json" "$dst/"
  [[ -f "$src/chain.bin" ]] && cp -a "$src/chain.bin" "$dst/chain.bin" || true
}

systemctl --user stop mesh-pool.service mesh-edge.service || true
sleep 1

if [[ "$SEED_H" -gt "$EDGE_H" ]]; then
  echo "realign: seed h=$SEED_H > edge h=$EDGE_H — copy seed → edge"
  copy_chain "$DATA" "$EDGE"
elif [[ "$EDGE_H" -gt "$SEED_H" ]]; then
  echo "realign: edge h=$EDGE_H > seed h=$SEED_H — copy edge → seed"
  systemctl --user stop mesh-node.service || true
  copy_chain "$EDGE" "$DATA"
  systemctl --user start mesh-node.service || true
else
  echo "realign: seed and edge both h=$SEED_H — leave files, HTTP/P2P will converge"
fi

systemctl --user start mesh-edge.service
sleep 3
systemctl --user start mesh-pool.service
curl -fsS -m 15 "http://127.0.0.1:18081/v1/getnodeinfo" | head -c 240
echo
curl -fsS -m 10 "http://127.0.0.1:12500/health" || true
echo
