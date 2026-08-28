# Genesis + tip freeze ceremony (Build/29 / M4)

**Status: LAB EXECUTED** — `Build/genesis-mesh-public-testnet.json` published from live tip.
Mainnet ceremony still open until geographic second seed + soak + signing.

## Why this exists

Public testnet tips can be wiped for lab reasons. Mainnet (and any “money tip”) must not.
This doc is the operator checklist so a freeze is deliberate, reproducible, and published.

## Lab freeze (equipment on hand — Aug 2026)

| Field | Value |
|-------|--------|
| Network | `mesh-public-testnet` |
| Status | `lab-freeze` |
| Genesis | `63d073e87981fdcd2e7457692e8f3f2662c8058ec70054327840bb84c6b668d5` |
| Height | `37110` |
| Tip | `1c36b82ef4c840675976f781cfea2d480c60df3136ee9c85cafa459b26ceb098` |
| Seed | `http://seednode.hashmonkeys.cloud:18080` / P2P `seednode.hashmonkeys.cloud:39001` |
| Edge | `http://seednode.hashmonkeys.cloud:18081` |
| Brain standby | hourly sync → hashserver `~/monkeymesh-edge2/data/*.bin` |
| Artifact | `Build/genesis-mesh-public-testnet.json` |
| Publisher | `.\Launchers\publish-tip-freeze.ps1` |

**Honesty:** same LAN/power domain — survives single-host software failure / disk loss via hashserver copy, **not** a site outage. Off-site VPS upgrades this to geographic M1.

## Re-publish

```powershell
.\Launchers\smoke-production-health.ps1 -MaxTipLag 0
.\Launchers\publish-tip-freeze.ps1
```

## Soft AI rule (non-negotiable)

AI may only touch soft knobs. Never BPS, subsidy, fork choice, or block validity from AI output.

## Before first real mainnet ceremony

- [ ] Off-site second seed (geographic M1)
- [ ] Signed release keys (M6 complete)
- [ ] External audit schedule (M9)
- [ ] Bug bounty draft (M18)
- [ ] 30-day no-wipe soak from this lab freeze (M19 started informally)
