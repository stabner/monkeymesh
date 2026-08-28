# Proof of Mesh Contribution (PoMC)

**Status: LIVE on public testnet** — details in `Build/33_WHITEPAPER.md`.

## Markets

| Height | Split |
|--------|--------|
| 0 (genesis) | 40% CPU / 40% GPU / 20% node |
| ≥ **1** | **45% CPU / 45% GPU / 10% nodes** (isolated lanes) |

Node share is paid only for **attested useful work**. Idle nodes get 0; leftover node coinbase goes to the deferred vault.

BPS moves only at a **human height gate**. AI cannot change it.

## How the tip is secured

From height **80**, MeshHash-Fusion (`Build/32`) is one digest. Security questions (bypass, replay, 51%): `Build/34_FUSION_SECURITY.md`.

- CPU lane: sequential pad walk
- GPU lane: parallel wavefront
- Every full node re-checks both

CPU-only or GPU-only farms are weaker than a home PC with both. After height 1, exams cannot eat the CPU 45%. GPU 45% is split: exam helpers (network CPUs that MATCH) vs Fusion finder credit (`Build/35`). Exams are **not** required to find a block.

Older one-liner (“CPU creates blocks, GPU provides secondary proofs”) is **wrong** under Fusion. Both lanes are the same hash.

## Clock

Target block time: **5 seconds**. Coinbase maturity: **20** confirmations.
