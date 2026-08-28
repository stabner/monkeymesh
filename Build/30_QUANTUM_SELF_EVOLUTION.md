# Quantum Self-Evolution (Build/30)

**Status: RESEARCH / SOFT-ONLY** — bounded retarget knobs after verified Grover certs. Does **not** move BPS, does **not** find blocks, and is **not** the public Testnet page. See `Build/33_WHITEPAPER.md`.

Goal: project-owned self-evolution — verified quantum research can auto-activate **bounded** protocol improvements. Not a user AI marketplace.

## Loop

```
quantum_train / quantum_grover protocol_eval
  → seed bit-exact verify
  → ImprovementCertificate ring + scenario counters
  → soft auto-adapt (≥3 protocol_evals)
  → quantum gate (≥5 new quantum_grover certs) unlocks retarget knobs
  → next_difficulty() reads active envelopes
```

## What AI may auto-change (clamped)

| Knob | Range | Default |
|------|-------|---------|
| `retarget_interval` | 10..=40 | 15 |
| `retarget_step` | 1..=2 | 1 |
| `min_difficulty_floor` | 1..=16 | 1 |

One adapt may move interval by at most ±5, step by 1, floor by 1.

Gate: `grover_certs_since_retarget_adapt >= MIN_GROVER_CERTS_FOR_RETARGET` (5).

## What stays human-only

- Market BPS / emission split (Build/31 90/10 is a **height gate**, not an AI knob)
- Signature crypto / PQC migration
- MeshHash algorithm version / catalog opcodes (v3 Evo is a height gate; recipe *inside* the catalog may move)
- Unbounded tip forks

## Marketplace

**Shelved** (Build/12). GPU pay = research / attestation for the project only.

## APIs

- `GET /v1/envelopes` — live retarget + `quantum_gate` counters
- `GET /v1/quantum` — board + `self_evolution` gate snapshot
- `GET /v1/proposals` — epoch history; look for `quantum-gated-retarget`

## Honesty

Guardians do not invent finished post-quantum crypto. They make weak Grover posture expensive by hardening the difficulty schedule within floors — visible, reversible by later healthy certificates within clamps.
