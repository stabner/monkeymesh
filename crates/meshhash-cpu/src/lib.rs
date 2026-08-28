//! MeshHash-CPU — CPU-friendly, memory-hard proof of work.
//!
//! Design goals (from Build/07_MESHHASH_CPU.md):
//! - Memory hard with random access (cache pressure)
//! - Consumer CPU oriented
//! - Independently designed (RandomX-inspired ideas, not a clone)
//!
//! Algorithm overview (v1):
//! 1. Expand seed into a scratchpad (default 16 MiB)
//! 2. Run forward mixing rounds with data-dependent reads/writes
//! 3. Fold scratchpad into a 32-byte Hash
//!
//! MeshHash v2 (height-gated): 32 MiB pad, 131_072 rounds, plus a reverse
//! data-dependent mix pass before fold.
//!
//! MeshHash-Evo v3 (Build/31): period recipe selects pad/rounds/fold salt.

mod evo;
mod fusion;
mod rate;

pub use evo::{
    evo_active, evo_work_seed, period_index, period_seed_height, pow_evo_activation_height,
    pow_search_inputs, EvoRecipe, DEFAULT_POW_EVO_ACTIVATION_HEIGHT, EVO_CATALOG_ID, EVO_PERIOD,
};
pub use rate::{format_hashrate, hashrate_fusion, RateWindow};

pub use fusion::{
    cpu_seal_after_gpu, cpu_seal_from_packed, fold_fusion, fold_fusion_sequential,
    fold_fusion_v5, fold_fusion_with_wave, fusion_active, fusion_program_words,
    fusion_sequential_active, fusion_wave, fusion_wave_acc, pow_fusion_activation_height,
    pow_fusion_sequential_height, FUSION_LANES, FUSION_STEPS, DEFAULT_POW_FUSION_ACTIVATION_HEIGHT,
    DEFAULT_POW_FUSION_SEQUENTIAL_HEIGHT,
};

use mesh_types::Hash;

/// MeshHash v1 scratchpad size: 16 MiB (power of two).
pub const SCRATCHPAD_SIZE: usize = 16 * 1024 * 1024;

/// MeshHash v1 mixing iterations.
pub const MIX_ROUNDS: usize = 65_536;

/// MeshHash v2 scratchpad size: 32 MiB.
pub const V2_SCRATCHPAD_SIZE: usize = 32 * 1024 * 1024;

/// MeshHash v2 mixing iterations (applied to both forward and reverse passes).
pub const V2_MIX_ROUNDS: usize = 131_072;

/// Light profile for tests / fast local mining demos (always v1 algorithm).
pub const LIGHT_SCRATCHPAD_SIZE: usize = 256 * 1024;
pub const LIGHT_MIX_ROUNDS: usize = 4_096;

/// Default height at which full (non-light) MeshHash switches to v2.
/// Override with `MESH_POW_V2_HEIGHT`. Set to `u64::MAX` to disable.
pub const DEFAULT_POW_V2_ACTIVATION_HEIGHT: u64 = 53_000;

/// Consensus PoW profile version (`1` = legacy, `2` = hardened).
pub fn pow_v2_activation_height() -> u64 {
    match std::env::var("MESH_POW_V2_HEIGHT") {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return DEFAULT_POW_V2_ACTIVATION_HEIGHT;
            }
            t.parse::<u64>().unwrap_or(DEFAULT_POW_V2_ACTIVATION_HEIGHT)
        }
        Err(_) => DEFAULT_POW_V2_ACTIVATION_HEIGHT,
    }
}

