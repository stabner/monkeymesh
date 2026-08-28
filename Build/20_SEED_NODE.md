# MonkeyMesh seed node

Official seed: **`seednode.hashmonkeys.cloud`**

Hosted by the seed operator behind **`seednode.hashmonkeys.cloud`**.

## Ports

| Port | Proto | Role |
|------|-------|------|
| 39001 | UDP (QUIC) + TCP | P2P seed |
| 39002 | UDP (QUIC) + TCP | P2P edge (LAN) |
| 18080 | TCP | Seed RPC / explorer / **embedded AI job board** |
| 18081 | TCP | Edge RPC (templates + submitblock only) |
| 18100 | TCP | Optional standalone orchestrator (legacy) |

## Client defaults

| Client | Setting |
|--------|---------|
| Standalone node | `--connect seednode.hashmonkeys.cloud:39001`; RPC bind `0.0.0.0:18080` for hosted |
| Wallet / GPU AI workers | Seed: `http://seednode.hashmonkeys.cloud:18080` — seed-canonical board |
| CPU / GPU miners | Public mine target `https://eu.hashmonkeys.cloud`. LAN/lab: edge-first RPC (`:18081` before `:18080`). Coinbase = miner `address`. |
| Node GUI | **Earnings** on the node wallet; P2P PeerList mesh beyond seed-only star |

Constants: `mesh_types::{SEED_DNS, default_seed_p2p, default_seed_rpc_url, default_seed_connects, default_rpc_urls}`.

## Mesh notes

- Seed is the **canonical AI board** (shared brain / legs). Peers gossip `AiJob` / `AiResult` and PeerList; brain advances stay on the verifying seed path (N9: verify off AI mutex).
- Node market: relay + AI-relay credits → `/v1/noderewards` + Node GUI Earnings.
- **PoW:** Fusion v4 is **live** from height 80 (`MESH_POW_FUSION_HEIGHT`). Evo v3 from height 1.
- **Payout:** height ≥ 1 is 90% contributor / 10% node (`MESH_SHARED_BPS_HEIGHT=1`). Pool pays the miner wallet; node market pays `--operator-address` / `MESH_OPERATOR_ADDRESS` (useful work only).
- **Edge RPC (N1):** `mesh-edge.service` on the seed host (`MESH_EDGE_MODE=1`) syncs from seed over P2P `:39002`, serves mine templates on `:18081`, and proxies AI to seed via `MESH_AI_UPSTREAM=http://127.0.0.1:18080`.
- **Remote edge2 (B1/N1):** a second host runs `mesh-edge2` on P2P `:39002` / RPC `:18083`. Deploy: `Launchers/testnet/deploy-hashserver.ps1`. Does not touch HTN/monkeypool.
- **AI shards (N3):** seed=`MESH_AI_SHARD_ID=0`, edge=`1`, hashserver=`2`; public `MESH_AI_SHARDS` map in health/nodeinfo. GPU clients discover the map and honor `421` sticky redirects (brain stays seed-canonical).
- **Fork choice:** depth-1 competing tips at the same height resolve by PoW work (leading zeros, then block id); orphans buffered in memory.
- **Node bond (N5):** operators need ≥ 0.1 MESH liquid + `/v1/nodebond` (auto on start when funded) to earn node-market credits. Soft score: attestation diversity + local median peer RTT dampener (`getnodeinfo.median_peer_rtt_ms` / `relay_rtt_factor_milli`). Catch-up prefers lowest-RTT archive peers.
- **Reward wallet (N10):** Node GUI **Earnings** or `--operator-address` / `MESH_OPERATOR_ADDRESS`. Idle nodes earn 0. `POST /v1/setoperator` (wallet token) applies live.
  - Precedence: `--operator-address` → `MESH_OPERATOR_ADDRESS` → `--operator-vault` / `MESH_OPERATOR_VAULT` → hot wallet.
  - Vault load reads the plaintext `address` field only — the node never unlocks the vault or loads cold private keys.
  - GUI / `Launchers/Node/config.json`: `operator_address` / `operator_vault`.
  - Bond/settle still need keys that own the bonded UTXOs.
- **Cold prune (N2):** edges may set `MESH_COLD_PRUNE=1` and `POST /v1/snapshot/prune` (wallet token + `confirm=1`). Opt-in auto: `MESH_AUTO_PRUNE=1` + `MESH_KEEP_BLOCKS` (default 2048, min 128). Keep ≥1 full-history archive seed — never prune the canonical archive. WAL fsync default ON (`MESH_WAL_FSYNC=0` to relax).
- **Archive prefer (N6):** P2P catch-up prefers peers advertising `archive`/`snapshot`; empty Headers/Snapshot retries another archive peer. RPC `services` only claim archive when `!pruned`.

## Auth surfaces

See **Build/09**. Wallet/gov RPC is **fail-closed**: `$DATA/rpc.token` (or `MESH_RPC_TOKEN`) is always required. Public mine stays open. AI board defaults to sticky `$DATA/ai.token` via `MESH_AI_TOKEN_AUTO=1` (set `0` to keep open).

## DNS

`seednode.hashmonkeys.cloud` → CNAME/A to the seed operator’s public address.

## Router (required for internet peers)

Forward WAN to the seed host:

- `18080/tcp`, `18081/tcp`, `18100/tcp`, `39001/udp`, `39001/tcp`, `39002/udp`, `39002/tcp`

Enable **NAT loopback / hairpin** on the operator router so local wallets using the DNS name reach the seed.

## Seed host

- Listen P2P: `0.0.0.0:39001` (do **not** dial itself)
- Edge P2P: `0.0.0.0:39002` → connects to `127.0.0.1:39001`
- RPC: `0.0.0.0:18080` (seed) / `0.0.0.0:18081` (edge)
- Only restart `mesh-*` user units — leave miningcore / cereblix alone

```bash
systemctl --user status mesh-node mesh-edge
~/src/MonkeyMesh/Launchers/testnet/mesh-testnet.sh status
```
