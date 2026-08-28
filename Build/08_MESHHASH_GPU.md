# MeshHash-GPU

**Status: ACTIVE** — mix accelerator + Fusion lane B (`Build/32`). Same digest as CPU verify.

## Design

- Scratchpads are **VRAM-resident** for the parallel batch (up to 64 pads / 1 GiB, VRAM-capped)
- Host **parallel-fills** pads (Blake3), bulk H2D, GPU forward mix, GPU reverse mix (v2+), bulk D2H, parallel fold
- Mix-round slices must persist `state` in a device buffer — reloading `pad[0:8]` each chunk yields a **wrong hash**
- CUDA and OpenCL share that path

## Support

- CUDA (primary when `nvcc` is available)
- OpenCL (AMD and other GPUs)

## Not pursued

- Full Blake3 fill on GPU (needs bit-exact parity with the node)
- A separate “GPU 40%” pot — Build/31 folded that into the shared 90% contributor ledger
- GPU-only consensus (Fusion requires both lanes)
