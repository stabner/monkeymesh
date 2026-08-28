# MonkeyMind Marketplace (MVP stub)

**Status: SHELVED.** GPU power targets self-adaptive research (Build/21), not user marketplace jobs.

Historical stub flow (Build/12):

```
User -> POST /v1/marketplace/jobs
Orchestrator queue -> GPU worker
Verified result -> job status Done
Receipt -> node /v1/aireceipt (GPU 40% market)
```

Code stubs may remain so existing smokes do not break; do not extend as a product.

Prefer: `Launchers/smoke-adaptive-auto.ps1` / `Launchers/smoke-research.ps1`.
