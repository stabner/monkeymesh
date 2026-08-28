//! MeshHash-Fusion — dual-lane CPU + GPU consensus hash.
//!
//! pow_version = 4 (height ≥ 80): both lanes on the mixed pad, then bind.
//! pow_version = 5 (height ≥ 29_000): **sequential and fair**
//!   1. GPU work — bandwidth-hard Fusion wave → `gpu_wave`
//!   2. CPU work — latency-hard seal bound to that ticket → `cpu_fold`
//!   3. Fuse — one digest. The CPU seal cannot start before the GPU ticket exists.

use mesh_types::Hash;

/// Independent wavefront lanes (SIMD / CUDA-thread shaped).
pub const FUSION_LANES: usize = 32;
/// Gather + ALU steps per lane.
pub const FUSION_STEPS: usize = 64;

/// Default height. Live testnet was ~59 when Fusion shipped — activate after
/// binaries roll out. Override with `MESH_POW_FUSION_HEIGHT`.
pub const DEFAULT_POW_FUSION_ACTIVATION_HEIGHT: u64 = 80;

pub fn pow_fusion_activation_height() -> u64 {
    match std::env::var("MESH_POW_FUSION_HEIGHT") {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return DEFAULT_POW_FUSION_ACTIVATION_HEIGHT;
            }
            t.parse::<u64>()
                .unwrap_or(DEFAULT_POW_FUSION_ACTIVATION_HEIGHT)
        }
        Err(_) => DEFAULT_POW_FUSION_ACTIVATION_HEIGHT,
    }
}

pub fn fusion_active(height: u64) -> bool {
    height >= pow_fusion_activation_height()
}

/// Sequential Fusion (GPU wave → CPU seal → fuse). Override with `MESH_POW_FUSION_V5_HEIGHT`.
pub const DEFAULT_POW_FUSION_SEQUENTIAL_HEIGHT: u64 = 29_000;

pub fn pow_fusion_sequential_height() -> u64 {
    match std::env::var("MESH_POW_FUSION_V5_HEIGHT") {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return DEFAULT_POW_FUSION_SEQUENTIAL_HEIGHT;
            }
            t.parse::<u64>()
                .unwrap_or(DEFAULT_POW_FUSION_SEQUENTIAL_HEIGHT)
        }
        Err(_) => DEFAULT_POW_FUSION_SEQUENTIAL_HEIGHT,
    }
}

pub fn fusion_sequential_active(height: u64) -> bool {
    height >= pow_fusion_sequential_height()
}

/// Program words for one Fusion wave (Blake3 of salt || pad[0:32] || "mesh-fusion").
pub fn fusion_program_words(fold_salt: u64, pad_head: &[u8]) -> [u64; FUSION_LANES] {
    let mut buf = Vec::with_capacity(8 + 32 + 12);
    buf.extend_from_slice(&fold_salt.to_le_bytes());
    let n = pad_head.len().min(32);
    buf.extend_from_slice(&pad_head[..n]);
    buf.extend_from_slice(b"mesh-fusion");
    let h = Hash::digest(&buf);
    fusion_program_words_from_digest(&h)
}

