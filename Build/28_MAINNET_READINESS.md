# Mainnet readiness (Build/28)

**Status: ACTIVE** — path from public testnet → production-hardening → mainnet freeze.

Companion lists: `Build/33_WHITEPAPER.md` (product truth), `Build/27_SCALE_BACKLOG.md` (scale), `Build/13_ROADMAP.md` (roadmap).

## Definition of done

MESH is **production-ready** when:

1. No single operator can silently wipe or rewrite the tip people rely on.
2. At least **two independent public seeds** (different hosts/regions).
3. Consensus + P2P have been **externally reviewed**; CI gates every release.
4. Soft AI never moves BPS / consensus rules (already policy — must stay enforced).
5. Tagged genesis + signed releases + 30+ day no-wipe soak.

Until then: **public testnet** only. Do not pitch as competing with BTC/ETH/SOL/Kaspa.

## Phase checklist

### P0 — Control plane (blocks everything else)

| ID | Item | Status | Notes |
|----|------|--------|-------|
| M1 | Second independent public seed | **Lab done** | Dual-host seed + edge2 tip-aligned + client discovery. Geographic off-site VPS still required for real M1 |
| M2 | Ban casual `-Wipe` on public tip | **Done** | Operator deploy needs `MESH_ALLOW_WIPE=1` + `-WipeConfirm DELETE_PUBLIC_TIP` |
| M3 | Brain multi-host replication | **Lab done** | Hourly cold standby sync seed → edge2; promote script for failover |
| M4 | Published genesis hash + tip freeze process | **Lab done** | `Build/genesis-mesh-public-testnet.json` via `Launchers/publish-tip-freeze.ps1` (lab-freeze, not mainnet) |

### P1 — Engineering maturity

| ID | Item | Status | Notes |
|----|------|--------|-------|
| M5 | CI (build + `cargo test`) | **Done** | `.github/workflows/ci.yml` |
| M6 | Release checksums / signed artifacts | **Partial** | `Launchers/write-release-checksums.ps1` (SHA256; signing later) |
| M7 | Snapshot bootstrap first-class (no 28k-block crawl) | **Done** | `Launchers/bootstrap-chain-from-seed.ps1` |
| M8 | Monitoring (tip lag, peers, AI verify_ok/fail) | **Partial** | Smoke + tip lag; RPC latency fixed (O(1) `next_difficulty`, no full-chain clone on getnodeinfo) |

### P2 — Security

| ID | Item | Status | Notes |
|----|------|--------|-------|
| M9 | External MeshHash + validation audit | **Open** | Internal 2026: vault v2, fail-closed RPC, GPU mix state fix, useful-work node pay — not a substitute for external audit |
| M10 | P2P eclipse / DoS / ban scores | **Open** | |
| M11 | Finish geo/ASN Node Score (B8/N5) | **Open** | Needs geo data source |
| M12 | Deeper fork-choice / finality story | **Partial** | Live rule is still 20-block reorg. F2 protocol is in-tree and **off** (gossip + persist + genesis bind + 100 MESH / 200-block attestor floor). Do not arm until geographic M1. |

### P3 — Product honesty / AI wedge

| ID | Item | Status | Notes |
|----|------|--------|-------|
| M13 | Keep soft-only AI policy | **Done** | Build/21 |
| M14 | Dual-connect heavy research loop | **Done** | Build/21–24 heavy path |
| M15 | Marketplace | **Shelved** | Until verify + payments exist |
| M16 | Wallet crypto UX (why BIP39/SLIP-0010) | **Done** | Build/03 + Security tab |

### P4 — Mainnet launch gate

| ID | Item | Status |
|----|------|--------|
| M17 | `v1.0.0-mainnet` tag + genesis ceremony | **Open** |
| M18 | Bug bounty | **Open** |
| M19 | 30–90 day no-wipe public soak | **Open** |

## Immediate next actions (this week)

1. Keep **edge2** + **hourly brain sync timer** online (lab M1/M3).
2. Run `.\Launchers\smoke-production-health.ps1` after deploys (tip lag ≤ 3).
3. Re-publish freeze after intentional tip moves: `.\Launchers\publish-tip-freeze.ps1`.
4. If the seed dies: run the edge2 promote script (uses cold brains).
5. Publish seed/edge RPC on the official hostnames if clients need direct edge RPC (the HTTPS pool is the public mine target).
6. No public tip wipes without `MESH_ALLOW_WIPE=1` + confirm string. Re-publish freeze after any intentional tip move.
7. After this hardening: seed/edge/edge2 HTTP-pull the highest tip; imported blocks are re-gossiped; pool submit fans out. Confirm heights match after deploy.
8. Off-site VPS later upgrades lab M1 → geographic M1 (then mainnet ceremony).

## Public endpoints (current)

| Role | Where |
|------|--------|
| Seed RPC / AI board | `http://seednode.hashmonkeys.cloud:18080` |
| Edge mine RPC | `http://seednode.hashmonkeys.cloud:18081` |
| HTTPS pool | `https://eu.hashmonkeys.cloud` |
| P2P | `seednode.hashmonkeys.cloud:39001` |
| Site | `https://hashmonkeys.cloud` |

## Explicit non-goals until P0–P2

- Auto BPS / tip forks from AI
- Relaunching MonkeyMind marketplace
- Claiming “competes with major L1s”
