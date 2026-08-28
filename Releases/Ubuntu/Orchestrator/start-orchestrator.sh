#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
chmod +x ./mesh-orchestrator 2>/dev/null || true
export MESH_ORCH_BIND="${MESH_ORCH_BIND:-0.0.0.0:18100}"
export MESH_NODE_RPC="${MESH_NODE_RPC:-http://127.0.0.1:18080}"
export MESH_ORCH_REQUIRE_NODE="${MESH_ORCH_REQUIRE_NODE:-1}"
exec ./mesh-orchestrator