/// Expand a 32-byte digest into 32 lane program words.
pub fn fusion_program_words_from_digest(h: &Hash) -> [u64; FUSION_LANES] {
    let b = h.as_bytes();
    let mut out = [0u64; FUSION_LANES];
    for (i, slot) in out.iter_mut().enumerate() {
        let off = (i * 2) % 24;
        let mut w = [0u8; 8];
        w[..8].copy_from_slice(&b[off..off + 8]);
        *slot = u64::from_le_bytes(w) ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    out
}

#[inline]
fn alu(op: u8, a: u64, b: u64) -> u64 {
    match op & 7 {
        0 => a.wrapping_add(b),
        1 => a ^ b,
        2 => a.rotate_left(((b & 63) as u32).max(1)) ^ b,
        3 => a.wrapping_mul(b | 1),
        4 => (a & b).wrapping_add(a | b),
        5 => a.wrapping_sub(b.rotate_left(11)),
        6 => a.rotate_right(((b & 63) as u32).max(1)).wrapping_add(b),
        _ => a ^ b.wrapping_mul(0xD6E8_FEB8_6659_FD93),
    }
}

/// 32-byte Fusion wave accumulator (Blake3 of this is the GPU-lane digest).
pub fn fusion_wave_acc(pad: &[u8], fold_salt: u64) -> [u8; 32] {
    assert!(pad.len() >= 64);
    fusion_wave_acc_with_prog(pad, &fusion_program_words(fold_salt, &pad[..32.min(pad.len())]))
}

/// Same as [`fusion_wave_acc`] when program words are already computed.
pub fn fusion_wave_acc_with_prog(pad: &[u8], prog: &[u64; FUSION_LANES]) -> [u8; 32] {
    assert!(pad.len() >= 64);
    let mask = (pad.len() - 8) as u64;
    let mut acc_bytes = [0u8; 32];

    for lane in 0..FUSION_LANES {
        let mut acc = prog[lane] ^ (lane as u64).wrapping_mul(0xA076_1D64_78BD_642F);
        for step in 0..FUSION_STEPS {
            let idx = ((acc ^ prog[step % FUSION_LANES]).wrapping_mul(0x94D0_49BB_1331_11EB)
                & mask) as usize
                & !7;
            let word = u64::from_le_bytes(pad[idx..idx + 8].try_into().unwrap());
            let op = ((prog[lane] >> ((step & 7) * 3)) ^ (step as u64)) as u8;
            acc = alu(op, acc, word);
            let idx2 = ((acc.rotate_left(17) ^ (step as u64)) & mask) as usize & !7;
            let word2 = u64::from_le_bytes(pad[idx2..idx2 + 8].try_into().unwrap());
            acc = acc.wrapping_add(word2).rotate_left(7);
        }
        let lane_bytes = acc.to_le_bytes();
        for (i, x) in lane_bytes.iter().enumerate() {
            acc_bytes[i % 32] ^= *x;
            acc_bytes[(i + lane) % 32] = acc_bytes[(i + lane) % 32].wrapping_add(*x);
        }
    }

    acc_bytes
}

/// Bandwidth-hard wave over an already-mixed pad. Deterministic.
pub fn fusion_wave(pad: &[u8], fold_salt: u64) -> Hash {
    Hash::digest(&fusion_wave_acc(pad, fold_salt))
}

/// Bind sequential CPU fold + parallel GPU wave into one consensus digest.
pub fn fold_fusion(pad: &[u8], cpu_fold: Hash, fold_salt: u64) -> Hash {
    fold_fusion_with_wave(cpu_fold, fusion_wave(pad, fold_salt), fold_salt, pad.len())
}

/// GPU wave first, then CPU seal bound to that ticket, then one digest (`v5`).
pub fn fold_fusion_sequential(pad: &[u8], fold_salt: u64) -> Hash {
    let gpu_wave = fusion_wave(pad, fold_salt);
    let cpu_fold = cpu_seal_after_gpu(pad, fold_salt, gpu_wave);
    fold_fusion_v5(cpu_fold, gpu_wave, fold_salt, pad.len())
}

/// Latency-hard CPU seal. Includes `gpu_wave`, so this stage cannot run first.
pub fn cpu_seal_after_gpu(pad: &[u8], fold_salt: u64, gpu_wave: Hash) -> Hash {
    let mut hasher = blake3::Hasher::new();
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
    hasher.update(gpu_wave.as_bytes());
    hasher.update(b"cpu-v5");
    Hash::from_bytes(*hasher.finalize().as_bytes())
}

/// Same seal from a GPU packed-sample extract.
pub fn cpu_seal_from_packed(
    packed: &[u8],
    pad_len: usize,
    fold_salt: u64,
    gpu_wave: Hash,
) -> Hash {
    let stride = (pad_len / 1024).max(32);
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
    hasher.update(gpu_wave.as_bytes());
    hasher.update(b"cpu-v5");
    Hash::from_bytes(*hasher.finalize().as_bytes())
}

pub fn fold_fusion_v5(cpu_fold: Hash, gpu_wave: Hash, fold_salt: u64, pad_len: usize) -> Hash {
    let mut buf = Vec::with_capacity(80);
    buf.extend_from_slice(cpu_fold.as_bytes());
    buf.extend_from_slice(gpu_wave.as_bytes());
    buf.extend_from_slice(&fold_salt.to_le_bytes());
    buf.extend_from_slice(&(pad_len as u64).to_le_bytes());
    buf.extend_from_slice(b"v5");
    Hash::digest(&buf)
}

/// Same as [`fold_fusion`] when the GPU-lane digest is already known.
pub fn fold_fusion_with_wave(
    cpu_fold: Hash,
    gpu_wave: Hash,
    fold_salt: u64,
    pad_len: usize,
) -> Hash {
    let mut buf = Vec::with_capacity(80);
    buf.extend_from_slice(cpu_fold.as_bytes());
    buf.extend_from_slice(gpu_wave.as_bytes());
    buf.extend_from_slice(&fold_salt.to_le_bytes());
    buf.extend_from_slice(&(pad_len as u64).to_le_bytes());
    buf.extend_from_slice(b"v4");
    Hash::digest(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fusion_deterministic() {
        let pad = vec![7u8; 4096];
        let a = fusion_wave(&pad, 99);
        let b = fusion_wave(&pad, 99);
        assert_eq!(a, b);
        let c = fusion_wave(&pad, 100);
        assert_ne!(a, c);
        assert_eq!(a, Hash::digest(&fusion_wave_acc(&pad, 99)));
    }

    #[test]
    fn sequential_needs_gpu_ticket_before_cpu_seal() {
        let pad = vec![3u8; 4096];
        let gpu_a = fusion_wave(&pad, 1);
        let gpu_b = fusion_wave(&pad, 2);
        let seal_a = cpu_seal_after_gpu(&pad, 1, gpu_a);
        let seal_b = cpu_seal_after_gpu(&pad, 1, gpu_b);
        assert_ne!(seal_a, seal_b, "CPU seal must change when the GPU ticket changes");
        let fused = fold_fusion_sequential(&pad, 1);
        let manual = fold_fusion_v5(seal_a, gpu_a, 1, pad.len());
        assert_eq!(fused, manual);
        assert_ne!(fused, fold_fusion(&pad, seal_a, 1));
    }
}
