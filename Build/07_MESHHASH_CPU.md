# MeshHash-CPU

Goals:
- CPU friendly
- Memory hard
- Cache pressure
- Random memory access

Inspired by RandomX concepts but independently designed.

Target:
Consumer CPUs
Home servers
Xeon systems

## Profiles

### v1 (legacy full)
- Scratchpad: **16 MiB**
- Mix: **65_536** forward data-dependent rounds
- Fold: Blake3 over sampled strides

### v2 (hardened, height-gated)
- Scratchpad: **32 MiB**
- Mix: **131_072** forward rounds, then a **reverse** data-dependent pass (different constants / index function)
- Fold: same Blake3 stride fold (scales with pad size)

Activation:
- Default height **`53000`** (`DEFAULT_POW_V2_ACTIVATION_HEIGHT`)
- Override with env **`MESH_POW_V2_HEIGHT`** on every consensus node (seed, edge, edge2)
- `getblocktemplate` returns `pow_version` (`1` or `2`); miners must honor it
- Light PoW (`MESH_LIGHT_POW=1`) always uses the light v1 algorithm (tests / local only)

History below the activation height remains MeshHash v1 forever.

### v3 (MeshHash-Evo, height-gated — Build/31)

- Scratchpad / rounds / fold salt from a **period recipe** (every 2048 blocks)
- Mix: same forward+reverse family as v2 (CPU-verifiable)
- Work seed binds `header_commitment || recipe || prev_hash` so hashrate commits to this chain
- Default height **`MESH_POW_EVO_HEIGHT=1`** (wiped testnet; genesis stays v1)
- Template exposes `pow_version=3` and `pow_recipe`

See [Build/31_MESHHASH_EVO.md](31_MESHHASH_EVO.md).

### v4 (MeshHash-Fusion, height-gated — Build/32)

- Same Evo pad / rounds / work seed as v3
- Extra **GPU wavefront** bound into the final digest (32 lanes × 64 gathers)
- A valid hash needs both the sequential CPU walk and the parallel lane
- Default height **`MESH_POW_FUSION_HEIGHT=80`**

See [Build/32_MESHHASH_FUSION.md](32_MESHHASH_FUSION.md).
