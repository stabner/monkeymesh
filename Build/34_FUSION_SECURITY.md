# Fusion MeshHash — why it is hard to cheat

**Status: LIVE testnet documentation** (`pow_version = 4` from height 80).  
**Not an audit.** This describes what the code does today and what an attacker can still do.

Product one-liner: `Build/33_WHITEPAPER.md`. Algorithm sketch: `Build/32_MESHHASH_FUSION.md`. Period recipe: `Build/31_MESHHASH_EVO.md`. Implementation: `crates/meshhash-cpu/src/{lib.rs,evo.rs,fusion.rs}` and `crates/mesh-chain/src/validate.rs`.

---

## 1. What “secure” means here

Fusion is a **proof-of-work**. It does not hide transactions. It does not stop a majority of *Fusion* hashrate from reorging the tip.

It is meant to make these attacks expensive:

| Attack | Intended cost |
|--------|----------------|
| GPU-only farm finds blocks | Must still produce a valid CPU fold on the **same** mixed pad |
| CPU-only farm / botnet finds blocks | Must still produce a valid GPU wave on that pad |
| Reuse yesterday’s hash | Work seed includes `prev_hash` + header commitment |
| Precompute the next height | `prev_hash` is unknown until the previous block exists |
| Skip rematch | Every full node recomputes the whole digest from the header |

It does **not** make outsourcing impossible. A colluding CPU shop and GPU shop can share a 16–64 MiB pad. Fusion makes a *one-sided* warehouse weaker than a normal PC that already has both chips. That is the home-PC claim. It is not “two-party mining is cryptographically forbidden.”

---

## 2. How work is generated (one nonce)

Consensus hash for a candidate header:

```
commitment = H(version || prev_hash || merkle || timestamp || height || difficulty)
             // nonce is excluded so miners can iterate it
recipe     = period recipe from height + previous period seed   // pad size, mix rounds, fold_salt
work_seed  = H(commitment || recipe.id || prev_hash)
pad        = Blake3 expand(work_seed || nonce)                  // 16 / 32 / 64 MiB
pad        = mix_forward(pad) then mix_reverse(pad)             // data-dependent
cpu_fold   = salted Blake3 sample of the mixed pad              // lane A
gpu_wave   = Fusion wavefront over the mixed pad                // lane B
digest     = H(cpu_fold || gpu_wave || fold_salt || pad_len || "v4")
```

A block is valid only if `digest` has at least `header.difficulty` **leading zero bits**. There is one digest and one difficulty. Lanes do not have separate targets.

Live activation: `meshhash_cpu::fusion_active(height)` — default height **80**.

### 2.1 CPU work (lane A)

1. **Fill.** Blake3 keyed blocks expand `work_seed || nonce` into the scratchpad. Different nonce ⇒ different pad. This is host-side and often shows up as “CPU H/s.”
2. **Mix.** Sequential, data-dependent read/write:
   - Forward: each round’s index depends on the previous state word.
   - Reverse (v2+): a second pass from the end of the pad.
   - Round counts come from the period recipe (65 536 / 98 304 / 131 072).
   - This is the latency-hard part. A GPU can run the same mix in VRAM; a verifier can run it in DRAM. The **result must match**.
3. **Fold.** Strided Blake3 over the mixed pad plus `pad_len` and `fold_salt`. Cheap compared with the mix. This is `cpu_fold`.

The marketing line “CPU does a latency-hard walk” refers to the **mix**, not the fold.

### 2.2 GPU work (lane B)

`fusion_wave(pad, fold_salt)` in `fusion.rs`:

- 32 independent lanes, 64 gather + ALU steps each.
- Program words are derived from `fold_salt` and the first 32 bytes of the **already mixed** pad (`"mesh-fusion"` domain).
- Each step reads 8-byte words at data-dependent offsets, applies a small ALU (`add` / `xor` / `rotate` / `mul` / …), then a second gather.
- Lane accumulators collapse to 32 bytes, then Blake3.

