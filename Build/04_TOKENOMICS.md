# MonkeyMesh Tokenomics

Ticker: MESH  
Status: **public testnet economics** — freeze the open decisions before mainnet.  
Companion: whitepaper `Build/33_WHITEPAPER.md` · node pay `Build/06` · market isolation `Build/14` · helper floor `Build/35`.

This file is the economics plan. If marketing copy disagrees with the **code schedule** below, the code wins until a height-gated change ships.

---

## 1. Live constants (code)

| Item | Value | Notes |
|------|--------|--------|
| Decimals | 8 | 1 MESH = 100,000,000 atomic |
| Target block time | 5 s | Retarget every 15 blocks, ±1 bit |
| Subsidy (era 0) | **50 MESH** | `block_reward` in `emission.rs` |
| Era length | 25,228,800 blocks | ≈ 4.00 years at 5 s |
| Halving | `subsidy >> era` | Era = height / 25,228,800 |
| Hard cap | **2,522,880,000 MESH** | `50 × 25,228,800 × 2`. Enforced: subsidy clamps so cumulative never exceeds |
| Coinbase maturity | 20 confirms | ≈ 100 s; `|mat:20` required from height 27,673 |
| Addresses | Ed25519 `mesh01…` | |
| Premine / treasury | **None** | Genesis is a normal 50 MESH coinbase |
| Tx fees | **None to miner** | `outputs ≤ inputs`; remainder is burned |

### Cap (decision A, 22 Aug 2026)

The old 21B figure is retired. Years 1–4 mint **50% of the real cap** (1.261B of 2.523B). `block_reward` clamps to remaining room; `/v1/getnodeinfo` and `/v1/markets` expose `supply_cap_mesh` and `emitted_atomic`.

---

## 2. Per-block split

Fusion is one digest, so there is **one finder pot**. Research units cannot move BPS. AI exams do not find the block.

| Height | Finder | Nodes |
|--------|--------|-------|
| 0 (genesis) | 40% + 40% GPU lane | 20% |
| 1 … 49999 | 45% Fusion seal + 45% GPU work (same wallet, two outputs) | 10% |
| ≥ **50000** (`DEFAULT_FINDER_UNIFY_HEIGHT`) | **90%** one output | **10%** |

| Lane (after 50000) | BPS | MESH @ 50 | Paid to | If nobody qualified |
|--------------------|-----|-----------|---------|---------------------|
| Finder (Fusion) | 9000 | 45 | Block finder | Impossible — finder is required |
| Nodes | 1000 | 5.0 | Bonded operators with attested useful work | Node vault (no spend key) |

Helper floor and exam-required **end at 50000**. Until then they stay as shipped at 39000 so the live tip does not fork. Env: `MESH_FINDER_UNIFY_HEIGHT`.

Height 0 (genesis only): 40% / 40% / 20%.

Governance clamps if a human ever height-gates a BPS change:

| Lane | Floor | Ceil |
|------|-------|------|
| Fusion seal | 25% | 50% |
| GPU work | 25% | 50% |
| Network nodes | 10% | 30% |

AI cannot activate BPS changes.

Public name after 50000 is **finder 90%**, not two miner markets. Fusion still requires both chips in one digest.

**Do not change 90 / 10 on theory.** Hardware ROI (MESH per watt on a home PC) is the only reason to height-gate BPS later.

---

## 3. Emission table (code)

Blocks / year at 5 s ≈ 6,311,520. Year-1 issuance ≈ **315,576,000 MESH**.

| Era | Years | MESH / block | MESH minted | Cumulative | % of cap |
|-----|-------|--------------|-------------|------------|----------|
| 0 | 0–4 | 50 | 1,261,440,000 | 1,261,440,000 | 50% |
| 1 | 4–8 | 25 | 630,720,000 | 1,892,160,000 | 75% |
| 2 | 8–12 | 12.5 | 315,360,000 | 2,207,520,000 | 87.5% |
| 3 | 12–16 | 6.25 | 157,680,000 | 2,365,200,000 | 93.75% |
| 4 | 16–20 | 3.125 | 78,840,000 | 2,444,040,000 | 96.9% |
| 5+ | 20+ | → 0 | tail | **2,522,880,000** | 100% |

Testnet at height 27,674 ≈ **1,383,750 MESH** out (height 0…27,674 inclusive). Treat as worthless at a mainnet ceremony unless you publish otherwise.

---

## 4. Node 10%

Paid only for attested work (`credit_local_service` → next coinbase). Idle = 0.

| Service | Weight BPS |
|---------|------------|
| Tx relay | 1000 |
| Block relay | 1500 |
| AI routing | 1200 |
| Snapshot | 2000 |
| Archive | 2000 |

Bond ≥ **0.1 MESH**, unbond cooldown **120 blocks**, slash freeze + optional settle. Pending caps: GPU 50,000 / node 25,000 / **500 per event** (a 1,000-unit exam is stored as 500).

Designed 40/20/20/20 Node Score and geo/ASN are **not live**.

Payout address: Node GUI Earnings, `--operator-address`, or `MESH_OPERATOR_ADDRESS`.

---

## 5. Vaults and burns

- GPU vault tag `MonkeyMesh/vault/gpu/v1`
- Node vault tag `MonkeyMesh/vault/node/v1`

These are deterministic addresses from ASCII tags, **not** wallets anyone holds. Unclaimed GPU/node coinbase is a **permanent sink** until a future claim/governance path exists. Decide: burn forever, or add a spend rule later (say it now).

User txs: if outputs &lt; inputs, the gap is **burned** (not a finder fee).

---

## 6. Env knobs that must die before mainnet

| Knob | Default | Risk |
|------|---------|------|
| `MESH_FAIR_SPLIT_HEIGHT` | 1 | Two nodes, two splits → invalid coinbase |
| `MESH_SHARED_BPS_HEIGHT` | 1 | Same |
| `MESH_FINDER_UNIFY_HEIGHT` | 50000 | Two nodes, two coinbase shapes → invalid blocks |
| `MESH_HELPER_FLOOR_HEIGHT` | 39000 until unify | Changes who gets 11.25 MESH before 50000 |
| `MESH_GPU_EXAM_PAY_HEIGHT` | 39000 until unify | Can vault the GPU 45% before 50000 |
| `MESH_NODE_BOND` | on | Sybil if off |

Compile-time only on mainnet.

---

## 7. Decisions still open

1. **Done (A):** cap is 2,522,880,000 and `block_reward` clamps to remaining room.
2. **Done (4 Sep 2026):** finder 90% / nodes 10% from height 50000. Helper floor and exam-required are testnet-only until that height.
3. Mainnet: compile-time 90/10. No exam gate. No helper floor.
4. Fee market vs keep burn-only remainder.
5. Vault UTXOs: burn vs later claim.
6. Premine / foundation / grants: **0%** or a published genesis allocation.
7. Testnet MESH dies at mainnet — publish that.
8. Delete the env BPS/split overrides.
9. Tail emission vs hard-zero after the 2.52B series (`Build/36`). A forever 0.5 MESH/block **raises or redefines** the cap — pick before mainnet.
10. Economic finality window (~1000 blocks) vs keep 20-block maturity as “final.”

---

## 8. What this is not

Not a price. Not a listing plan. Not a promise that testnet balances migrate. Fusion pay is coinbase accounting, not a second difficulty.
