# MonkeyMesh Orchestrator

**Status: SHELVED product path (N12)** — MonkeyMind marketplace is not shipping. Live AI jobs use the **seed embedded board** at `:18080` (`/v1/advertise|job|result`). This folder still runs the optional standalone orch stub for local experiments.

## Start (lab stub)

1. Node: `Launchers\Node\Start-Node.bat`
2. Orchestrator: `Launchers\Orchestrator\Start-Orchestrator.bat`
3. Worker: prefer seed board, or `Launchers\Orchestrator\Start-GpuWorker.bat`

Marketplace UI (stub only): http://127.0.0.1:18100/marketplace

Free-tier guardrails: max prompt 8 KiB; ~30 submits / 60s; job history capped.

## Env

| Var | Default |
|-----|---------|
| `MESH_ORCH_BIND` | `127.0.0.1:18100` |
| `MESH_NODE_RPC` | `http://127.0.0.1:18080` |
| `MESH_ORCH_REQUIRE_NODE` | `1` (set `0` to accept results without node) |

## API

- `POST /v1/marketplace/jobs` `{ "service":"llm", "prompt":"..." }`
- `GET /v1/marketplace/jobs`
- Worker: `/v1/advertise`, `/v1/job`, `/v1/result`

See Build/17_MONKEYMIND_MARKETPLACE_MVP.md (SHELVED). Smoke: `.\Launchers\smoke-marketplace.ps1`
