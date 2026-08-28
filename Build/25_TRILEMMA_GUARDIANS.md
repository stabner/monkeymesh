# Trilemma Guardians — self-hardening mesh (Build/25)

**Status: RESEARCH / OPTIONAL** — not the public Testnet product.

The explorer **retired** the giant four-guardian / “trained 0 epochs” board. Fusion MeshHash is the live work. These legs may still run as optional GPU jobs and feed soft needles; they are **not** a live training TV and they do **not** find blocks. See `Build/33_WHITEPAPER.md`.

Goal: use GPU miners to continuously train **four specialist guardian models** that evolve against harder attacks, publish **absolute** public metrics, and **soft-auto-protect** the live mesh — without tip forks and without auto-moving BPS.

This does **not** claim a mathematical “solution” to the blockchain trilemma. It claims something shippable and stronger for a live network: **measure the triangle in public numbers, feed the weak leg first, and tighten soft defenses when security slips**.

## Why guardians (not one mega-model)

One MNIST brain (Build/23–24) proves bit-exact GPU train. Guardians are the **real product brain**:

| Leg | Protects | Learns to predict / pressure-test |
|-----|----------|-----------------------------------|
| **Security** | Manipulation, spam, bad shares, crashes | Attack detect vs miss |
| **Network** | Congestion, unfair work distribution | Latency / backlog / routing health |
| **Blocks** | Block spawn, tip splits, transfer finality | Orphan / confirm health |
| **Transpar** | Hidden loopholes via opacity | Linkability / public verify honesty |

Curriculum **hardens with epoch** (adversarial features intensify). Wrong predictions → weight update → next epoch is smarter. Seed always **re-executes** the train step (same contract as shared brain).

## Absolute Trilemma Board (public)

All scores are integers **0–100**. No EMA soup on the main panel.

| Needle | Meaning | Healthy |
|--------|---------|---------|
| `sec` | Catch attacks / reject junk | ↑ |
| `scale` | Move work without backlog | ↑ |
| `decent` | Many independent verifiers / nodes | ↑ |
| `transpar` | Hard to secretly game / link | ↑ |
| `balance` | `100 × min(sec,scale,decent) / max(...)` | → 100 |
| `weakest` | Name of lowest needle | feed this leg first |

**Working definition of progress:** all three core needles rise **and** `balance` stays high. One needle at 95 while another at 20 = not protected — skewed.

API: `GET /v1/trilemma` (+ embedded on MeshPulse).

## Job wire

```text
mesh-legtrain:v1:leg=security|network|blocks|transpar:epoch=E:steps=N:lr_milli=M:samples=K:offset=O
```

- Kind on wire: `leg_train` (still `AiJobKind::MlTrain` internally)
- Persist: `data/leg_brains.bin` (four weight blobs + epochs)
- Model download: `GET /v1/leg/{leg}` / meta

Parallelism: research tick enqueues **up to `leg_parallel` jobs**, preferring the **weakest** needle, then uncovered legs. MNIST v1/v2 remain optional gym jobs.

## Soft knobs (auto)

| Knob | Default | Role |
|------|---------|------|
| `leg_train_enable` | 1 | Master switch |
| `leg_parallel` | 2 | Max simultaneous leg jobs |
| `leg_harden_sec_floor` | 55 | If `sec` below this → raise verifier floor |
| `brain_prefer_v2` | (Build/24) | Unchanged |

Security auto-harden (soft only): low `sec` → higher `min_verifier_weight`, cooler adapt threshold, more `SecurityAdversary` / security-leg jobs. **Never** changes BPS (90/10 is a height gate).

## Self-changing loop

```text
miners train guardians + run protocol_eval red-team sims
        ↓
seed verifies bit-exact → advances leg epochs
        ↓
Trilemma Board updates absolute needles
        ↓
soft auto-adapt tightens defenses on weak/security
        ↓
tick feeds weakest leg first (more GPU $ there)
        ↓
curriculum hardness ↑ with epoch → models evolve
```

Humans still own BPS / consensus difficulty. Guardians own **continuous pressure-testing + soft shields**.

## Phased delivery

| Phase | Deliverable |
|-------|-------------|
| A | Board + absolute metrics + `/v1/trilemma` |
| B | Four leg brains + job wire + verify |
| C | Parallel weakest-first enqueue + security harden |
| D | Node GUI Trilemma Board |
| E | Later: richer CUDA guardians / open adversarial corpus |

## Honesty bar

- Guardians **reduce** loophole surface by paying GPUs to find and score failure modes early.
- They **cannot** guarantee zero exploits forever (no system can).
- They **can** make silent skew and weak security **public, expensive, and self-correcting**.
