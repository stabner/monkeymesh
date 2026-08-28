# Governance / adaptive UX

Soft envelopes **auto-apply** from verified research (Build/15, Build/21). Humans alone change **hard** consensus params (BPS — Build/11).

## Surfaces

| Surface | Action |
|---------|--------|
| Node GUI | MeshPulse, research progress, **active soft knobs**, **param epoch history** (read-only; no Suggest / Approve) |
| Explorer | Markets + MeshPulse + envelopes + **epoch history table** |
| RPC | `GET /v1/proposals`, `/v1/envelopes` (includes `epoch_history`); `POST /v1/proposals/generate` (dev/smoke only); vote routes kept for hard governance later |
| Smoke | `Launchers/smoke-adaptive-auto.ps1` |

## What soft auto-apply does

- Applies **soft envelopes** (adapt threshold, benchmark rounds, min verifier weight, idle stipend cap hint, suggested CPU diff bias)
- Does **not** change emission BPS (90/10 after Build/31; AI cannot move it)

## Node operator model

Operators **observe** soft updates. They do **not** propose or vote soft knobs in the GUI — verified GPU research decides those inside floors/ceilings.

## Safety

- BPS suggestions clamped to floors/ceilings in types; not applied by auto-adapt
- Mutating routes honor `MESH_RPC_TOKEN` when set
- Orchestrator soft-adapt / research tick reads activated envelopes