A CPU verifies this in microseconds once the pad is mixed. A GPU is meant to mine it quickly. Skipping the wave, or running it on a different pad, changes `gpu_wave` and therefore `digest`.

### 2.3 Who may compute which step

Fill and mix are **shared**. The miner GUI may fill on the host and mix on CUDA/OpenCL. That is an implementation choice. Consensus does not care which chip mixed, only that the mixed pad is the unique function of `(work_seed, nonce, recipe)`.

---

## 3. How the two results are cryptographically bound

`fold_fusion` is a single Blake3 over:

```
cpu_fold (32) || gpu_wave (32) || fold_salt (8) || pad_len (8) || "v4"
```

Properties:

- Swap in a `cpu_fold` from another nonce → digest changes.
- Swap in a `gpu_wave` from another pad → digest changes.
- Wrong `fold_salt` or `pad_len` → digest changes.
- There is no “CPU hash OR GPU hash meets difficulty.” Only the **bound** digest is checked.

The header does not carry `cpu_fold` or `gpu_wave`. Nodes recompute both from `(commitment, nonce, height, prev_hash)`.

---

## 4. How the CPU result prevents a GPU bypass

A GPU-only attacker who mixes the pad and runs `fusion_wave` still needs the correct `cpu_fold` of **that** pad.

If they omit the fold, invent a fold, or reuse a fold from another nonce:

- `fold_fusion` does not match what the node computes.
- `meets_difficulty` is evaluated on the node’s digest, not the miner’s claimed parts.

A GPU farm that never produces a valid CPU fold cannot produce a valid block. They can hire CPUs or run the fold themselves (it is cheap after mix). What they cannot do is **ignore** the CPU lane and still pass rematch.

---

## 5. How the GPU result prevents a CPU bypass

A CPU-only attacker who fills, mixes, and folds still needs `fusion_wave` on **that** pad.

If they skip the wave or substitute a wave from another pad, rematch fails the same way.

A botnet of CPUs that never runs the wave cannot produce a valid Fusion block. They can add a GPU or outsource the wave (see §8). They cannot treat Fusion like RandomX and leave GPUs idle.

---

## 6. What prevents replay

The work seed binds this chain and this header:

```
work_seed = H(commitment || recipe.id || prev_hash)
```

`commitment` includes merkle root, height, timestamp, difficulty, and `prev_hash` again at the header layer.

So a winning `(nonce, digest)` from height *N* does not work at height *N+1*:

- `prev_hash` changed.
- `height` changed.
- Coinbase / merkle almost certainly changed.
- Recipe may have changed (every 2048 blocks).

Replaying on a fork with a different parent fails the same way. There is no standalone “Fusion puzzle” you can grind offline and later attach to an arbitrary block.

Timestamp must be strictly greater than the parent and not more than two hours in the future (`validate.rs`). That stops some toy replays; it is not a replacement for the work-seed bind.

---

## 7. What prevents precomputation

To grind height *N+1* you need `prev_hash` of height *N*. That hash is not known until *N* is found.

The period recipe is `H(period_index || period_seed || catalog_id)` where `period_seed` is the last block of the previous 2048-block period. You can know the recipe for the current period, but you still cannot precompute the **next block’s** pad because `work_seed` includes the previous block hash.

Nonce search is therefore online with the tip. That is the hashrate-recycle rule from Build/31.

---

## 8. What prevents outsourcing one lane

**Honest answer: friction, not a crypto ban.**

| Split | What you must ship | Friction |
|-------|--------------------|----------|
| Fill+mix here, wave elsewhere | Mixed pad (16–64 MiB) per nonce | Bandwidth + latency vs local GPU |
| Mix on GPU, fold on CPU | Mixed pad or the GPU stays on the same box | Fold is cheap; usually not worth splitting |
| Two warehouses, no shared pad | Impossible | Lanes are bound to one pad |

