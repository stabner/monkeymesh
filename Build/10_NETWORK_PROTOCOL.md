# P2P Network

**Status: ACTIVE (phased)** — QUIC + gossipsub/RR. PeerList dial, AiJob/AiResult gossip, ServiceAds live.

Transport:
QUIC

Discovery:
libp2p seeds + PeerList gossip + Hello listen_addr dial + ServiceAds (archive/snapshot preference)

Message Types:
HELLO / HELLO_ACK
PEER_LIST
SERVICE_ADS (archive, snapshot, tx_relay, block_relay, ai_routing)
TX
BLOCK
GET_HEADERS / HEADERS
GET_SNAPSHOT / SNAPSHOT
GET_BLOCKS / BLOCKS
SLASH_MARK
FINALITY_ATTEST (Build/36 F2 — genesis-bound vote; default off)
AI_JOB
AI_RESULT

Topics:
- `monkeymesh/chain/1` — blocks, txs, slash marks, finality attests, service ads
- `monkeymesh/ai/1` — AI job / result
- `monkeymesh/1` — legacy catch-all (still published for mixed fleets)

Notes:
- Seed is the canonical AI board (shared brain / legs). Non-seed nodes mirror non-brain jobs and import AI receipts for market scores; they do not advance the shared brain from gossip alone.
- Catch-up prefers peers advertising `archive` / `snapshot` for GetHeaders / GetSnapshot (pruned nodes and large gaps).
- Serving headers/snapshots/blocks credits the local node market when bonded.
- `FINALITY_ATTEST` is signature-gated (`mesh-final:v2` + genesis). Peers re-gossip new votes and SlashMark on equivocation. `MESH_FINALITY_HEIGHT` default off — do not arm on the public tip.
