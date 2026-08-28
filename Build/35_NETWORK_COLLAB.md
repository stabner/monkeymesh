# Network collab — CPU miners + GPU farms (Build/35)

**Status: helper floor OFF** — product truth `Build/33_WHITEPAPER.md`. Live pay is **45 / 45 / 10**. Set `MESH_HELPER_FLOOR_HEIGHT=1` only if you want the old exam split back.

Goal: a GPU hall and a CPU box on the same mesh **help each other**, without pretending the chain can see which silicon ran a hash.

## What is waterproof

Fusion rematches **functions**, not chips (`Build/34`). Every valid block still needs `cpu_fold` and `gpu_wave` on the **same pad**. A node can prove the digest. It cannot prove “this came from an AMD card in Sweden.”

Therefore:

| Want | Waterproof? | What we do |
|------|-------------|------------|
| One hash, both lanes | Yes | Fusion rematch (already live) |
| GPU farm plugs into pool/chain | Yes | Same GBT / submit as today |
| CPU miners help that farm over the network | Yes, as **different work** | Immune exam MATCH on the same node ledger |
| Split one nonce: CPU fills here, GPU mixes there, over WAN | No | Pad is 16–64 MiB per nonce; verify ≈ fill; trust is not consensus |
| Self-reported “I am a CPU share” vs “I am a GPU share” | No | Anyone can lie |

Do **not** ship mixed pads across the public internet as a consensus feature. A farm may pair fill machines and GPUs on **its own LAN**. That is an operator choice. The chain still rematches the finished digest.

## How they help each other

1. **GPU farm** (6 cards → pool) mixes the pad. Host **CPU seals** the Fusion digest on the live tip. Finder takes the **Fusion seal 45%**.
2. **CPU miners** pull the same pool/node template and **MATCH the immune exam** (tiny deterministic sim). The seed/edge **rematches**. Forged digests pay 0.
3. **GPU work 45%** goes to the **finder**. Exam MATCH does not take a slice of this lane unless `MESH_HELPER_FLOOR_HEIGHT` is set.

```
Block subsidy
├── 45% Fusion seal  → finder wallet (CPU rematch on this tip)
├── 45% GPU work     → finder wallet
└── 10% nodes        → attested node work (or vault)
```

Gate: `helper_floor_active` = fair split **and** height ≥ `MESH_HELPER_FLOOR_HEIGHT` (default **off** / `u64::MAX`).

## What miners must do

- GPU farm: mine the pool, keep **exam MATCH** on (same payout address).
- CPU miners: mine **the same pool** (or the same edge that builds templates) so exam scores sit on the **same** `gpu_scores` ledger. Different node = different coinbase.
- One exam per address per height. Many CPU wallets → many slices of the exam floor.

Exam submit is already proxied: miner → pool → edge `/v1/exam/submit`.

## What this is not

- Not PPLNS of all Fusion hashrate into both 45s (a GPU farm would vacuum the CPU lane).
- Not a second difficulty for CPU vs GPU.
- Not “AI finds the block.”
- Not pad-exchange stratum. If we ever add LAN pair mining, it stays **outside** consensus: optional, signed, rematch still required.

## Code

| Piece | Where |
|-------|--------|
| Floor BPS / height | `mesh_types::HELPER_EXAM_FLOOR_BPS`, `helper_floor_active` |
| Coinbase split | `build_market_coinbase` → `gpu_lane_helper_outputs` |
| Exam rematch | `POST /v1/exam/submit` |
| Why pads don’t go on-wire | `Build/34` §8 |
| Payout labels | `mesh_types::coinbase_payout_label`, `GET /v1/getrewards` |

## Showing what they were paid for

Coinbase is several pots, not one “you got 45 MESH” line. New blocks tag the GPU split in the memo (`pomc:v1:{h}:{n_gpu}:{n_node}|exam:{n_exam}`). The prefix is still what consensus parses. The suffix is display-only.

| Output | Typical share of 50 MESH | Paid for |
|--------|--------------------------|----------|
| Fusion seal · 45% | 22.5 | CPU sealed the Fusion digest on this tip |
| GPU work · helper share | part of 22.5 | Rematched immune exam MATCH |
| GPU work · 45% | rest of GPU lane | GPU mix credit (finder) |
| Node work · 10% | 5 | Attested node useful work |

Where it shows:

- Miner GUI — lifetime by lane + last pay, and a log line `Paid 11.25 MESH — Immune exam · GPU floor`
- Wallet **Receive** — same breakdown + recent pays with mature/immature
- Wallet **History** — each coinbase output to this address is labeled
- Explorer address lookup / tx / block — pills on every coinbase output
- Testnet pool page — per-block CPU / exam / Fusion / node columns

`GET /v1/getrewards?address=` returns `by_lane` + `recent[]` with `title` and `paid_for`. The HTTPS pool proxies that route so the miner can use the same mine target.