A home PC already has both chips and does not pay the pad-transfer tax. A GPU warehouse that adds a rack of CPUs *on the same fabric* can still mine. Fusion’s claim is weaker than “no pool can split roles.” It is: **a one-sided market cannot ignore the other chip class.**

If a future revision wants stronger anti-outsource, it would need a latency-bound interactive challenge or a pad that does not fit cheap WAN transfer at the target H/s. That is **not** implemented today. Do not claim it is.

---

## 9. How difficulty works

- Target spacing: **5 seconds**.
- Metric: leading zero **bits** of the Fusion digest (`Hash::meets_difficulty`).
- Genesis / first window: `INITIAL_DIFFICULTY = 10`.
- Retarget every **15** blocks by default (`RETARGET_INTERVAL`), step 1 bit, clamp 1..=48.
- Soft envelopes (Build/30) may change interval / step / floor. They must not invent a second PoW.

**CPU and GPU do not have different difficulties.** There is no “CPU target” and “GPU target.” If the GPU wave is missing, the bound digest is just wrong — it does not “almost pass” a GPU-only bar.

Pay split (45 / 45 / 10) is **coinbase accounting**, not a second difficulty. Faster GPUs cannot take the CPU 45%. They also cannot find blocks without the CPU fold. After the helper floor (`Build/35`), they also cannot keep the whole GPU 45% unless they MATCH the exam themselves — network CPU miners take the exam half.

---

## 10. How a 51% attack would look

Same shape as other PoW coins:

1. Accumulate more Fusion hashrate than the honest network.
2. Mine a secret heavier chain (more leading-zero work, then block-id tie-break — `block_pow_work`).
3. Publish it and replace the public tip.

What Fusion changes:

- Rental **GPU-only** (classic KawPow 51%) is the wrong hardware mix. The attacker still needs lane A on every nonce.
- Botnet **CPU-only** (classic RandomX) is the wrong mix. They still need lane B.
- The realistic 51% is **rent or own both**, or own mixed home-PC hashrate.

Testnet fork choice is still shallow (higher-work tip replace). F2 economic finality is in-tree and **off** (`Build/36`): genesis-bound votes, gossip, persist, 100 MESH / 200-block attestor floor. Do not arm it on one LAN. Fusion does not fix depth-1 reorgs by itself.

---

## 11. Hardware specialization

| Specialist | What they win | What they still owe |
|------------|---------------|---------------------|
| DRAM-latency ASIC (lane A mix) | Faster mix / fold | Must still emit a valid `gpu_wave` on that pad |
| Bandwidth GPU / ASIC (lane B) | Faster wave | Must still emit a valid `cpu_fold` on that pad |
| Period-locked ASIC | One recipe | Recipe mutates pad size, mix rounds, `fold_salt` every 2048 blocks |
| NiceHash GPU hours | Cheap lane B | Useless alone |

ASICs are not “impossible.” Fusion’s bet is that a **one-lane** ASIC is not enough to own the tip, and that a two-lane ASIC looks a lot like a well-balanced home PC (DRAM + wide memory). That is an economic claim, not a proof.

Light PoW (`MESH` test / demo profile) is a different, smaller pad. It is **not** the public testnet consensus path.

---

## 12. What a node actually checks

On `submitblock` / apply (`validate_block_header`):

1. Header links to parent, height +1, timestamp rules.
2. Merkle root matches the block’s transactions.
3. Difficulty equals the consensus next-difficulty (not miner-chosen).
4. Coinbase pays the full 50 MESH subsidy with the locked 45 / 45 / 10 memo.
5. Recompute `pow_hash_header(commitment, nonce, height, prev_hash)`.
6. Reject if the digest lacks the required leading zeros.

The node does not trust miner-reported H/s, AI exam results, or “I ran CUDA.” Fake GPU mix that does not match the CPU reference is an invalid block.

