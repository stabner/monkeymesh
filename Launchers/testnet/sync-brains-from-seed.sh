#!/usr/bin/env bash
# Cold-standby brain replication: seed → edge2 data dir (Build/28 M3).
# Brains are NOT loaded while MESH_EDGE_MODE=1; files are for seed-promotion failover.
#
# Run on hashserver (timer or manual):
#   ~/src/MonkeyMesh/Launchers/testnet/sync-brains-from-seed.sh
set -euo pipefail

SEED_HOST="${MESH_SEED_HOST:?set MESH_SEED_HOST}"
SEED_USER="${MESH_SEED_USER:?set MESH_SEED_USER}"
DEST="${MESH_BRAIN_DEST:-$HOME/monkeymesh-edge2/data}"
SRC_DATA="${MESH_SEED_DATA:-monkeymesh-testnet/data}"

mkdir -p "$DEST"
files=(shared_brain.bin shared_brain_v2.bin leg_brains.bin quantum_brains.bin)
ok=0
for f in "${files[@]}"; do
  if scp -o BatchMode=yes -o ConnectTimeout=8 \
    "${SEED_USER}@${SEED_HOST}:${SRC_DATA}/${f}" "${DEST}/${f}.tmp" 2>/dev/null; then
    mv -f "${DEST}/${f}.tmp" "${DEST}/${f}"
    echo "ok $f ($(wc -c < "${DEST}/${f}") bytes)"
    ok=$((ok + 1))
  else
    echo "warn: miss $f"
    rm -f "${DEST}/${f}.tmp"
  fi
done
# Also keep ai.token in sync for promotion
if scp -o BatchMode=yes -o ConnectTimeout=5 \
  "${SEED_USER}@${SEED_HOST}:${SRC_DATA}/ai.token" "${DEST}/ai.token.tmp" 2>/dev/null; then
  mv -f "${DEST}/ai.token.tmp" "${DEST}/ai.token"
  echo "ok ai.token"
fi
echo "brain sync done ($ok/4 files) → $DEST"
test "$ok" -ge 1
