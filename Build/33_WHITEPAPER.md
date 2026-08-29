# MonkeyMesh Whitepaper

**Ticker:** MESH  
**Consensus:** Proof of Mesh Contribution (PoMC) + MeshHash-Fusion  
**Status:** public testnet — not mainnet, not a claim against BTC/ETH/SOL/Kaspa  
**Companion:** roadmap `Build/13_ROADMAP.md` · readiness `Build/28_MAINNET_READINESS.md`

This document is the product truth. If another spec disagrees with it, this one wins until that spec is updated.

## 1. What this is

MonkeyMesh is a home-PC coin and a small adaptive compute mesh.

- **CPU and GPU together** find blocks (one Fusion digest — not two independent chains).
- **GPU mixes the pad; CPU seals the Fusion digest** on the live tip. Finder takes **45% Fusion seal** and **45% GPU work**. Nodes take **10%**. Pads are **not** shipped over WAN.
- **Every Fusion template assigns one immune exam** (named protocol sim). The node **CPU-rematches** it. Extra MNIST/brain jobs do not move BPS.
- **Nodes earn only for useful work** (relay, AI routing, snapshot, archive), paid to a wallet the operator sets.
- **AI never moves** emission split, opcodes, or the tip. Soft knobs only.

It is **not** a foundation-model trainer, a chat agent, or “AI finds the block.” From height 39000 it **is** a paid homework mesh: exam MATCH to submit, helper-floor MESH, and a Work market for sending tips.

## 2. Why Fusion (not RandomX, not KawPow)

| Design | Typical winner | Typical loser |
|--------|----------------|---------------|
| CPU-only (RandomX-like) | CPUs / botnets | GPUs sit idle |
| GPU-only (KawPow-like) | Rental GPU farms | Home CPUs sit idle |
| Two separate PoWs | Whichever market is cheaper to attack | The other market is ignored |

**Fusion rule:** one digest, two lanes, both required.

1. **Lane A (CPU, latency-hard)** — data-dependent walk on a 16 / 32 / 64 MiB pad (MeshHash-Evo).
2. **Lane B (GPU, bandwidth-hard)** — 32 wavefronts × 64 gathers over that pad. A CPU re-checks it cheaply.

```
work_seed = H(header_commitment || period_recipe || prev_hash)
cpu_fold  = salted Blake3 sample of the mixed pad
gpu_wave  = Fusion wavefront
digest    = H(cpu_fold || gpu_wave || salt || pad_len || "v4")
```

A warehouse of only CPUs or only GPUs is strictly weaker than a normal gaming PC. Live testnet: `pow_version = 4` from height **80**; **v5 sequential** (GPU wave → CPU seal → fuse) from height **29,000**.

## 3. How a block is found

1. Miner pulls a template (HTTPS pool `https://eu.hashmonkeys.cloud` or edge RPC `:18081`).
2. **GPU work** — mix + Fusion wave on the pad bound to that tip. If the tip moves, the job is dropped.
3. **CPU work** — seal bound to that GPU ticket (v5). The seal cannot be computed first.
4. **Fuse** — one digest. That is the block. Official miners refuse CPU-only after v5.
5. Coinbase pays the **miner wallet in the template** (`address` / `?address=`), not a pool treasury.
6. Coinbase is **immature for 20 confirmations**.

Target spacing: **5 seconds**. Difficulty retargets on a human-gated schedule. Soft AI may nudge practice intensity, never BPS.

**AI is not the Fusion clock.** The hash still finds the block. From height **39000** the **immune exam MATCH is required to submit** — a block without homework is rejected. That is useful work on the tip, not a second PoW.

## 4. How AI is paid (and verified)

| Job | Who runs it | Who verifies | What it may change |
|-----|-------------|--------------|--------------------|
| Immune exam (1 per address/height) | Required to submit from 39000 | Same deterministic sim on the node | Helper-floor MESH (half of GPU 45%) |
| Fusion GPU work | Block finder (GPU wave) | In the Fusion digest | GPU 45% (finder needs exam MATCH) |
| Shared brain / protocol eval | Miner research (on by default) + seed stepper | Seed re-runs | Brain epoch **and** research units into the helper floor |

After height **1**: **45% Fusion seal / 45% GPU work / 10% nodes**. From **39000** the GPU 45% splits: exam/brain helpers vs Fusion finder. Research still cannot move BPS, opcodes, or difficulty.

Honesty: these are small, bit-exact MLPs and protocol sims — not ChatGPT-scale training. The explorer **Market** tab shows epoch, paid exams, and how to send MESH for a check.

