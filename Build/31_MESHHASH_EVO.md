# MeshHash-Evo, shared pot, hashrate recycle (Build/31)

**Status: LIVE** (height-gated; default activation height 1 on this testnet). Fusion v4 sits on top from height 80 (`Build/32`).

Goal: miner work strengthens **this** chain. CPU and GPU share one contributor pot. Nodes keep a small isolated slice. The period recipe assigns device roles and mutates MeshHash so ASICs/FPGAs cannot lock one circuit.

## Payout (after activation height)

History below the gate keeps 40 / 40 / 20.

| Pot | BPS | Who |
|-----|-----|-----|
| Contributor | 9000 (90%) | Block finder + verified AI receipts, split by units |
| Node | 1000 (10%) | Useful work only (relay / AI routing / snapshot / archive) — idle = 0 |

Block finder units = `CONTRIB_BLOCK_UNITS` (1000). GPU units = pending receipt weights. Pay from the 90% pot:

`pay_i = subsidy * 9000bps * (units_i / (1000 + S_gpu))`

**From height 1 (`MESH_FAIR_SPLIT_HEIGHT`):** that unit share is retired. Coinbase is **45% CPU / 45% GPU / 10% nodes**. GPU units only split the GPU 45%. The finder is auto-credited `FUSION_GPU_UNITS` for lane B. One rematched immune exam per address/height earns `EXAM_LANE_UNITS`. Other AI receipts store on the tape but credit **0**.

Node pot stays isolated. BPS does **not** auto-move (Build/11 / Build/30).

Env: `MESH_SHARED_BPS_HEIGHT` (default `1` on wiped testnet; genesis height 0 stays 40/40/20).

## Role scheduler

Devices advertise CPU / VRAM / `os_family`. The period recipe assigns a role. Credit only if the proof matches the assignment.

| Role | Typical device | Work |
|------|----------------|------|
| `pow_cpu` | CPU | MeshHash-Evo nonce search |
| `ai_gpu` | CUDA GPU | Shared brain / guardian train |
| `protocol` | any GPU | Deterministic protocol / quantum sims |
| `verify_assist` | CPU or GPU | Re-verify or snapshot assist |

Default recipes keep MeshHash on CPU (memory-hard, every node can check). From height 80, **MeshHash-Fusion** (`Build/32`) binds a CPU-verifiable GPU wavefront into the same digest (`pow_version=4`).

## MeshHash-Evo (`pow_version = 3`)

Frozen catalog. Recipe changes every `EVO_PERIOD` (2048) blocks.

```
recipe = H(period_index || period_seed || catalog_id)
```

`period_seed` = hash of the last block of the previous period (genesis if none).

Recipe selects:

- Scratchpad: 16 / 32 / 64 MiB
- Mix rounds: 65_536 / 98_304 / 131_072
- Fold salt (hashrate recycle)
- Role-mix tilt (0..=3)

v3 mix is v2 forward+reverse plus a salted fold. Light PoW stays v1.

Env: `MESH_POW_EVO_HEIGHT` (default `1` on wiped testnet; genesis stays MeshHash v1).

## Hashrate recycle

1. **Work seed** (v3 only) = `H(header_commitment || recipe_id || prev_hash)` — hashes this chain, not a disconnected puzzle.
2. **Winning fold** (block PoW hash at period end) is the next `period_seed` — honest hashrate writes the next mix graph.
3. **GPU receipts** advance brains and the Build/30 retarget gate; they share the contributor pot.
4. **Mesh strength** = recent blocks + verified AI receipts. May tighten `min_difficulty_floor` by at most 1 inside Build/30 clamps.

Failed nonces still die. Recycle applies to **accepted** hashes and **verified** research.

## What stays human / height-gated

- BPS 90/10
- New catalog opcodes
- Period length
- Signature crypto

## APIs

- `getblocktemplate`: `pow_version`, `pow_recipe`, `assigned_role`, `mesh_strength`
- `GET /v1/envelopes`: `contributor_bps`, `node_bps`, `evo`, `roles`
- `GET /v1/ai/health`: assigned role per worker
