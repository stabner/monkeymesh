# Adaptive protocol research framework (Phase 5)

GPUs run structured **protocol simulations** on MonkeyMesh itself (block creation,
security, privacy, scale). Results are deterministically verified, credit the
**GPU units in the 90% contributor pot**, enrich MeshPulse, and feed soft **param epochs** (Build/15, Build/21, Build/31).
Hard BPS still needs human governance (Build/11).

## Loop

```
MeshPulse / coverage gaps
  → Orchestrator auto-enqueue (or POST /v1/research/enqueue)
  → GPU worker protocol_eval (scenario sim → score digest)
  → orchestrator verifies digest
  → AiJobReceipt (+ scenario / scores) → GPU scores
  → MeshPulse.research_progress + score trends
  → soft envelopes auto-apply as a new param epoch
```

## Scenarios

| Id | Purpose |
|----|---------|
| `block_propagation` | Block create + fan-out; orphan / majority timing |
| `security_adversary` | Invalid shares / spam / verifier dropout |
| `privacy_leakage` | Gossip metadata linkability (heuristic) |
| `scale_throughput` | Growth under height + queue + peers |
| `spam_recovery` | Tighter spam / verifier quorum |
| `routing_efficiency` | Task-aware soft routing |
| `market_balance` | PoMC markets stay isolated |
| `verifier_quorum` | Raise verifier weight under load |

## Verification

Worker and orchestrator share `mesh_ai::run_protocol_eval`.
Wire payload `mesh-research:v2:<scenario>:h=<height>:sig=<signal>`.
Digest = blake3 of canonical `ResearchResult` bytes (scores), not a generic mix.
Forged digests are rejected.

## Param epochs

Each soft auto-apply increments an **envelope epoch** on the node store
(`GET /v1/envelopes` → `param_epoch`, `epoch_history`). This is operating-param
adaptation, not a tip fork. BPS never auto-moves (90/10 is a height gate, Build/31).

## API (orchestrator `:18100`)

- `GET /v1/research/scenarios`
- `POST /v1/research/enqueue` `{ "scenario": "scale_throughput", "height": 10, "pulse_signal": 0.2 }`
- `GET /v1/research/status` — verify rates, scenario coverage, recent scores

## Smoke

```powershell
.\Launchers\smoke-adaptive-auto.ps1
.\Launchers\smoke-research.ps1
```
