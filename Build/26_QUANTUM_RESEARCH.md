# Quantum Research Guardians — post-quantum readiness (Build/26)

**Status: RESEARCH / OPTIONAL** — not the public Testnet product.

The explorer **retired** the quantum-guardian scoreboard. These jobs may still run as optional GPU research; they do **not** mean MESH is post-quantum, and they do **not** find blocks. See `Build/33_WHITEPAPER.md`.

Goal: pay GPU miners to continuously train **three specialist quantum-research models** that pressure-test the mesh against quantum-era threats, publish **absolute** public readiness needles, and feed **soft** suggestions — without tip forks and without auto-moving BPS.

This does **not** claim a working quantum computer, Shor break of Ed25519 on MESH, or a finished post-quantum migration. It claims something shippable: **measure quantum-era risk in public numbers, train against hardening curricula, and keep humans in control of crypto upgrades.**

## Why separate from Trilemma Guardians

| System | Job | Models |
|--------|-----|--------|
| Trilemma (Build/25) | Live mesh health triangle | security / network / blocks / transpar |
| Quantum (Build/26) | Future crypto / PoW / secrecy risk | pqc / grover / harvest |

Both run side-by-side as GPU units in the 90% pot. Neither can change BPS.

## Three quantum legs

| Leg | Studies | Learns to predict |
|-----|---------|-------------------|
| **PQC** | Post-quantum signature / hash migration readiness | “Would classical assumptions fail under rising quantum pressure?” |
| **Grover** | PoW / search resistance under √N speedup modeling | Difficulty / search budget under Grover-style adversary |
| **Harvest** | Harvest-now-decrypt-later (long-lived secrecy) | How badly recorded traffic ages if keys fall later |

Curriculum **hardens with epoch**. Seed **re-executes** the train step (same bit-exact contract as Trilemma / shared brain).

## Absolute Quantum Board (public)

Scores are integers **0–100**. Higher is healthier / more ready.

| Needle | Meaning | Healthy |
|--------|---------|---------|
| `pqc` | Post-quantum crypto readiness posture | ↑ |
| `grover` | PoW resilience under Grover-style speedup | ↑ |
| `secrecy` | Long-term secrecy vs harvest-now risk | ↑ |
| `readiness` | `min(pqc, grover, secrecy)` — weakest link | ↑ |
| `weakest` | Name of lowest needle | feed this leg first |

API: `GET /v1/quantum` (+ optional MeshPulse note).

## Job wire

```text
mesh-qtrain:v1:leg=pqc|grover|harvest:epoch=E:steps=N:lr_milli=M:samples=K:offset=O
```

- Kind on wire: still `AiJobKind::MlTrain` internally (like `leg_train`)
- Persist: `data/quantum_brains.bin`
- Soft knobs: `quantum_train_enable` (default 1), `quantum_parallel` (default 1)

## Protocol sims (GPU `protocol_eval`)

Extra research scenarios (also enqueued):

- `quantum_pqc` — classical→PQC migration pressure
- `quantum_grover` — search / PoW under √N adversary
- `quantum_harvest` — recorded ciphertext aging / HNDL

## Soft + bounded retarget (Build/30)

- AI may propose / soft-adapt research intensity.
- AI may auto-adjust **bounded** retarget knobs (`retarget_interval`, `retarget_step`, `min_difficulty_floor`) when ≥5 verified `quantum_grover` certificates accrue since the last retarget adapt (Build/30).
- AI **must not** auto-change market BPS, signature crypto, or MeshHash algorithm version.
- Humans activate any real PQC migration.

## Honesty bar

- Guardians **reduce surprise** by scoring quantum-era failure modes early.
- They **cannot** invent a finished NIST-PQC rollout by themselves.
- They **can** make “we ignored quantum risk” expensive and visible on the testnet board.
