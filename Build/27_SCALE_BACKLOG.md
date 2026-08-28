# Scale & build backlog

**Status: ACTIVE** — master punch-list for “bad / missing” before ~2000 miners.

Last updated: 2026-08-16 (whitepaper `Build/33`, roadmap rewrite, GPU mix + node reward-wallet honesty).

## A. Bad things — FIX

| ID | Issue | Severity | Status |
|----|-------|----------|--------|
| B1 | Single seed SPOF | P0 | **Partial** (edge-first mine + hashserver edge2 `:18083` + pool `:12500` + discovery; not multi-region DNS yet) |
| B2 | Full chain rewrite | P0 | **Done** (WAL fsync + meta/ckpt sync + cold prune + auto-prune env + nodeinfo) |
| B3 | AI poll storms | P0 | **Done** (env caps + Retry-After + client failover/backoff) |
| B4 | Open AI board | P0 | **Done** (rate limits + `MESH_AI_TOKEN_AUTO` sticky `ai.token` on Linux+Windows + `/v1/ai/health.auth_ai`) |
| B5 | Token vs mine | P0 | **Done** |
| B6–B7, B11 | Seed mine / wipe / GPU PoW | P0 | **Done** |
| B8 | Node Sybil | P1 | **Partial** (bond + soft score live + UI; geo/ASN Node Score still open) |
| B9 | Star defaults | P1 | **Done** (edge-first mine defaults + release templates + failover + Preserve-Config `rpc` sync) |
| B10, B12 | Docs / clutter | P2 | **Done** (whitepaper `33`, roadmap `13`, README/00/19/20 match live Fusion + HTTPS pool) |
| B13 | GPU PoW << CPU (false mix / tiny waves) | P0 | **Done** (carry mix `state` across CUDA chunks; parallel fill; GPU reverse; larger waves) |
| B14 | Node pay for idling / hidden payout addr | P1 | **Done** (idle reputation 0; Earnings reward wallet + `/v1/setoperator`) |

## B. Not built — BUILD

| ID | Item | Priority | Status |
|----|------|----------|--------|
| N1 | Multi-seed / edge RPC | P0 | **Partial** (monkeynas edge + hashserver edge2; DNS still single seed) |
| N2 | WAL + SNAPSHOT | P0 | **Done** (prune/ckpt/fsync/auto-prune + nodeinfo prune fields) |
| N3 | AI board scale-out | P0 | **Partial** (hybrid + sticky + health/nodeinfo shard map + client 421 pin; brain still seed-canonical) |
| N4 | Auth surfaces | P0 | **Done** |
| N5 | Node anti-Sybil bond | P1 | **Partial** (bond + soft diversity/RTT + GUI/explorer; not full geo Node Score) |
| N6 | Archive node | P1 | **Done** (honest ads + prefer archive + empty Headers/Snapshot retry) |
| N7 | Template cache | P1 | **Done** (tip+mempool+soft_diff+scores_epoch + GBT rate-limit + TTL/LRU env) |
| N8 | Topic split + durable AI | P1 | **Partial** (cursor WAL + `ai_queue.snap`) |
| N9 | Brain verify offload | P2 | **Done** (off-mutex verify + `finish_completes` + `POST /v1/results`) |
| N10 | Cold operator keys | P2 | **Done** (`MESH_OPERATOR_ADDRESS` / vault address-only / CLI / GUI / launcher) |
| N11 | MeshHash-GPU PoW | — | **Done as Fusion v4** (Build/32 — same digest, height 80) |
| N12 | MonkeyMind marketplace | — | **Shelved** |
| N13 | Governance UX polish | P2 | **Done** (explorer envelopes/epoch history + GUI soft knobs; Suggest/Approve out of scope) |

## D. Next

**Software-side scale Partials are exhausted on this host.** Remaining work needs hardware or a deliberate architecture project:

1. **B1/N1** — second public seed host (then DNS + `MESH_RPC_EDGES` / shard map / install units)  
2. **B8/N5** — geo/ASN Node Score (needs IP→geo data source)  
3. **N3** — brain multi-host replication (architecture; local snap/hybrid already live)  
4. **N8** — SQLite/rocks only if snap+cursor insufficient under load  

Operator unblock for P0 SPOF: stand up a second public seed (or accept single-NAS SPOF until then).

