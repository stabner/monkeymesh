# MonkeyMesh releases (portable packs)

Ship **one folder per app**. Each pack is standalone: binary + start script + config + runtime libs.

```
Releases/
  Windows/
    Node/            MonkeyMesh-Node.exe + mesh-node.exe + Start-Node.bat
    Wallet/          MonkeyMesh-Wallet.exe + CLI
    AiWorker/        mesh-gpu-worker — research jobs (90% pot units)
    Orchestrator/    local orch + worker (optional; seed already hosts orch)
    CpuMiner/        CPU miner pack (Fusion lane A)
    GpuMiner/        GPU miner GUI (Fusion lane B + AI)
    Miner/           all-in-one miner GUI (CPU + GPU hash + AI)
  Ubuntu/
    Node/            mesh-node + start-node.sh
    Orchestrator/    orch + gpu-worker + scripts
    CpuMiner/        mesh-miner-cpu + start-cpu-miner.sh
```

**Markets (live):** `Miner` / `GpuMiner` find MeshHash / Fusion blocks and pull AI. `AiWorker` is research-only. Height ≥ 1 pays 90% contributor / 10% node. Fusion v4 at height 80 needs CPU + GPU.
## Build / refresh

From the repo root:

```powershell
.\Launchers\stage-platform-releases.ps1
```

Or full lab + releases:

```powershell
.\Launchers\build-release.ps1
```

Ubuntu binaries are copied from `MESH_NAS_HOST` when that env is set and SSH works; otherwise scripts/config are still written.

## How to use

1. Copy e.g. `Releases\Windows\Node` anywhere.
2. Double-click `Start-Node.bat` (Windows) or `./start-node.sh` (Ubuntu).
3. Do **not** move the `.exe` / binary without its DLLs / sibling files.

`Launchers\` stays the in-repo development lab. Prefer `Releases\` for zips you give to others.

Legacy flat paths `Releases\CpuMiner` etc. only contain a pointer to `Releases\Windows\...`.