/// PoW version for a block height under full (non-light) MeshHash.
pub fn pow_version_for_height(height: u64) -> u8 {
    if fusion_sequential_active(height) {
        5
    } else if fusion_active(height) {
        4
    } else if evo_active(height) {
        3
    } else if height >= pow_v2_activation_height() {
        2
    } else {
        1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshHashParams {
    pub scratchpad_size: usize,
    pub mix_rounds: usize,
    /// `1` forward; `2` +reverse; `3` Evo salted fold; `4` Fusion bind; `5` GPU then CPU then fuse.
    pub version: u8,
    /// Build/31 fold salt (0 for v1/v2).
    pub fold_salt: u64,
}

impl Default for MeshHashParams {
    fn default() -> Self {
        Self::v1()
    }
}

impl MeshHashParams {
    /// Full MeshHash v1 (16 MiB / 65k forward rounds).
    pub fn v1() -> Self {
        Self {
            scratchpad_size: SCRATCHPAD_SIZE,
            mix_rounds: MIX_ROUNDS,
            version: 1,
            fold_salt: 0,
        }
    }

    /// Full MeshHash v2 (32 MiB / 131k forward + reverse).
    pub fn v2() -> Self {
        Self {
            scratchpad_size: V2_SCRATCHPAD_SIZE,
            mix_rounds: V2_MIX_ROUNDS,
            version: 2,
            fold_salt: 0,
        }
    }

    /// Light profile (tests / local demos) — always v1 algorithm.
    pub fn light() -> Self {
        Self {
            scratchpad_size: LIGHT_SCRATCHPAD_SIZE,
            mix_rounds: LIGHT_MIX_ROUNDS,
            version: 1,
            fold_salt: 0,
        }
    }

    /// Select params from light flag + block height (consensus).
    /// Evo callers should prefer [`Self::from_recipe`] so pad/rounds match the period.
    pub fn for_pow(light_pow: bool, height: u64) -> Self {
        if light_pow {
            Self::light()
        } else if evo_active(height) {
            EvoRecipe::derive(height, &Hash::zero()).params()
        } else if pow_version_for_height(height) >= 2 {
            Self::v2()
        } else {
            Self::v1()
        }
    }

    pub fn from_recipe(recipe: &EvoRecipe) -> Self {
        recipe.params()
    }

    /// Select from explicit RPC `pow_version` (miners). Light still wins.
    /// For v3, pass the template recipe via [`Self::from_recipe`].
    pub fn from_template(light_pow: bool, pow_version: u8) -> Self {
        if light_pow {
            Self::light()
        } else if pow_version >= 3 {
            let mut p = EvoRecipe::derive(0, &Hash::zero()).params();
            if pow_version >= 5 {
                p.version = 5;
            } else if pow_version >= 4 {
                p.version = 4;
            }
            p
        } else if pow_version >= 2 {
            Self::v2()
        } else {
            Self::v1()
        }
    }
}

/// Hash a block commitment + nonce under MeshHash-CPU v1 full params.
pub fn meshhash_cpu(seed: &Hash, nonce: u64) -> Hash {
    meshhash_cpu_with_params(seed, nonce, &MeshHashParams::v1())
}

pub fn meshhash_cpu_light(seed: &Hash, nonce: u64) -> Hash {
    meshhash_cpu_with_params(seed, nonce, &MeshHashParams::light())
}

pub fn meshhash_cpu_with_params(seed: &Hash, nonce: u64, params: &MeshHashParams) -> Hash {
    assert!(params.scratchpad_size.is_power_of_two());
    assert!(params.scratchpad_size >= 64);

    let mut pad = scratchpad_for_nonce(seed, nonce, params);
    mix_scratchpad_with_params(&mut pad, params);
    fold_pow(&pad, params)
}

/// Fill an existing scratchpad for a nonce (no mix). Avoids a 16–64 MiB alloc per hash.
pub fn fill_scratchpad_for_nonce(seed: &Hash, nonce: u64, params: &MeshHashParams, pad: &mut [u8]) {
    assert!(params.scratchpad_size.is_power_of_two());
    assert!(params.scratchpad_size >= 64);
    assert_eq!(pad.len(), params.scratchpad_size);
    let mut key = [0u8; 40];
    key[..32].copy_from_slice(seed.as_bytes());
    key[32..].copy_from_slice(&nonce.to_le_bytes());
    fill_scratchpad(&key, pad);
}

/// Allocate + fill scratchpad for a nonce (no mix). Used by GPU miners.
pub fn scratchpad_for_nonce(seed: &Hash, nonce: u64, params: &MeshHashParams) -> Vec<u8> {
    let mut pad = vec![0u8; params.scratchpad_size];
    fill_scratchpad_for_nonce(seed, nonce, params, &mut pad);
    pad
}

/// Full mix for the given params (forward, and reverse when `version >= 2`).
pub fn mix_scratchpad_with_params(pad: &mut [u8], params: &MeshHashParams) {
    mix_scratchpad_forward(pad, params.mix_rounds);
    if params.version >= 2 {
        mix_scratchpad_reverse(pad, params.mix_rounds);
    }
}

/// CPU mix — forward pass only (legacy). Prefer [`mix_scratchpad_with_params`].
pub fn mix_scratchpad_cpu(pad: &mut [u8], rounds: usize) {
    mix_scratchpad_forward(pad, rounds);
}

/// After an external forward-only mix (CUDA/OpenCL), apply any remaining
/// consensus stages (v2 reverse pass). No-op for v1.
pub fn finish_pow_mix(pad: &mut [u8], params: &MeshHashParams) {
    if params.version >= 2 {
        mix_scratchpad_reverse(pad, params.mix_rounds);
    }
}

/// Fold a fully mixed scratchpad into the PoW hash.
pub fn fold_mixed_scratchpad(pad: &[u8]) -> Hash {
    fold_scratchpad_salted(pad, 0)
}

pub fn fold_mixed_scratchpad_salted(pad: &[u8], fold_salt: u64) -> Hash {
    fold_scratchpad_salted(pad, fold_salt)
}

/// Consensus fold. v5 = GPU wave → CPU seal → fuse. v4 = both lanes, then bind.
pub fn fold_pow(pad: &[u8], params: &MeshHashParams) -> Hash {
    if params.version >= 5 {
        return fold_fusion_sequential(pad, params.fold_salt);
    }
    let cpu = fold_scratchpad_salted(pad, params.fold_salt);
    if params.version >= 4 {
        fold_fusion(pad, cpu, params.fold_salt)
    } else {
        cpu
    }
}

/// Stride used by the salted Blake3 fold (`pad_len/1024`, at least 32).
pub fn fold_sample_stride(pad_len: usize) -> usize {
    (pad_len / 1024).max(32)
}

/// Number of 32-byte (or shorter last) samples hashed by [`fold_scratchpad_salted`].
pub fn fold_sample_count(pad_len: usize) -> usize {
    if pad_len == 0 {
        return 0;
    }
    pad_len.div_ceil(fold_sample_stride(pad_len))
}

/// Host/GPU packed sample buffer: `count * 32` bytes (short last sample is padded).
pub fn fold_samples_buf_len(pad_len: usize) -> usize {
    fold_sample_count(pad_len).saturating_mul(32)
}

/// Pack fold samples the GPU kernel writes: each sample occupies 32 bytes.
pub fn copy_fold_samples(pad: &[u8], out: &mut [u8]) {
    let stride = fold_sample_stride(pad.len());
    let need = fold_samples_buf_len(pad.len());
    assert!(out.len() >= need);
    let mut o = 0usize;
    let mut i = 0usize;
    while i < pad.len() {
        let end = (i + 32).min(pad.len());
        let n = end - i;
        out[o..o + n].copy_from_slice(&pad[i..end]);
        if n < 32 {
            out[o + n..o + 32].fill(0);
        }
        o += 32;
        i += stride;
    }
}

/// Blake3 fold from a packed sample buffer (does not hash the 32-byte padding).
pub fn fold_from_packed_samples(packed: &[u8], pad_len: usize, fold_salt: u64) -> Hash {
    let stride = fold_sample_stride(pad_len);
    let mut hasher = blake3::Hasher::new();
    let mut o = 0usize;
    let mut i = 0usize;
    while i < pad_len {
        let end = (i + 32).min(pad_len);
        let n = end - i;
        hasher.update(&packed[o..o + n]);
        o += 32;
        i += stride;
    }
    hasher.update(&(pad_len as u64).to_le_bytes());
    if fold_salt != 0 {
        hasher.update(&fold_salt.to_le_bytes());
    }
    Hash::from_bytes(*hasher.finalize().as_bytes())
}

/// Consensus fold when the GPU already mixed the pad and packed fold samples + wave acc.
pub fn fold_pow_from_device_extract(
    packed_samples: &[u8],
    wave_acc: &[u8; 32],
    pad_len: usize,
    params: &MeshHashParams,
) -> Hash {
    let gpu_wave = Hash::digest(wave_acc);
    if params.version >= 5 {
        let cpu = cpu_seal_from_packed(packed_samples, pad_len, params.fold_salt, gpu_wave);
        return fold_fusion_v5(cpu, gpu_wave, params.fold_salt, pad_len);
    }
    let cpu = fold_from_packed_samples(packed_samples, pad_len, params.fold_salt);
    if params.version >= 4 {
        fold_fusion_with_wave(cpu, gpu_wave, params.fold_salt, pad_len)
    } else {
        cpu
    }
}

fn fill_scratchpad(key: &[u8], pad: &mut [u8]) {
    // Expand keystream via keyed Blake3 XOF-style blocks.
    let mut counter = 0u64;
    let mut offset = 0;
    while offset < pad.len() {
        let mut block_key = [0u8; 48];
        block_key[..key.len()].copy_from_slice(key);
        block_key[40..].copy_from_slice(&counter.to_le_bytes());
        let digest = blake3::hash(&block_key);
        let take = (pad.len() - offset).min(32);
        pad[offset..offset + take].copy_from_slice(&digest.as_bytes()[..take]);
        offset += take;
        counter += 1;
    }
}

fn mix_scratchpad_forward(pad: &mut [u8], rounds: usize) {
    let mask = (pad.len() - 8) as u64;
    let mut state = u64::from_le_bytes(pad[0..8].try_into().unwrap());

    for i in 0..rounds {
        let idx_a =
            (((state ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) & mask) as usize) & !7;
        let mut a = u64::from_le_bytes(pad[idx_a..idx_a + 8].try_into().unwrap());

        let idx_b = ((a.rotate_left(17) ^ state) & mask) as usize & !7;
        let b = u64::from_le_bytes(pad[idx_b..idx_b + 8].try_into().unwrap());

        a = a
            .wrapping_add(b)
            .rotate_left((state % 63) as u32 + 1)
            ^ state.wrapping_mul(0xD6E8_FEB8_6659_FD93);

        pad[idx_a..idx_a + 8].copy_from_slice(&a.to_le_bytes());
        state = state.wrapping_add(a).wrapping_add(0xA076_1D64_78BD_642F);
    }
}

/// Reverse-direction data-dependent mix (MeshHash v2 second pass).
fn mix_scratchpad_reverse(pad: &mut [u8], rounds: usize) {
    let mask = (pad.len() - 8) as u64;
    let last = pad.len().saturating_sub(8);
    let mut state = u64::from_le_bytes(pad[last..last + 8].try_into().unwrap());

    for i in 0..rounds {
        let rev_i = (rounds - 1 - i) as u64;
        let idx_a =
            (((state ^ rev_i.wrapping_mul(0xC2B2_AE3D_27D4_EB4F)) & mask) as usize) & !7;
        let mut a = u64::from_le_bytes(pad[idx_a..idx_a + 8].try_into().unwrap());

        let idx_b = ((a.rotate_right(13) ^ state.rotate_left(7)) & mask) as usize & !7;
        let b = u64::from_le_bytes(pad[idx_b..idx_b + 8].try_into().unwrap());

        a = a
            .wrapping_mul(0x94D0_49BB_1331_11EB)
            .rotate_right((state % 63) as u32 + 1)
            ^ b.wrapping_add(state);

        pad[idx_a..idx_a + 8].copy_from_slice(&a.to_le_bytes());
        state = state.wrapping_add(a).wrapping_add(0x85EB_CA77_C2B2_AE63);
    }
}

fn fold_scratchpad_salted(pad: &[u8], fold_salt: u64) -> Hash {
    let mut hasher = blake3::Hasher::new();
    // Sample stride keeps fold cost proportional but covers whole pad.
    let stride = (pad.len() / 1024).max(32);
    let mut i = 0;
    while i < pad.len() {
        let end = (i + 32).min(pad.len());
        hasher.update(&pad[i..end]);
        i += stride;
    }
    hasher.update(&(pad.len() as u64).to_le_bytes());
    if fold_salt != 0 {
        hasher.update(&fold_salt.to_le_bytes());
    }
    Hash::from_bytes(*hasher.finalize().as_bytes())
}

/// Simple hashrate helper: hashes/sec for light params over `iterations`.
pub fn benchmark_light(iterations: u64) -> f64 {
    use std::time::Instant;
    let seed = Hash::digest(b"meshhash-bench");
    let start = Instant::now();
    let mut last = Hash::zero();
    for n in 0..iterations {
        last = meshhash_cpu_light(&seed, n);
    }
    std::hint::black_box(last);
    let secs = start.elapsed().as_secs_f64().max(1e-9);
    iterations as f64 / secs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let seed = Hash::digest(b"test");
        let a = meshhash_cpu_light(&seed, 42);
        let b = meshhash_cpu_light(&seed, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn nonce_changes_output() {
        let seed = Hash::digest(b"test");
        let a = meshhash_cpu_light(&seed, 1);
        let b = meshhash_cpu_light(&seed, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn v1_deterministic_and_differs_from_v2_light_algo() {
        // Use tiny pads via crafting params so the test stays fast.
        let seed = Hash::digest(b"v1v2");
        let v1 = MeshHashParams {
            scratchpad_size: LIGHT_SCRATCHPAD_SIZE,
            mix_rounds: 256,
            version: 1,
            fold_salt: 0,
        };
        let v2 = MeshHashParams {
            scratchpad_size: LIGHT_SCRATCHPAD_SIZE,
            mix_rounds: 256,
            version: 2,
            fold_salt: 0,
        };
        let a = meshhash_cpu_with_params(&seed, 7, &v1);
        let b = meshhash_cpu_with_params(&seed, 7, &v1);
        let c = meshhash_cpu_with_params(&seed, 7, &v2);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn from_template_selects_v2() {
        assert_eq!(MeshHashParams::from_template(false, 1).version, 1);
        assert_eq!(MeshHashParams::from_template(false, 2).version, 2);
        assert_eq!(
            MeshHashParams::from_template(false, 2).scratchpad_size,
            V2_SCRATCHPAD_SIZE
        );
        assert_eq!(MeshHashParams::from_template(true, 2).version, 1); // light stays v1
        assert_eq!(MeshHashParams::from_template(false, 4).version, 4);
    }

    #[test]
    fn chunked_forward_mix_must_carry_register_state() {
        // CUDA Stop slices mix into 4096-round kernels. `state` is a register,
        // not pad[0:8]. Reloading the prefix each chunk yields a different pad.
        fn mix_range(pad: &mut [u8], start: usize, rounds: usize, mut state: u64) -> u64 {
            let mask = (pad.len() - 8) as u64;
            for i in start..start + rounds {
                let idx_a = (((state ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) & mask)
                    as usize)
                    & !7;
                let mut a = u64::from_le_bytes(pad[idx_a..idx_a + 8].try_into().unwrap());
                let idx_b = ((a.rotate_left(17) ^ state) & mask) as usize & !7;
                let b = u64::from_le_bytes(pad[idx_b..idx_b + 8].try_into().unwrap());
                a = a
                    .wrapping_add(b)
                    .rotate_left((state % 63) as u32 + 1)
                    ^ state.wrapping_mul(0xD6E8_FEB8_6659_FD93);
                pad[idx_a..idx_a + 8].copy_from_slice(&a.to_le_bytes());
                state = state.wrapping_add(a).wrapping_add(0xA076_1D64_78BD_642F);
            }
            state
        }
        let params = MeshHashParams::light();
        let seed = Hash::digest(b"chunk-state");
        let base = scratchpad_for_nonce(&seed, 7, &params);
        let mut full = base.clone();
        mix_scratchpad_cpu(&mut full, params.mix_rounds);

        let mid = params.mix_rounds / 2;
        let mut good = base.clone();
        let st0 = u64::from_le_bytes(good[0..8].try_into().unwrap());
        let st1 = mix_range(&mut good, 0, mid, st0);
        let _ = mix_range(&mut good, mid, params.mix_rounds - mid, st1);
        assert_eq!(full, good, "carried state must match a single pass");

        let mut bad = base.clone();
        let _ = mix_range(&mut bad, 0, mid, st0);
        let reset = u64::from_le_bytes(bad[0..8].try_into().unwrap());
        let _ = mix_range(&mut bad, mid, params.mix_rounds - mid, reset);
        assert_ne!(full, bad, "reloading pad[0:8] must not match a single pass");
    }

    #[test]
    fn fill_scratchpad_for_nonce_matches_alloc() {
        let params = MeshHashParams::light();
        let seed = Hash::digest(b"fill-into");
        let a = scratchpad_for_nonce(&seed, 9, &params);
        let mut b = vec![0u8; params.scratchpad_size];
        fill_scratchpad_for_nonce(&seed, 9, &params, &mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn packed_fold_samples_match_full_pad() {
        let seed = Hash::digest(b"samples");
        let mut params = MeshHashParams::light();
        params.version = 4;
        params.fold_salt = 11;
        let mut pad = scratchpad_for_nonce(&seed, 4, &params);
        mix_scratchpad_with_params(&mut pad, &params);
        let mut packed = vec![0u8; fold_samples_buf_len(pad.len())];
        copy_fold_samples(&pad, &mut packed);
        let acc = fusion_wave_acc(&pad, params.fold_salt);
        let a = fold_pow(&pad, &params);
        let b = fold_pow_from_device_extract(&packed, &acc, pad.len(), &params);
        assert_eq!(a, b);
        params.version = 5;
        let a5 = fold_pow(&pad, &params);
        let b5 = fold_pow_from_device_extract(&packed, &acc, pad.len(), &params);
        assert_eq!(a5, b5);
        assert_ne!(a, a5);
    }

    #[test]
    fn fusion_fold_differs_from_evo() {
        let seed = Hash::digest(b"fusion");
        let mut p3 = MeshHashParams::light();
        p3.version = 3;
        p3.fold_salt = 7;
        let mut p4 = p3.clone();
        p4.version = 4;
        let a = meshhash_cpu_with_params(&seed, 3, &p3);
        let b = meshhash_cpu_with_params(&seed, 3, &p4);
        assert_ne!(a, b);
    }

    #[test]
    fn pow_version_for_height_default_gate() {
        assert_eq!(pow_version_for_height(0), 1);
        // Fresh-chain default: Evo from height 1 (skips unused v2 on a wiped tip).
        if DEFAULT_POW_EVO_ACTIVATION_HEIGHT <= 1 {
            if DEFAULT_POW_FUSION_ACTIVATION_HEIGHT <= 1 {
                assert_eq!(pow_version_for_height(1), 4);
            } else {
                assert_eq!(pow_version_for_height(1), 3);
                assert_eq!(
                    pow_version_for_height(DEFAULT_POW_FUSION_ACTIVATION_HEIGHT),
                    4
                );
                assert_eq!(
                    pow_version_for_height(DEFAULT_POW_FUSION_SEQUENTIAL_HEIGHT),
                    5
                );
            }
        } else {
            assert_eq!(
                pow_version_for_height(DEFAULT_POW_V2_ACTIVATION_HEIGHT.saturating_sub(1)),
                1
            );
            assert_eq!(pow_version_for_height(DEFAULT_POW_V2_ACTIVATION_HEIGHT), 2);
        }
    }
}
