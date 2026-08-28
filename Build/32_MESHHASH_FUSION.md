# MeshHash-Fusion — dual-lane CPU + GPU PoW (Build/32)

**Status: LIVE on public testnet** (height-gated, default 80)

Goal: compete as a **home-PC coin**. One chip class must not own the tip.

## What the market taught us

| Algo | Wins on | Loses to |
|------|---------|----------|
| RandomX | CPUs, ASIC-hard | Botnets, GPUs are wasted |
| ProgPoW / KawPow | GPUs, ASIC-narrow | NiceHash rental 51%, CPUs idle |
| Autolykos | GPU RAM | Same rental market |
| X16 / GhostRider chains | Short-term novelty | Static hash lists get ASICs |
| Dual independent PoW | Two miner sets | Weaker chain can be ignored |

**Fusion rule:** one digest, two jobs. GPU does one job. CPU does the other. They fuse into one block. Official miners refuse CPU-only after v5.

## Lanes

1. **GPU work (bandwidth-hard)** — mix + 32×64 Fusion wave on the pad. This ticket must exist first.
2. **CPU work (latency-hard)** — MeshHash-Evo seal **bound to that GPU ticket**. Cannot be computed before the wave.
3. **Fuse** — one digest. First valid nonce is the block.

```
# pow_version = 4 (height 80 … 28,999)
cpu_fold  = salted Blake3 sample of mixed pad
gpu_wave  = Fusion wavefront
digest    = H(cpu_fold || gpu_wave || salt || pad_len || "v4")

# pow_version = 5 (height ≥ 29,000) — sequential / fair
gpu_wave  = Fusion wavefront
cpu_fold  = salted Blake3(pad samples || salt || gpu_wave || "cpu-v5")
digest    = H(cpu_fold || gpu_wave || salt || pad_len || "v5")
```

Work seed is still `H(commitment || recipe || prev_hash)` (Build/31 recycle).

## Activation

- `pow_version = 4` at `MESH_POW_FUSION_HEIGHT` (default **80**)
- `pow_version = 5` at `MESH_POW_FUSION_V5_HEIGHT` (default **29_000**) — GPU then CPU then fuse
- History below each gate stays the previous version
- Catalog / BPS / crypto stay human-gated

## What miners do

- **CPU** mines lane A (and can verify B)
- **GPU** mixes the pad (forward+reverse on device) + lane B; optional AI in parallel
- Best rig = **both** on one machine
- GPU H/s far below CPU is a miner bug (fill/mix path), not the Fusion rule

## What Fusion does not do

- Does not clone RandomX random programs or ProgPoW DAG
- Does not let AI move BPS or opcodes
- Does not require AI jobs to find a block
- Does not replace optional GPU research (exams are rematched homework; they do not find the block)

**Why an attacker cannot skip a lane, replay, or precompute:** `Build/34_FUSION_SECURITY.md`.
