# Shared-brain v2 — GPU train path

**Status: ACTIVE (phased)** — additive soft-gated path beside v1. No tip fork, no auto BPS.

v1 (Build/23) stays live for workers that still advertise `cpu_v1`. v2 is Q16.16 `mlp512` (`784→512→128→10`) with CPU verify on the seed and CUDA train on miners.

## Live gate behavior

Soft envelopes (defaults):

| Knob | Default | Meaning |
|------|---------|---------|
| `brain_prefer_v2` | `1` | Prefer v2 when capable workers online |
| `brain_v2_min_workers` | `1` | Min workers advertising `cuda_v2` |
| `brain_v2_vram_floor_mb` | `4096` | Count only workers at/above this VRAM |

Research tick: if prefer_v2 && `#cuda_v2 ≥ min` && brain_v2 present → enqueue **one** `ml_train_shared:v2` job; else v1.

Workers advertise:

GPU miner (trains on the card; does **not** run protocol sims on mine cores):

```json
{
  "brain_backends": ["cuda_v2"],
  "kinds": ["ml_train", "ml_train_shared"],
  "brain_contract": "v2.0.0"
}
```

CPU / no-CUDA worker (research + benches only — not the default GUI miner):

```json
{
  "brain_backends": ["cpu_v1"],
  "kinds": ["echo", "benchmark", "protocol_eval", "ml_train", "ml_train_shared"]
}
```

**Roles:** GPU trains v2 when Fusion is not mixing. Miner CPU fills pads and runs the **exam sidecar** (GPU 45% ticket). The **seed rematches** exams and brain weights (the seal). GPU miners must not CPU-fallback a v2 job when the mix lock is held.

## Contract

| Item | Detail |
|------|--------|
| Crate | `crates/mesh-ai-v2` |
| Dtype | Q16.16 `i32` (i64 mul) — CPU and CUDA bit-exact |
| Arch | `mlp512` |
| Wire | `mesh-mltrain-shared:v2:epoch=…:steps=…:lr_milli=…:samples=…:offset=…:arch=mlp512:dtype=q16` |
| Weights | magic `MESHBRAINv2`; persist `data/shared_brain_v2.bin` |
| Model API | `GET /v1/model?ver=2`, `GET /v1/model/meta?ver=2` |
| CUDA | Auto when `nvcc` present (`cfg(mesh_brain_cuda)`); `MESH_BRAIN_CUDA=0` disables |

## Goal

| v1 | v2 |
|----|-----|
| ~400 KB MLP, CPU train | ~0.5M Q16 weights, CUDA train + CPU verify |
| Node verifies by re-running CPU train | Same numeric contract (CPU ref always) |
| PoW owns the GPU | PoW + train share VRAM (~40% train workspace) |

## Non-goals

- Tip forks as the activation mechanism (Build/21)
- Auto-moving BPS (90/10 is a height gate, not AI)
- OpenCL v2 twin (later)
- Multi-gigabyte foundation models

## Phased delivery

| Phase | Deliverable | Status |
|-------|-------------|--------|
| A | Contract crate + golden tests | **done** |
| B | Soft knobs + advertise fields | **done** |
| C | CUDA backend matching goldens | **done** (when nvcc) |
| D | Seed enqueue v2 when gate satisfied | **done** |
| E | Parallel v2 by `train_slots` + heavier steps/samples (≤1024/2048) | **done** |
| F | Soft `brain_audit_every` for light jobs; brains always full-verify | **done** |
| G | OpenCL / multi-GPU train split | later |

## Dual-connect ops

Windows `MonkeyMesh-Miner` with `ai_research: true` mines the pool for hash and pulls AI from seed `:18080`. Advertise real VRAM + `cuda_v2`. Re-advertise automatically on `unknown worker` after seed board wipe.
