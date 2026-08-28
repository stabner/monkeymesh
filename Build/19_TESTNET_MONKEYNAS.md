# MonkeyMesh public testnet / seed

Official DNS: **`seednode.hashmonkeys.cloud`** (see Build/20).

## Live endpoints

| Service | URL / addr |
|---------|------------|
| Public explorer UI | https://hashmonkeys.cloud/testnet-explorer.html |
| Public pool / connect page | https://hashmonkeys.cloud/testnet-pool.html |
| HTTPS mine target | `https://eu.hashmonkeys.cloud` (front for `:12500`) |
| Seed RPC / AI board | http://seednode.hashmonkeys.cloud:18080/ |
| Edge mine (templates/submit) | http://seednode.hashmonkeys.cloud:18081/ |
| Marketplace stub | **SHELVED** — do not treat `:18100/marketplace` as a product |
| P2P seed | `seednode.hashmonkeys.cloud:39001` |

## Host layout

| Path | Role |
|------|------|
| `~/src/MonkeyMesh` | source + cargo build |
| `~/monkeymesh-testnet/bin` | release binaries |
| `~/monkeymesh-testnet/data` | chain + keys |
| `~/monkeymesh-testnet/edge` | edge node data |
| `~/monkeymesh-testnet/logs` | node / edge / orch / worker logs |

## systemd (user) — mesh only

```bash
systemctl --user status mesh-node mesh-edge mesh-orchestrator mesh-gpu-worker
```

Do **not** stop `miningcore-crb` / `cereblixd`.

## Redeploy from Windows

```powershell
.\Launchers\testnet\deploy-monkeynas.ps1
```

## Firewall + router

UFW / router: forward `18080`, `18081`, `18100`, `39001` (udp+tcp), `39002` (udp+tcp) to the seed host.
Enable NAT loopback so LAN clients can use the DNS name.