## 5. How nodes are paid

The **10% node pot** is split by attested useful work only:

| Service | Examples |
|---------|----------|
| Tx / block relay | P2P import and serve |
| AI routing | Job/result relay, board `/v1/result` |
| Snapshot / archive | Serving history to peers |

Idle reputation is **0**. If nobody attested work, that block’s node share goes to the deferred node vault — not to a process that is merely online.

Operators set a **reward wallet** (`mesh01…`) in the Node GUI Earnings tab, or `--operator-address` / `MESH_OPERATOR_ADDRESS`. Bond: **≥ 0.1 MESH** on that address. Pending credits already earned stay on the previous address.

## 6. Tokenomics (live testnet)

| Item | Value |
|------|--------|
| Ticker | MESH |
| Cap | **2,522,880,000** (50 MESH × 25,228,800 × 2; not 21B) |
| Block time | 5 s |
| Coinbase maturity | 20 blocks |
| Height 0 | 40% CPU / 40% GPU / 20% node (genesis only) |
| Height ≥ **1** | **45% Fusion seal / 45% GPU work / 10% nodes** (same finder gets both 45s) |
| PoW | Fusion v4 from 80; **v5 sequential** (GPU then CPU then fuse) from 29,000 |
| Addresses | Ed25519, `mesh01…` |
| Vault | BIP39 + SLIP-0010; v2 Argon2id + XChaCha20-Poly1305 (15-char floor) |

BPS and crypto upgrades stay **human height gates**. AI cannot vote them.

## 7. Live network (testnet)

| Role | Where |
|------|--------|
| Public site / explorer UI | `https://hashmonkeys.cloud` (testnet pages) |
| HTTPS mine target | `https://eu.hashmonkeys.cloud` → pool `:12500` (not stratum) |
| Seed RPC / AI board | `http://seednode.hashmonkeys.cloud:18080` |
| Edge mine RPC | `http://seednode.hashmonkeys.cloud:18081` |
| P2P seed | `seednode.hashmonkeys.cloud:39001` |
| Wallet | Native egui (`mesh-wallet-gui`) — Tauri tree is legacy |
| Miner | `MonkeyMesh-Miner.exe` (CPU + NVIDIA + AMD + optional AI) |

Do not point WAN miners at raw `:12500`. Do not treat HTN / miningcore / cereblix as this chain.

## 8. Known flaws (do not paper over)

1. **Single-operator control plane.** Seed + canonical AI brain are still one operator. The second host on the same LAN is an edge, not a geographic second seed. A wipe or outage still hurts everyone.
2. **File-backed `chain.bin`.** WAL and prune exist; this is not a multi-region production ledger.
3. **Fork choice is shallow.** Depth-1 tip replace by PoW work is too soft to call money final.
4. **AI board is seed-canonical.** Verify cost ≈ train cost for brains. That is fine for *credit*, fatal as the *only* block condition.
5. **GPU PoW can look “broken” next to CPU H/s** if pad fill stays single-thread or mix state is wrong. The miner must keep GPU mix bit-exact with CPU rematch and feed the card in parallel.
6. **Node Sybil is partial.** Bond + diversity + RTT are live; geo/ASN score is not.
7. **Marketplace, Agent chat, and the guardian scoreboard-as-product are out.** Specs that still read like a shipping storefront or a live four-guardian TV are wrong.
8. **No external audit, no bug bounty, no mainnet genesis ceremony.**

## 9. What we will not claim

- AI solves the blockchain trilemma
- AI finds blocks or moves emission
- Quantum guardians mean we are post-quantum
- This testnet competes with major L1s
- A GPU at 2 H/s vs a CPU at 370 H/s is “working as designed”

## 10. Specs

| Doc | Role |
|-----|------|
| `Build/13_ROADMAP.md` | Shipped / next / mainnet gates |
| `Build/36_FUSION_FOUNDATION.md` | Keep Fusion; finality + nodes next |
| `Build/28_MAINNET_READINESS.md` | Production checklist |
| `Build/32_MESHHASH_FUSION.md` | Fusion digest |
| `Build/34_FUSION_SECURITY.md` | Why Fusion is hard to cheat |
| `Build/31_MESHHASH_EVO.md` | Period recipe + 90/10 |
| `Build/06_NODE_REWARD_SYSTEM.md` | Useful-work node pay |
| `Build/09_WALLET_RPC.md` | Auth surfaces |
| `Build/21_SELF_ADAPTIVE_MINING.md` | Soft AI policy |
| `Build/20_SEED_NODE.md` | Seed / edge / pool ops |
