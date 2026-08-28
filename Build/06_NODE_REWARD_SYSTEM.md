# Node Reward System

**Status: ACTIVE (phased)** — relay + AI job/result relay credits → node pot (legacy 20% / Build/31 **10%**) → node wallet UTXOs. Node GUI **Earnings** tab: balance, pending, soft score, bond, copy address, send.

Nodes earn rewards only for measurable services.

Services (implemented, weighted BPS):
| Kind | BPS (1000 = 1.0×) | Sources |
|------|-------------------|---------|
| Tx relay | 1000 | P2P import |
| Block relay | 1500 | P2P import + serving GetBlocks |
| AI routing | 1200 | P2P AI job/result + board `/v1/result` |
| Snapshot | 2000 | HTTP snapshot + P2P GetSnapshot serve |
| Archive | 2000 | HTTP archive + P2P GetHeaders serve |

Credit formula: `credited = weight × service_bps/1000 × idle_stipend_bps_cap/1000 × reputation_milli/1000 × relay_rtt_factor_milli/1000` (min 1). Requires node bond when `MESH_NODE_BOND` is on.

**Soft score (live):**
- `reputation_milli` — attestation diversity (empty→0, 1 kind→600, 2→800, 3+→1000)
- `relay_rtt_factor_milli` — local median peer RTT (≤50ms→1000, ≤200→850, else→700)
- Exposed on `/v1/noderewards`, `/v1/nodeservices`, `/v1/getnodeinfo`, Node GUI Earnings, explorer

RPC: `GET /v1/noderewards`, `GET /v1/nodeservices` (BPS table + recent attestations ring).

Node Score (design target):
40% uptime
20% bandwidth contribution
20% relay performance
20% service contribution

**Current implementation:** typed `credit_local_service` → `node_scores`; paid in the next block’s node-market share to `node_operator`. Idle nodes (no attestations) have reputation **0** and take none of the 10% pot — leftover coinbase goes to the deferred node vault. Set the payout address in the Node GUI **Earnings** tab (`POST /v1/setoperator`) or `--operator-address` / `MESH_OPERATOR_ADDRESS`. Soft diversity + RTT are the live stand-ins for service contribution / relay performance.

Anti-Sybil (v0 live):
- Min locked UTXO bond: **0.1 MESH** (`MIN_NODE_BOND_ATOMIC`) — `POST /v1/nodebond`, unbond cooldown, slash freeze
- Soft reputation (diversity) + RTT dampener
- Slash soft-assigns + optional on-chain settle (`slash:v1` memo → `deferred_slash_vault` via `/v1/nodeslashsettle`)
- SlashMark P2P gossip for multi-seed soft freeze; mark may carry preferred settle txid
- Mempool: first-seen input conflict wins; preferred SlashMark settle may replace a racing settle
- Settle body is gossiped with the mark when the local wallet owns the bond
- Per-address pending credit caps + per-event caps
- Empty-wallet relay farming earns nothing

Anti-Sybil (planned):
- Full geo / ASN diversity weighting
- Bandwidth / uptime Node Score formula
