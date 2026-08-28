MonkeyMesh Desktop (Windows)

Wallet, Fusion mine, and optional local node in one app.
Double-click MonkeyMesh.exe or Start-MonkeyMesh.vbs (no Command Prompt).

  MonkeyMesh.exe       wallet, Fusion mine, optional local node
  mesh-node.exe        replica started from Network (ports 18082 / 39012)
  mesh-wallet-cli.exe  optional CLI
  config.json          wallet RPC (seed / edge). Mine target defaults to the official pool.

Wallet: unlock, send, receive.
Mine: official pool https://eu.hashmonkeys.cloud - GPU mix + CPU Fusion seal.
      CPU/GPU rates, height, v5 countdown, and a tagged event list.
Network: live node pulse (height, tip, peers, finality, supply) plus optional local replica.
      Do not Use local RPC until the sidecar catches up to the public tip.

From height 29,000 sequential Fusion (v5) needs a GPU. Official CPU-only miners refuse v5.

Keep every DLL in this folder. Do not split the pack.
