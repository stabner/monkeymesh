# MonkeyMesh

<img src="branding/hmny_128.png" alt="MonkeyMesh" width="128" height="128">

**MESH** is a home-PC coin with a small adaptive compute mesh. Consensus is **PoMC** with **MeshHash-Fusion**.

This repository is the **public testnet** — not mainnet.

| | |
|---|---|
| Site | [hashmonkeys.cloud](https://hashmonkeys.cloud) |
| Explorer | [testnet explorer](https://hashmonkeys.cloud/testnet-explorer.html) |
| Pool | [connect / mine](https://hashmonkeys.cloud/testnet-pool.html) |
| Mine (HTTPS) | `https://eu.hashmonkeys.cloud` |
| Seed RPC | `http://seednode.hashmonkeys.cloud:18080` |
| P2P | `seednode.hashmonkeys.cloud:39001` |
| License | [MIT](LICENSE) |

Product truth: [`Build/33_WHITEPAPER.md`](Build/33_WHITEPAPER.md) · Roadmap: [`Build/13_ROADMAP.md`](Build/13_ROADMAP.md)

Live: Fusion **v4 from height 80**. Height ≥ 1 is **45% Fusion seal / 45% GPU work / 10% nodes**. Sequential Fusion (v5) activates at height **29,000**. Coinbase matures after **20** blocks.

Do not use raw `:12500` on the WAN. Do not use `stratum+tcp`. This is not HTN / miningcore.

## Windows apps

GUI packs are Windows apps — they should not open a Command Prompt. Double-click the `.exe`, or `Start-*.vbs`.

| Pack | What it is |
|------|------------|
| `Releases/Windows/MonkeyMesh` | Wallet + Fusion mine + optional local node |
| `Releases/Windows/Miner` | All-in-one miner GUI (CPU + NVIDIA + AMD) |
| `Releases/Windows/Node` | Node GUI (`mesh-node.exe` is the headless console binary) |
| `Releases/Windows/Wallet` | Wallet GUI + CLI |
| `Releases/Windows/CpuMiner` | CPU miner (console by design) |

```powershell
.\Launchers\stage-platform-releases.ps1
# Releases\Windows\{MonkeyMesh,Node,Miner,Wallet,CpuMiner,GpuMiner,AiWorker}
```

Miner: set **Mine target** `https://eu.hashmonkeys.cloud` and **Your address** to the HD index you actually watch.  
Node: **Earnings → Reward wallet** (`mesh01…`). Idle nodes earn nothing.  
Wallet: unlock the same vault/address the miner uses.

## Layout

| Path | Purpose |
|------|---------|
| `Build/` | Specs, whitepaper (`33`), roadmap (`13`), readiness (`28`) |
| `crates/` | Shared libraries |
| `apps/` | Node, miners, wallet GUIs, pool, workers |
| `Launchers/` | Lab scripts + ship templates |
| `Releases/` | Portable packs (`Windows/`, `Ubuntu/`) |
| `branding/` | Official marks |

## Operator deploy

Scripts in `Launchers/testnet/` talk to the seed over SSH. Set `MESH_NAS_HOST` (and optional `Launchers/testnet/local.env`) on the operator machine — those values are not stored in this repository.

Public tip wipe needs `MESH_ALLOW_WIPE=1` plus a confirm string.

## Dev

```bash
cargo build --release -p mesh-node -p mesh-miner-gpu -p mesh-wallet-gui

cargo run -p mesh-node -- --chain data/a/chain.bin serve \
  --listen 127.0.0.1:39001 --rpc 127.0.0.1:18080 \
  --wallet data/a/wallet.key --p2p-key data/a/p2p.key \
  --operator-address mesh01…
```

## Specs

Whitepaper `Build/33` · Fusion `Build/32` · Evo / 90/10 `Build/31` · seed ops `Build/20` · wallet `Build/03` · RPC `Build/09`
