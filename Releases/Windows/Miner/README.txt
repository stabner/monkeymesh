MonkeyMesh All-in-One Miner
===========================

Live payout from height 1: CPU 45% / GPU 45% / nodes 10%.
Fusion needs both lanes in one digest. The GPU exam is rematched; it does not find blocks.
Sequential Fusion (v5) starts at height 29,000 — hop this pack before then.
CPU-only official miners refuse v5; this GUI needs a GPU from that height.

Tabs: Mining (rates, payout, hardware) · Node (height, tip, peers, v5 countdown)
· Events (tagged mine / node / pay / AI list). The bottom strip is a short Recent feed.

  CPU       → pad-fill H/s (usually the larger number — different step)
  GPU       → VRAM mix H/s (usually lower — that is normal)
  GPU exam  → MATCH = the 45% GPU ticket for that block (same weight as Fusion)

Research sims / brain steps are optional. They do not pay extra MESH.

Select CPU and/or GPUs, set payout address + pool/RPC, then Start.

Windows: double-click MonkeyMesh-Miner.exe or Start-Miner.vbs (no Command Prompt).

Requirements:
- Mine target (default https://eu.hashmonkeys.cloud)
- NVIDIA: Game Ready/Studio driver + cudart64_*.dll next to this exe
- AMD: OpenCL driver for PoW
- Keep all files in this folder together
