# Adaptive Loop (MeshPulse)

## Intent

GPUs are the learning engine of MonkeyMesh. They do **not** run inference inside consensus. They produce verified research work + telemetry that trains an **Adaptive Proposer**. **Soft envelopes auto-apply** inside hard floors/ceilings. **Bounded retarget knobs** may auto-apply from quantum-gated certificates (Build/30). Humans alone activate **hard** BPS / emission / crypto (Build/11).

See **Build/21_SELF_ADAPTIVE_MINING.md** for the miner-facing loop.

## Closed loop

```
AI research jobs on GPUs
  → verified AiJobReceipts
  → MeshPulse feature blob
  → Adaptive model
  → Soft envelopes auto-apply (clamped)
  → safe envelopes feed markets / routing / research tick
```

Hard BPS still requires human-weighted governance (not auto).

## What GPUs earn for

Adaptive research / benchmark jobs that yield attested receipts:

- job id
- input commitment
- output hash
- latency_ms
- worker address
- verifier status

Receipts credit **`S_gpu` only** (Build/14).

Marketplace user jobs are **shelved** (Build/12, Build/17); stubs may remain in code.

## MeshPulse features

Aggregated from receipts + chain metrics:

- job success / fail rates, p50/p95 latency
- queue depth / congestion
- orphan rate, invalid share rate
- relative CPU vs GPU vs node health signals
- research_eval_receipts / research_progress

## AI may

- Retarget **soft** envelopes (routing thresholds, verifier weight floor, idle stipend hint, CPU soft diff bias)
- Soft control-plane tweaks inside safe bounds that do not change consensus emission

## AI must never

- Activate hard consensus rule changes
- Move emission across markets without human-weighted vote
- Slash or seize funds by model fiat

## Safety envelope

- Soft auto-apply only within floors/ceilings
- Hard floors/ceilings on AI-suggested BPS / difficulty (BPS not applied without governance)
- Deterministic receipt verification (echo/benchmark/protocol_eval)
- Permissioned orchestrator early; public AI directory out of v1

## Evolution path

1. Faster recovery from abuse (tighter spam / verifier quorum)
2. Better GPU capacity use (task-aware research scheduling)
3. Balanced security without paying GPUs from the CPU budget
4. Protocol-eval research jobs paid from **GPU units in the 90% contributor pot** (Build/31)

See **Build/18_ADAPTIVE_RESEARCH.md** for scenarios.
