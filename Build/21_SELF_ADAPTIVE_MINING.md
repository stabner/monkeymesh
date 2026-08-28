# Self-adaptive mining (Build/21)

Mining power improves **this** chain. No marketplace product. No foreign-chain redirect.

## Role split

| Market | Work | Effect |
|--------|------|--------|
| CPU (45% from height 1) | Fusion lane A / block find | Consensus security |
| GPU (45% from height 1) | Fusion lane B credit + one immune exam | Dual-lane PoW + rematched protocol sim |
| Node (legacy 20% / Build/31 10%) | Relay / archive / AI routing attestations | Mesh health |

GPU research stays on the shared pot. Fusion (Build/32) also puts GPU mix into the **same** block hash as the CPU walk.

## Closed loop

```
MeshPulse
  → Orchestrator auto-enqueues under-covered research
  → GPU workers run protocol_eval (deterministic protocol sims)
  → Orchestrator verifies digest
  → AiJobReceipt → GPU scores (Build/14)
  → MeshPulse research_progress + score trends
  → Soft envelopes auto-apply as a new **param epoch**
  → Orchestrator + mining use active envelopes
```

**BPS never auto-moves.** Legacy 40/40/20, then height-gated 90/10 contributor/node (Build/31). Soft knobs only unless a human height gate says otherwise.

**Self-adaptation** means versioned soft param epochs — not competing chain tips.

## Non-goals

- MonkeyMind marketplace (Build/12, Build/17 — **shelved**)
- MonkeyMesh Agent chat (Build/22 — **removed**)
- Silent hashrate diversion to other chains
- AI-activated consensus / emission changes
- Tip forks as an adaptation mechanism

## Ops

- Orchestrator research tick keeps the queue warm when GPU workers are online
- Node auto-applies soft envelopes after enough verified `protocol_eval` receipts
- Smoke: `Launchers/smoke-adaptive-auto.ps1`

## Heavy dual-connect research (1A + 2A)

- **Pool** = MeshHash shares only. **AI** = miner → seed `:18080` (`ai_research: true`).
- Parallel `ml_train_shared_v2` jobs sized to VRAM/`train_slots`; CUDA on miners; seed CPU-verifies brain advances.
- Light jobs may use soft `brain_audit_every` (1-in-K full re-exec). Shared brain / guardians **always** full-verify.
- Epoch-race losers (correct train, stale tip) still earn **partial** GPU credit.
- Honesty: this is sustained CUDA research on small verifiable MLPs + protocol sims — **not** foundation-model training. The public explorer does not show a live guardian training board.

See Build/15 (adaptive loop) and Build/18 (research scenarios).
