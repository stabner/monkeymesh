# MonkeyMesh GPU Miner

**Status: ACTIVE** — GUI `MonkeyMesh-Miner.exe` / CLI `mesh-miner-gpu`.

GPU does **both** (optional AI):

1. **MeshHash-Fusion mix** — same digest the CPU rematch verifies — shown as GPU PoW H/s
2. **Immune exam sidecar** — one named `protocol_eval` per height from the template; seed rematches. After height 1 this (plus Fusion lane B) is the GPU 45%. Optional brain jobs no longer move the pot.

From height **80**, consensus is Fusion (`Build/32`). A GPU-only or CPU-only farm is weaker than a normal PC. AI is **not** required to find a block.

## Binaries

- `mesh-miner-gpu` / `mesh-miner-gpu-gui` (`MonkeyMesh-Miner.exe`)
- Stage: `Launchers\stage-platform-releases.ps1` → `Releases\Windows\{Miner,GpuMiner}`

## Backends

- NVIDIA CUDA (when `nvcc` is present at build)
- AMD / other OpenCL
- CPU fallback for the sequential lane

## PoW path (current)

1. Work seed = `H(commitment || recipe || prev_hash)` (Build/31)
2. Parallel Blake3 pad fill on host (several cores; leaves the rest for the CPU miner)
3. Bulk upload → GPU **forward + reverse** mix (chunked mix must carry register `state`)
4. Bulk download → parallel fold / Fusion wave
5. CPU rematch of any winner → `submitblock` (HTTPS pool or edge RPC)

A GPU H/s far below the CPU usually means fill is single-thread, mix state was reset per chunk, or reverse still ran on the host. That is a bug.

## Mine target

Public: `https://eu.hashmonkeys.cloud` (`rpc` in config — not `stratum+tcp`, not raw `:12500`).  
Coinbase pays **Your address**.

## Markets (height ≥ 1)

- Height ≥ 1: **45% CPU finder / 45% GPU exam+Fusion / 10% nodes**
- Nodes keep **10%** (useful work)
- Genesis only: 40/40/20
