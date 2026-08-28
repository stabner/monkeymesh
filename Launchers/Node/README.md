# MonkeyMesh Node

Double-click **Start-Node.bat** (opens **MonkeyMesh-Node.exe** GUI). Headless `bin\mesh-node.exe` remains for scripts.

## What you get

- Native desktop GUI (start/stop, height, rewards, read-only soft-update history, miner activity feed)
- P2P node (libp2p QUIC) via `mesh-node.exe`
- REST wallet RPC + block explorer
- Default seed peer: `seednode.hashmonkeys.cloud:39001`

## Config (`config.json`)

| Key | Meaning |
|-----|---------|
| `listen` | P2P listen `host:port` |
| `rpc` | REST bind `host:port` (local explorer) |
| `connect` | Seed peer (`seednode.hashmonkeys.cloud:39001`) |
| `orch` | AI job board on node RPC (`seednode.hashmonkeys.cloud:18080`) |
| `mine` | `true` to auto-mine while serving |
| `miner_key` | Coinbase payout key (defaults to same as `wallet`) |
| `operator_address` | Reward wallet (`mesh01…`) for useful-work node pay. Also set on the Earnings tab. |
| `operator_vault` | Optional vault JSON — reads plaintext `address` only (never unlocked) |
| `chain` / `wallet` / `p2p_key` | Paths relative to this folder |

Cold operator precedence on the node: `--operator-address` → `MESH_OPERATOR_ADDRESS` → `--operator-vault` / `MESH_OPERATOR_VAULT` → hot wallet.

See **Build/20_SEED_NODE.md** and `Launchers/network.json`.

Local explorer: http://127.0.0.1:18080/  
Seed explorer: http://seednode.hashmonkeys.cloud:18080/

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Node exits immediately | Seed must be `host:port` e.g. `seednode.hashmonkeys.cloud:39001` |
| `store error: unexpected end of file` | Corrupt `data/chain.bin` — delete and restart |
| Won't reach seed over DNS | Check WAN/hairpin for `seednode.hashmonkeys.cloud:39001` (public DNS only) |
| Height stuck at 0 while seed is high | Wait for sync; keep Mine off until synced |
