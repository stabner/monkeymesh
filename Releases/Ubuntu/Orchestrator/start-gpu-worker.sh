#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
chmod +x ./mesh-gpu-worker 2>/dev/null || true
ORCH="${MESH_ORCH:-http://127.0.0.1:18080}"
mkdir -p data
exec ./mesh-gpu-worker --orch "$ORCH" --jobs 8 --poll-ms 400 --keyfile data/gpu-worker.key