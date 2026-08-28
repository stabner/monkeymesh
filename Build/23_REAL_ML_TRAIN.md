# Real AI training + shared network brain

**Status: ACTIVE** — GPU market jobs on the **node** (embedded job board) do two things:

1. **Shared network brain** — one MLP for everyone; workers train from epoch **E**, seed verifies and publishes **E+1**
2. **Protocol research** — named blockchain improvement questions (propagation, security, scale, markets…)

Workers connect to **node RPC** (`:18080`). For the **one shared brain**, point AI at the **seed** (`http://seednode.hashmonkeys.cloud:18080`) so everyone advances the same weights.

## Shared brain (primary path)

| Item | Detail |
|------|--------|
| Model | MLP `784 → 64 → 10`, tanh hidden, softmax CE (~400KB `f64` weights) |
| State | Live weights on seed AI board; persisted as `data/shared_brain.bin` |
| Genesis | Epoch `0` from deterministic seed (`GENESIS_BRAIN_SEED`) |
| Verify | Seed re-runs train from epoch E; output weights/digest must match |
| Stale | Submit for an old epoch → no brain advance, no GPU weight |
| Job kind | `ml_train_shared` (or `ml_train` with shared wire prefix) |

Wire input:

```text
mesh-mltrain-shared:v1:epoch=E:steps=N:lr_milli=M:samples=K:offset=O
```

Weights blob magic: `MESHBRAINv1` (little-endian `f64`). Portable `libm` math so Windows workers match Linux seed verify.

### Worker flow

1. `GET /v1/model` — epoch + weights
2. `POST /v1/job` — shared train job for current epoch
3. Train from those weights
4. `POST /v1/result` — new weight blob; on match seed applies epoch+1

Lightweight UI: `GET /v1/model/meta`. MeshPulse includes `brain_epoch`, `brain_digest_hex`, `brain_acc`, `brain_advances`.

## Capacity advertise (hardware → network)

Miners report real hardware on `POST /v1/advertise`:

| Field | Meaning |
|-------|---------|
| `vram_mb` | Sum of selected GPU VRAM |
| `train_slots` | Parallel job capacity (0 = derive from VRAM) |
| `gpu_name` | Best selected device label |

VRAM → slots (proxy): ≤2 GiB→1, ~6 GiB→2, ~12 GiB→3, ~20 GiB→4, ~24–32 GiB→6, larger→8.

The shared-brain MLP is **small and CPU-verifyable** (so every node can re-exec). VRAM does **not** fill with model weights today. Instead:

- **Heavier** shared-brain jobs (more `steps` / `samples`, parallel v2 by `train_slots`)
- Capacity-driven queue depth (soft cap high enough for multi-slot GPUs)
- Miner runs **up to 8 local AI pullers** matching advertised slots (dual-connect)
- **Weight cache** + HTTP keep-alive — no ~800 KB model re-download every job
- Re-advertise on `unknown worker`; backoff on empty / stale (409)
- PoW CUDA and AI train **share** the card; research priority rises when the AI board is deep

PoW MeshHash uses VRAM for scratchpads. Heavy research aims for sustained CUDA util from `ml_train_shared_v2` + filled protocol/guardian queues — still not foundation-model training.

## Legacy scratch MNIST (dev only)

Fresh-from-scratch jobs still parse:

```text
mesh-mltrain:v1:steps=N:lr_milli=M:seed=S:samples=K:offset=O
```

Research tick **no longer** enqueues these; shared brain jobs replace them.

## Dataset / optimizer

| Item | Detail |
|------|--------|
| Dataset | First **4096** samples of official MNIST training set |
| Optimizer | SGD (deterministic `f64`) |

## Blockchain training questions

Scenarios from `Build/18` / `mesh-ai` research catalog, picked from MeshPulse:

| Signal | Question enqueued |
|--------|-------------------|
| High orphan risk | `block_propagation` |
| Low detect rate | `security_adversary` |
| High backlog / latency | `scale_throughput` |
| Weak GPU vs height | `routing_efficiency` |
| Low research progress | `market_balance` |
| Low primary scores | `verifier_quorum` |
| Default rotate | `privacy_leakage` |

## Network-growth + capacity self-tune

Node research tick every ~5s (when workers are advertised):

- Target queue depth ≈ `4 + 3×slots` (clamped 4–48), scaled by soft stipend envelope
- Enqueues **one** shared-brain job (epochs are sequential) sized by growth × VRAM slots
- Fills remaining depth with **protocol** jobs (≈ `2×slots`)
- Extra protocol job when GPU signal is below soft threshold
- Soft envelopes auto-adapt from receipts (`mesh-chain` growth rules)

Height growth factor: 1→5 as height rises (100 / 1k / 5k / 20k+).

BPS split stays human-only (90/10 after Build/31 gate).

## Worker API (on node `:18080`)

- `GET /v1/model`, `GET /v1/model/meta`
- `POST /v1/advertise`, `POST /v1/job`, `POST /v1/result`, `POST /v1/results` (batch ≤ `MESH_BRAIN_VERIFY_BATCH_MAX`, default 8)
- `GET /v1/ai/health`, `GET /v1/research/status`, `GET /v1/workers`, `GET /v1/research/scenarios`

Verify path (Build/27 N9): `prepare_complete` clones weights under the AI lock → CPU re-train in `spawn_blocking` / `run_cpu_batch` → `finish_complete` / `finish_completes` applies if epoch still matches. Soft caps: `MESH_BRAIN_VERIFY_MAX_STEPS`, `MESH_BRAIN_VERIFY_MAX_SAMPLES`, `MESH_BRAIN_VERIFY_BATCH_MAX`. Shared/SharedV2 stay sequential on apply (stale → per-item fail); Light + different Legs batch safely.

Standalone `:18100` orchestrator remains for marketplace UI only; point AiWorker / Miner AI at seed `:18080` for the shared brain.

## Next

Shared-brain **v2** (CUDA Q16 `mlp512`) is **ACTIVE (phased)** — see **Build/24_CUDA_BRAIN_V2.md**. v1 remains the fallback when no worker advertises `cuda_v2`.
