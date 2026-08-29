# MonkeyMesh roadmap

**Status: ACTIVE** — this is the live plan. Product truth: `Build/33_WHITEPAPER.md`. Mainnet gates: `Build/28_MAINNET_READINESS.md`. Scale leftovers: `Build/27_SCALE_BACKLOG.md`.

Phases 1–4 below are **shipped**. Do not list Fusion, 90/10, or the miner GUI as “upcoming.”

## Shipped (testnet)

| Area | What actually shipped |
|------|------------------------|
| Chain | File-backed `chain.bin` + WAL, 5s target, coinbase maturity 20 |
| PoW | MeshHash-Evo v3 from height 1; **Fusion v4 live from height 80** |
| Markets | Genesis 40/40/20; height ≥ 1 is **45/45/10 isolated lanes**; GPU 45% is exam-floor + Fusion (`Build/35`) |
| Miner | All-in-one GUI: CPU + CUDA/OpenCL MeshHash + optional AI |
| Pool | HTTP GBT `mesh-pool` behind `https://eu.hashmonkeys.cloud` — pays the miner wallet |
| Wallet | egui HD vault v2 (Argon2id + XChaCha20-Poly1305); Tauri is legacy |
| Node pay | Attested useful work only; reward wallet on Earnings / `--operator-address` |
| RPC | Fail-closed wallet cookie; public mine + `submittx` stay open |
| AI | Immune exam sidecar on every template; seed rematches; soft param epochs only |
| Explorer | `https://hashmonkeys.cloud/testnet-explorer.html` — Fusion/AI jobs, not a guardian TV |

## Closed / will not revive as product

| Item | Spec | Disposition |
|------|------|-------------|
| MonkeyMind marketplace | Build/12, Build/17 | **Shelved** as a full compute exchange. A thinner **Work market** (send MESH + paid exams/brain) is live from height 39000 |
| MonkeyMesh Agent chat | Build/22 | **Removed** |
| Guardian / quantum scoreboard as the Testnet page | Build/25, Build/26 | **Retired from public UI.** Optional research jobs may still run; they are not the live work |
| AI as the only way to find a block | — | **Rejected** as the hash clock. Exam MATCH is required to *submit* from 39000; Fusion still finds the block |
| Auto BPS / tip forks from AI | Build/21 | **Forbidden** |

## Now (public testnet)

Keep the live tip honest and the miner/node story match the whitepaper.

1. **GPU PoW must look and be real** — parallel pad fill, GPU forward+reverse mix, CPU rematch of winners. A GPU far below the CPU is a bug, not a feature.
2. **Pool + wallet alignment** — coinbase = miner `address`; wallet shows spendable vs immature (20 conf).
3. **Node reward wallet** — operators paste `mesh01…` and get paid only after useful work.
4. **Public pages stay published** — explorer and pool on `https://hashmonkeys.cloud`.
5. **No casual tip wipes** — `MESH_ALLOW_WIPE=1` + confirm string.

## Next (before anyone should call this money)

Strengthen Fusion. Do not replace it. Full sequence: `Build/36_FUSION_FOUNDATION.md`.

| Priority | Work | Why |
|----------|------|-----|
| P0 | Second **independent public seed** (other region + DNS) | One seed operator is still the control plane |
| P0 | Brain / AI board failover that is not a cold copy on the same LAN | Seed-canonical brain is an SPOF |
| P1 | **Economic finality** — lab module shipped default-off (`Build/36` F2); do not arm on public tip until a second seed | 20-block reorg is still the live rule |
| P1 | External MeshHash + validation review | Internal hardening is not an audit |
| P1 | Finish Node Score + raise bond; geo/ASN (M11) | 0.1 MESH is a door fee |
| P2 | Signed release artifacts + bug bounty | Mainnet hygiene |
| P2 | 30–90 day no-wipe soak on a frozen genesis | Build/28 M19 |

## Later / research only

- Immune exam sidecar + 45/45/10 fair lanes (height 1) + helper floor (`Build/35`) — shipped; never instead of the hash clock
- Post-quantum *measurement* jobs (Build/26) — not a claim that Ed25519 is already migrated
- Marketplace — only after receipts settle on-chain without a seed blessing

## Mainnet

There is **no** mainnet tag until Build/28 P0–P2 are actually done (two public seeds, audit, soak, ceremony). Until then, say **public testnet**.
