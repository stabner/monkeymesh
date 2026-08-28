# MonkeyMesh — master spec (index)

**Status: INDEX** — vision in one page; **product truth is `Build/33_WHITEPAPER.md`.**

Ticker: MESH  
Consensus: Proof of Mesh Contribution (PoMC) + MeshHash-Fusion

## Intent

A decentralized compute network where:

- CPU and GPU **together** secure the tip (Fusion — one digest)
- Every miner runs one rematched immune exam per height (soft knobs only)
- Nodes earn **only** for useful infrastructure work, to a wallet the operator sets

## Live markets (testnet)

- Height 0: 40 / 40 / 20
- Height ≥ **1**: **45% CPU / 45% GPU / 10% nodes**
- Fusion v4 from height **80**
- Coinbase maturity **20**
- Target block time **5 s**

## Document map

| Doc | Read it for |
|-----|-------------|
| `Build/33_WHITEPAPER.md` | What the project is, what it is not, known flaws |
| `Build/13_ROADMAP.md` | Shipped / next / mainnet |
| `Build/28_MAINNET_READINESS.md` | Production checklist |
| `Build/32_MESHHASH_FUSION.md` | Dual-lane hash |
| `Build/34_FUSION_SECURITY.md` | Why Fusion is hard to cheat (replay, bypass, 51%) |
| `Build/31_MESHHASH_EVO.md` | Period recipe + 90/10 units |
| `Build/06_NODE_REWARD_SYSTEM.md` | Useful-work node pay |
| `Build/09_WALLET_RPC.md` | Auth surfaces |
| `Build/21_SELF_ADAPTIVE_MINING.md` | Soft-only AI policy |
| `Build/20_SEED_NODE.md` | Seed / edge / pool |
| `Build/12`, `Build/17`, `Build/22` | Shelved / removed — not the product |

## Stack (live testnet)

Rust, libp2p QUIC, file-backed `chain.bin` (+ WAL), REST (`:18080` / edge `:18081`), HTTPS pool front, egui wallet (`mesh-wallet-gui`).

Legacy only: RocksDB, gRPC, Tauri wallet frontend.