Optional AI exams are **not** in this path. They cannot move difficulty, BPS, or the tip.

**Emission lock:** GPU exam units are credited only after `/v1/exam/submit` rematch. `/v1/aireceipt` and P2P `AiResult` cannot mint exam share from an `exam:v1:` prefix. `/v1/nodescore` requires the wallet RPC cookie and an existing bond — public spam cannot redirect the node 10%. Spendable coins still appear only as coinbase UTXOs in a validated block.

---

## 13. Known limits (do not oversell)

- No external MeshHash audit yet.
- Leading-zero difficulty is coarse (1-bit steps).
- Fork choice on testnet is shallow.
- Pad shipping still allows split-shop mining.
- Fill is Blake3-expand, not a RandomX-style random program.
- Fusion wave is a fixed 32×64 structure; it is not a ProgPoW DAG.
- `MESH_POW_FUSION_HEIGHT` is an env gate for activation height — changing it on a live network is a hard fork.
- Local AI auto-adapt must not change `retarget_interval` / `retarget_step` / `min_difficulty_floor`. On this testnet that drift forked seed (interval 20, expected diff 9 at 1720) from edge (interval 15, kept diff 8) on the **same** block 1719. Soft bias only. Heal: `MESH_FORCE_RETARGET_INTERVAL=15` on the lagging seed, then P2P sync. No wipe.

---

## 15. What a third-party miner cannot fake

Consensus rematches the **whole** digest. A closed-source miner cannot:

- Skip `cpu_fold` or `fusion_wave` and still pass `validate_block_header`
- Mix on a different pad than the wave
- Replay a nonce from another height (`work_seed` binds `prev_hash`)
- Take the CPU 45% without being the block finder
- Invent a second difficulty for “GPU-only” or “CPU-only”

They **can** still:

- Write a faster **honest** implementation of the same hash (that is PoW)
- Run the cheap Fusion wave on CPU (it is the verifier path) — so a CPU-only box is valid, just mix-bound
- Fill on extra CPU cores while a GPU mixes (our miner already does this)
- Ship pads between a CPU shop and a GPU shop (friction, not a crypto ban — §8)

**CPU looking “harder” than the GPU is expected.** Blake3 pad fill is host work. The GPU mix is a latency-bound walk in VRAM; Task Manager SM% will pulse. Fair H/s is finished Fusion hashes, not “how hot the chip looks.”

**GPU 45% anti-vacuum:** Helper floor is **off** (`DEFAULT_HELPER_FLOOR_HEIGHT = u64::MAX`). Finder takes both 45s for one fused solution. From height **29,000** the digest is sequential: GPU wave first, CPU seal bound to that ticket, then fuse. Official miners refuse CPU-only. A custom CPU-only miner can still rematch (nodes must), but it is slower than a GPU wave. `MESH_GPU_EXAM_PAY_HEIGHT` can also withhold Fusion GPU units unless the finder MATCH’d an exam.

**Exam race:** `/v1/exam/submit` accepts the current template height **or** the height that just became the tip, and remembers the previous GBT height. A 5s block must not drop the GPU ticket.

---


## 14. Code map

| Question | Code |
|----------|------|
| Fill / mix / fold | `crates/meshhash-cpu/src/lib.rs` |
| Work seed + recipe | `crates/meshhash-cpu/src/evo.rs` |
| GPU wave + bind | `crates/meshhash-cpu/src/fusion.rs` |
| Header commitment | `crates/mesh-types/src/block.rs` `pre_pow_commitment` |
| Difficulty bits | `crates/mesh-types/src/hash.rs` `meets_difficulty` |
| Retarget | `crates/mesh-chain/src/difficulty.rs` |
| Rematch | `crates/mesh-chain/src/validate.rs` `validate_block_header` |
| Helper floor (exam vs Fusion GPU 45%) | `crates/mesh-chain/src/lib.rs` `gpu_lane_helper_outputs` |
