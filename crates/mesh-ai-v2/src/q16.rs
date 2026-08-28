//! Q16.16 fixed-point helpers (deterministic, no float in hot path).

pub const FRAC: i32 = 16;
pub const ONE: i32 = 1 << FRAC;

#[inline]
pub fn qmul(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> FRAC) as i32
}

#[inline]
pub fn qadd(a: i32, b: i32) -> i32 {
    a.saturating_add(b)
}

#[inline]
pub fn qsub(a: i32, b: i32) -> i32 {
    a.saturating_sub(b)
}

#[inline]
pub fn qrelu(x: i32) -> i32 {
    x.max(0)
}

/// `milli / 1000` as Q16 (e.g. 50 → 0.05).
pub fn q_from_milli(milli: u32) -> i32 {
    // (milli * ONE) / 1000 with rounding
    let n = milli as i64 * ONE as i64;
    ((n + 500) / 1000) as i32
}

/// Deterministic Q16 in roughly [-scale, scale] from blake3 lanes.
pub fn seed_q(seed: u64, lane: u64, i: u64, scale: i32) -> i32 {
    let mut buf = [0u8; 24];
    buf[..8].copy_from_slice(&seed.to_le_bytes());
    buf[8..16].copy_from_slice(&lane.to_le_bytes());
    buf[16..24].copy_from_slice(&i.to_le_bytes());
    let h = *blake3::hash(&buf).as_bytes();
    let u = u32::from_le_bytes(h[..4].try_into().unwrap());
    let span = (scale as i64) * 2;
    if span <= 0 {
        return 0;
    }
    ((u as i64 % span) - scale as i64) as i32
}

#[inline]
pub fn clamp_q(x: i32) -> i32 {
    x.clamp(-ONE * 8, ONE * 8)
}
