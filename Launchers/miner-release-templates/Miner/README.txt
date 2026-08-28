MonkeyMesh All-in-One Miner
===========================

One app for both kinds of work (shared 90% contributor pot after height 1):

  Block mining  - CPU and GPUs find MeshHash / Evo / Fusion blocks
  AI research   - protocol sims + MNIST training on the node AI board

From height 80, MeshHash-Fusion requires both CPU and GPU lanes in one digest.
From height 29,000, sequential Fusion (v5) binds the CPU seal to that GPU ticket.
Official CPU-only miners refuse v5 — use this Miner pack (GPU required) before then.

The app has three tabs: Mining (rates, payout, hardware), Node (height, tip,
difficulty, peers, v5 countdown), and Events (tagged mine / node / pay / AI list).

Enable either or both, set payout address and node RPC, then Start.

The header shows CPU speed and GPU speed separately (block mining hashrate).
AI jobs are counted separately - they are not PoW hashrate.

Requirements:
- Node RPC reachable (default http://127.0.0.1:18080) with embedded AI board
- For NVIDIA PoW: keep cudart64_*.dll next to this exe
- For AMD PoW: GPU driver with OpenCL.dll

Keep all files in this folder together.
