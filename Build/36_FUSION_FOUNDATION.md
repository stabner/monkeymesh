# Strengthen Fusion (do not replace it)

**Status: ACTIVE plan** — public testnet. Not a new PoW. Not mainnet.  
Companions: whitepaper `Build/33`, readiness `Build/28`, roadmap `Build/13`, Fusion `Build/32` + `Build/34`, node pay `Build/06`, tokenomics `Build/04`.

Keep the distinctive identity: **CPU lane + GPU lane = one Fusion digest, one difficulty, one winner, ~5 s blocks.** Spend the next year on finality, node incentives, and honest markets. Do not invent a replacement hash.

---

## What stays (Layer 1 — already live)

| Keep | Why |
|------|-----|
| One digest, one difficulty | Two independent PoWs can be ignored; Fusion cannot |
| 90% finder / 10% nodes from height 50000 | One Fusion pay line; 45/45 labels retired |
| Official miner: GPU wave → CPU seal → fuse (v5 @ 29,000) | Fair path; custom CPU rematch still possible |
| AI is not the clock | Useful compute pays *beside* Fusion, never instead of it |
| No premine / no ICO / no founder allocation | Fair launch |

Do not change BPS on a slide. Do not put exams, storage, or rendering in `validate_block_header`.

---

## What is actually weak (and is solvable)

| Weakness | Today | Upgrade |
|----------|--------|---------|
| Shallow finality | Reorg up to 20 blocks (~100 s) | Economic finality after a depth window + attestations |
| Node incentives | 10% pot, 0.1 MESH bond, diversity+RTT only | Real Node Score + higher bond + geo/ASN |
| Outsourcing | Pad shipping still allowed | Friction + sequential v5; not a crypto ban |
| Security budget | Hard cap, subsidy → 0 | Decide tail emission *before* mainnet |
| Governance | Env knobs + unused token-vote text (`Build/11`) | Human height gates + miner/node activation, no whale DAO |

Second public seed (other region) is still P0. A finality committee on one LAN is theater.

---

## Sequenced build (do in order)

### Year-now — Foundation (before anyone calls this money)

**F1. Second independent seed** — other host + region + DNS. `Build/28` M1 geographic. Without this, every later layer is still “trust one seed operator.”

**F2. Economic finality (Layer 2)** — this is the largest protocol upgrade worth doing.

Not “longest chain forever.” After a window (target **~1000 blocks / ~80 min**, not 100 blocks / 8 min — 8 minutes is still rental-attack cheap):

1. Fusion still picks the tip (heaviest valid work).
2. Bonded nodes **attest** the checkpoint (`mesh01` + bond + uptime).
3. When attestation weight clears a threshold, that block is **final**.
4. A reorg of a final block is consensus-invalid **and** slashes attestors who flip.

Rules that keep this from becoming a second 51%:

- Committee = bonded infrastructure nodes, not a foundation multisig
- Miners still produce blocks; they do not vote BPS
- Threshold needs **independent** operators (geo/ASN when M11 ships)
- Until ≥ 2 public seeds exist, call it **lab finality** only

**F2 protocol (default off, do not arm on the public tip).** Votes are genesis-bound (`mesh-final:v2 || genesis || height || hash`), persisted in `chain.finality.json` (checkpoint + pending), gossiped on the chain topic, pulled over HTTP, and auto-signed by a node whose **hot wallet** holds a finality-eligible bond. Attest is **signature-gated**, not a cookie. Production window / threshold / min attestors / 100 MESH floor / 200-block bond age are compile-time; only `MESH_FINALITY_HEIGHT` is an activation gate. Single-operator theater cannot lock: **2 independent attestors** required. Equivocation slashes and gossips a SlashMark. **Do not set `MESH_FINALITY_HEIGHT` on the public seed or edge** until a geographic second seed exists.

**F3. Freeze consensus env knobs** — compile-time on the mainnet binary. `MESH_FAIR_SPLIT_*`, `MESH_POW_FUSION_*`, helper floor, exam-pay.

**F4. Node Score v1 (Layer 4)** — finish `Build/06` design target (uptime / archive / relay / useful work). Raise `MIN_NODE_BOND` to something that hurts to Sybil (decide a number after testnet data; 0.1 MESH is a door fee). Geo/ASN is M11.

**F5. Soak + audit** — 30–90 day no-wipe, external Fusion/validation review, signed releases.

### After money is plausible — Markets (not consensus)

**M1. Useful compute stays Layer 3** — Fusion finds blocks. Rematched jobs earn **from the node 10% or a later extra lane**, never from difficulty. Exams, archive, snapshots already do a thin version of this. Do not reopen MonkeyMind until receipts settle without a seed blessing (`Build/12` / `Build/17` stay shelved).

**M2. Tail emission (Layer 7)** — open tokenomics decision, not a silent code change. Options: hard-zero after the 2.52B series, or a small forever tail (e.g. 0.5 MESH/block) that **raises the cap** or is paid from fees. Pick one in `Build/04` before mainnet. Monero-style tail is compatible with Fusion; it is not compatible with “cap is 2,522,880,000 forever” unless you redefine the cap.

**M3. Adaptive recipes (Layer 6)** — already live (period pad / mix / salt). Push wave structure only at a human height gate after miner soak. Enough to raise ASIC cost; not enough to break home miners every epoch.

**M4. Governance (Layer 10)** — Bitcoin-style: published proposal → node/miner activation threshold → height. **Reject** `Build/11` token-holder 40% voting as product. AI may research; AI may not activate.

### Later / research only (easy to get wrong)

**Layer 5 — move 10% → 16–20% infrastructure.** Allowed only as a height-gated BPS change after months of node vs miner ROI data. Not now.

**Layer 8 — compute marketplace.** Same as shelved MonkeyMind. Verify + pay first.

**Layer 9 — diminishing hashrate returns.** **Do not ship.** “100 H/s ≠ 10× rewards” breaks one-hash-one-chance and is a pool-centralization magnet (one entity splits identities). Prefer Fusion’s existing bet: a one-sided farm is weaker than a home PC.

---

## Honest evaluation of the v2 slide

| Layer | Verdict |
|-------|---------|
| 1 Fusion | **Keep.** This is the product. |
| 2 Finality | **Build next** (after a second seed). 8-minute finality is still too soft; aim ~1 hour+. |
| 3 Useful compute | **Already policy.** Do not put it in the digest. |
| 4 Node Score | **Finish**, don’t invent a second system. Raise the bond. |
| 5 Dynamic 42/42/16 | **Later**, data-gated. |
| 6 Recipes | **Already have.** Evolve slowly. |
| 7 Tail emission | **Decide on paper** before mainnet. Conflicts with a frozen 2.52B cap. |
| 8 Compute market | **Shelved** until verify exists. |
| 9 Diminishing H/s | **Reject.** |
| 10 Soft-fork activation | **Adopt.** Retire whale-vote language. |

---

## This week (concrete)

1. Keep Fusion v4/v5 and 45/45/10.
2. Keep edge2 + no casual wipes.
3. **F2 lab code is in** (default off). Do not arm `MESH_FINALITY_HEIGHT` on the public seed or edge.
4. Next *ops*: geographic second seed — then a lab activation height, not a surprise flip.
5. Do not start a marketplace, tail emission, or BPS retune in the same week.
