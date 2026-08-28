# MonkeyMesh Launchers (in-repo lab)

Dev-friendly folders next to the source tree. For **shipping** portable zips, use:

```
Releases\Windows\...
Releases\Ubuntu\...
```

```powershell
.\Launchers\stage-platform-releases.ps1
```

| Folder | What it launches |
|--------|------------------|
| `Node/` | Full node GUI + headless binary |
| `Wallet/` | Desktop GUI wallet + optional CLI |
| `Miners/` | Lab miner scripts (prefer Releases\Windows\*Miner) |
| `Orchestrator/` | Local orch + GPU research worker |

## First-time setup

```powershell
.\Launchers\build-release.ps1
```

This builds release binaries, fills `Launchers\*\bin`, and stages `Releases\Windows` + `Releases\Ubuntu`.
