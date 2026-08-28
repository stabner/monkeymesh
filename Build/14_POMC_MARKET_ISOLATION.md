# PoMC Market Isolation

**Live testnet:** height ≥ 1 is 90/10 (`Build/31`). Fusion binds CPU+GPU into one digest (`Build/32`). Product write-up: `Build/33_WHITEPAPER.md`.

## Hard rule

Competition exists only inside a market’s own score ledger.

- A GPU never increases CPU difficulty.
- A CPU never earns GPU budget.
- A node never earns miner budget by hashing.

Emission splits are protocol-fixed (basis points of each block subsidy).

Legacy (below Build/31 shared-pot height):

| Market | BPS | Share |
|--------|-----|-------|
| CPU    | 4000 | 40% |
| GPU    | 4000 | 40% |
| Node   | 2000 | 20% |

Build/31 (at/after `MESH_SHARED_BPS_HEIGHT`, default 1 on wiped testnet):

| Pot | BPS | Share |
|-----|-----|-------|
| Contributor (CPU finder + GPU receipts) | 9000 | 90% |
| Node | 1000 | 10% |

Fair lane split (at/after `MESH_FAIR_SPLIT_HEIGHT`, default **1** on wiped testnet):

| Pot | BPS | Share |
|-----|-----|-------|
| CPU / Fusion finder | 4500 | 45% |
| GPU / Fusion lane B + one exam | 4500 | 45% |
| Node | 1000 | 10% |

GPU units no longer share a denominator with the finder. One exam per address per height (`EXAM_LANE_UNITS` = 1000). Finder also receives `FUSION_GPU_UNITS` (1000) for lane B.

Changing BPS requires a height gate / human governance. AI may propose; AI cannot activate.

## Proof objects (cross-typed claims are invalid)

| Market | Proof type | Counts toward |
|--------|------------|---------------|
| CPU | MeshHash-CPU block / share | `S_cpu` only |
| GPU | `AiJobReceipt` (shared-brain / protocol sims). Fusion lane B is the **same** block digest as CPU (Build/32), not a separate GPU pot | `S_gpu` units in the 90% pot |
| Node | `NodeServiceAttestation` (relay, archive, snapshot, AI routing) | `S_node` only |

Wrong-algorithm submission = **zero credit** (not weak credit in another market).

## Score ledgers

Per epoch (or per block window):

- `S_cpu`, `S_gpu`, `S_node` — independent totals
- Pay contributor `i` in market `m`:

  `pay = subsidy * bps_m * (contrib_i / S_m)`

No global hashrate. Separate difficulty / demand knobs never transfer BPS across markets.

## Coinbase layout (economic firewall)

Every block’s coinbase **must** total exactly `block_reward(height)` and allocate:

1. `cpu_market_reward(height)` → CPU winner(s)
2. `gpu_market_reward(height)` → GPU winner(s), or **deferred GPU vault** if `S_gpu == 0`
3. `node_market_reward(height)` → Node winner(s), or **deferred node vault** if `S_node == 0`

Unclaimed GPU/Node budget **must not** be absorbed by the CPU miner. It parks in the deferred vault until that market has verified contribution.

Deterministic vault addresses:

- GPU: `MonkeyMesh/vault/gpu/v1`
- Node: `MonkeyMesh/vault/node/v1`

## Anti-Sybil (per market)

- **CPU:** PoW cost
- **GPU:** receipt verification + challenge jobs (+ stake later)
- **Node:** stake bond, reputation, geographic diversity weighting (see Build/06)

## Operational meaning

Legacy 40/40/20 isolated the three pies. After Build/31, CPU finder units and GPU receipt units share the **90%** pot; nodes keep **10%**. More of one device class dilutes **units**, not the other pot’s BPS.

## Hardware roles (client packs — live)

| Hardware | Allowed work | Paid from |
|----------|--------------|-----------|
| CPU | MeshHash / Evo / Fusion lane A | Contributor 90% (finder units) |
| GPU (CUDA/OpenCL) | Fusion lane B + mix + AI jobs | Contributor 90% (receipt units + same block) |
| Node process | Relay, AI board fill, verify both lanes | Node 10% |

Miner GUI: selected CPUs and GPUs hash MeshHash; GPUs also pull AI. Fusion (height 80) requires both lanes in one digest.
