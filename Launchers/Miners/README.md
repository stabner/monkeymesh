# MonkeyMesh Miners (lab / legacy scripts)

Prefer the distributable packs:

- `Releases/Windows/CpuMiner/` — `MonkeyMesh-CpuMiner.exe` + `Start-CpuMiner.bat`
- `Releases/Windows/GpuMiner/` — `MonkeyMesh-GpuMiner.exe` + `cudart64_*.dll` + `Start-GpuMiner.bat`
- `Releases/Windows/Miner/` — all-in-one GUI

Flat `Releases/{CpuMiner,GpuMiner,Miner}/` are **redirect stubs** only (`MOVED.txt` → Windows packs).

Stage or refresh them with:

```
.\Launchers\stage-platform-releases.ps1
# or miner-only:
.\Launchers\stage-miner-releases.ps1
```

## This folder

Scripts under `Launchers/Miners/` still work against `bin\` for local testing.
Set payout `"address"` in `config.json` (edge-first `rpc` by default).
