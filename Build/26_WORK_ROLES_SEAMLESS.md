# Work roles & seamless loop

**Status: ACTIVE** — market-isolated client roles + node AI board that keeps GPUs busy.

## Who does what

```
CPU miners  ──getblocktemplate/submitblock──►  Node RPC :18080  ──► contributor pot (legacy CPU 40% / Build/31 90% units)
GPU workers ──advertise/job/result──────────►  Node AI board     ──► contributor pot (legacy GPU 40% / Build/31 90% units)
Hosted nodes ──P2P relay + board fill────────►  Node market (legacy 20% / Build/31 10%)

Build/31: the period recipe **assigns** `pow_cpu` / `ai_gpu` / `protocol` / `verify_assist`.  
Build/32: MeshHash-Fusion requires **both** CPU latency and GPU bandwidth lanes in one digest (height 80). GPU also still runs AI jobs into the shared pot.
Build/35: CPU miners on the **same pool/node** MATCH exams; that exam floor is half of GPU 45% when a GPU farm (or anyone) finds the block. Not WAN pad-shipping.
```

| Role | Binary / pack | Work | Must not |
|------|---------------|------|----------|
| CPU miner | Miner (CPU check) / CpuMiner | MeshHash / Evo / Fusion lane A | Skip Fusion verify after height 80 |
| GPU miner | Miner (GPU check) / GpuMiner | Fusion lane B + mix accelerate | Ignore work seed / recipe |
| GPU worker | AiWorker / Miner AI loop | Shared brain, Trilemma, protocol sims | Change BPS or catalog |
| Node | Node pack / seed | Chain, P2P, fill AI queue, verify both lanes | Steal GPU for PoW under load |

## AI models (canonical)

All loaded on the node beside the wallet path:

| Model | File | Purpose |
|-------|------|---------|
| Shared brain v1 | `shared_brain.bin` | MNIST MLP — default network brain |
| Shared brain v2 | `shared_brain_v2.bin` | Q16 mlp512 — soft-gated when CUDA workers advertise |
| Trilemma legs | `leg_brains.bin` | Security / Network / Blocks / Transpar specialists |

Workers fetch weights from `/v1/model`, `/v1/model?ver=2`, `/v1/leg/{leg}`. Only the verifying node advances brains after `/v1/result`.

## Node keeps miners busy

- Research tick every **2s** when workers are advertised (8s if idle)
- Queue depth scales with train slots (cap 128)
- Advertise triggers an immediate board refill
- Fill order: Trilemma legs → shared brain (one epoch) → protocol sims

## Client enforcement

`mesh-miner-gpu` PoW path accepts CUDA/OpenCL for MeshHash mix + Fusion lane B. GUI: selected CPUs and GPUs hash; GPUs also pull AI jobs.
