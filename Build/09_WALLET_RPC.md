# Wallet RPC

**Status: ACTIVE** — REST on the node (`:18080`). Auth is split into surfaces (Build/27 N4/B5).

## Surfaces

| Surface | Env | Routes | Notes |
|---------|-----|--------|-------|
| **Public mine** | — | `getblocktemplate`, `submitblock` | Always open; rate-limited. Wallet token must **not** gate miners. |
| **Public mesh** | — | `submittx`, `aireceipt`, `nodescore`, `finality/attest` | Signed / scored traffic; rate-limited. Finality attest is ed25519 + 100 MESH bond + 200-block age — cookie is not a vote. |
| **Wallet / operator** | `MESH_RPC_TOKEN` (auto cookie `$DATA/rpc.token`) | `getnewaddress`, `sendtoaddress`, `setoperator`, `mine`, proposals activate/reject/vote/generate | **Fail-closed.** Bitcoin Core–style cookie: OS CSPRNG 32-byte hex, owner-only file. Bearer or `X-Mesh-Token`. `getnewaddress` never overwrites an existing key. `setoperator` points node-market credits at a `mesh01…` reward wallet. |
| **AI board** | `MESH_AI_TOKEN` | `advertise`, `job`, `result` | Independent of wallet token. Unset = open + rate limits. Default `MESH_AI_TOKEN_AUTO=1` arms sticky `$DATA/ai.token` on node start (Linux install + Windows launchers + `mesh-node`). `/v1/ai/health.auth_ai` mirrors `getnodeinfo.auth.ai`. |
| **Read** | — | `getnodeinfo`, `getbalance`, `snapshot`, `nodeservices`, explorer, AI health/meta, `finality` | Public. `getnodeinfo` also reports `finalized_height` / `finality_active` (lab; default off). |

`getnodeinfo.auth` reports which mutate surfaces are armed. `getnodeinfo.edges` lists optional edge RPC bases (`MESH_RPC_EDGES`).

## Cookie + vault (2026 hardening)

- **RPC cookie** — Bitcoin Core model. `mesh-node` mints `$DATA/rpc.token` with OS CSPRNG (32 bytes hex) and owner-only perms. Wallet/gov routes never run open.
- **Token compare** — constant-time (`subtle`), length folded in.
- **CORS** — GET/POST/OPTIONS, no credentials (Geth: never `*` + cookies).
- **Vault v2** — RFC 9106 Argon2id (64 MiB, t=3, p=4) + XChaCha20-Poly1305. NIST SP 800-63B-4 passphrase floor: 15 characters. v1 vaults still unlock; long-enough passwords upgrade on next unlock.

## Methods

getbalance · sendtoaddress · getnewaddress · setoperator · listtransactions · getnodeinfo · getrewards · noderewards · nodeservices · submitvote · getblocktemplate · submitblock · snapshot · envelopes · markets · meshpulse · proposals\* · finality · finality/attest

## Interfaces

REST (live) · CLI · gRPC (planned)
