MonkeyMesh Miner (portable GUI pack)
====================================

Same all-in-one GUI as Releases\Windows\Miner.

Live payout (height ≥ 1): 90% contributor pot / 10% nodes.

  CPU  → MeshHash / Evo / Fusion lane A
  GPU  → Fusion lane B + mix + shared-brain AI jobs
  From height 80 both lanes are required in one digest (Build/32).
  From height 29,000 sequential Fusion (v5) binds the CPU seal to the GPU ticket.
  Official CPU-only miners refuse v5 — hop this GPU pack before then.

Contents
  MonkeyMesh-GpuMiner.exe       - desktop GUI
  mesh-miner-gpu-cli.exe       - headless CLI
  cudart64_*.dll               - CUDA runtime (NVIDIA)
  vcruntime140*.dll / msvcp140.dll / concrt140.dll
  config.json
  Start-GpuMiner.vbs / Start-GpuMiner.bat

Setup
  1. Install GPU drivers.
  2. Point RPC at the seed (http://seednode.hashmonkeys.cloud:18080).
  3. Double-click MonkeyMesh-GpuMiner.exe or Start-GpuMiner.vbs → check CPU and/or GPUs → Start.

OpenCL.dll comes from your GPU driver. Keep CUDA DLLs beside the exe.
