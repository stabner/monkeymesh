//! MeshHash-Evo period recipe (Build/31).
//!
//! Frozen catalog: pad size, mix rounds, fold salt, role-mix tilt.
//! `period_seed` is the last block hash of the previous period (hashrate recycle).

use mesh_types::{DeviceRole, Hash};

use crate::{
    MeshHashParams, MIX_ROUNDS, SCRATCHPAD_SIZE, V2_MIX_ROUNDS, V2_SCRATCHPAD_SIZE,
};

/// Blocks per Evo period (~3h at 5s).
pub const EVO_PERIOD: u64 = 2_048;

/// Default height for `pow_version = 3`. Override with `MESH_POW_EVO_HEIGHT`.
/// Fresh testnet: 1 (genesis stays v1). Legacy public tip used 70000.
pub const DEFAULT_POW_EVO_ACTIVATION_HEIGHT: u64 = 1;

/// Catalog identifier (human/height-gated; not AI-moved).
pub const EVO_CATALOG_ID: u8 = 1;

const PAD_64_MIB: usize = 64 * 1024 * 1024;
const ROUNDS_MID: usize = 98_304;

pub fn pow_evo_activation_height() -> u64 {
    match std::env::var("MESH_POW_EVO_HEIGHT") {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return DEFAULT_POW_EVO_ACTIVATION_HEIGHT;
            }
            t.parse::<u64>().unwrap_or(DEFAULT_POW_EVO_ACTIVATION_HEIGHT)
        }
        Err(_) => DEFAULT_POW_EVO_ACTIVATION_HEIGHT,
    }
}

pub fn evo_active(height: u64) -> bool {
    height >= pow_evo_activation_height()
}

pub fn period_index(height: u64) -> u64 {
    height / EVO_PERIOD
}

/// Height of the last block of the previous period (0 if none).
pub fn period_seed_height(height: u64) -> u64 {
    let p = period_index(height);
    if p == 0 {
        0
    } else {
        p.saturating_mul(EVO_PERIOD).saturating_sub(1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvoRecipe {
    pub period: u64,
    pub scratchpad_size: usize,
    pub mix_rounds: usize,
    pub fold_salt: u64,
    /// 0..=3 — tilts role scheduler (higher = more AI / protocol).
    pub role_tilt: u8,
}

impl EvoRecipe {
    /// `period_seed` is the previous block hash (hashrate recycle into fold salt).
    /// Pad / rounds stay stable for the 2048-block period.
    pub fn derive(height: u64, period_seed: &Hash) -> Self {
        let period = period_index(height);
        let mut stable = Vec::with_capacity(9);
        stable.extend_from_slice(&period.to_le_bytes());
        stable.push(EVO_CATALOG_ID);
        let hs = Hash::digest(&stable);
        let sb = hs.as_bytes();
        let pad_sel = sb[0] % 3;
        let scratchpad_size = match pad_sel {
            0 => SCRATCHPAD_SIZE,
            1 => V2_SCRATCHPAD_SIZE,
            _ => PAD_64_MIB,
        };
        let mix_rounds = match sb[1] % 3 {
            0 => MIX_ROUNDS,
            1 => ROUNDS_MID,
            _ => V2_MIX_ROUNDS,
        };
        let role_tilt = sb[2] % 4;

        let mut rec = Vec::with_capacity(8 + 32 + 1);
        rec.extend_from_slice(&period.to_le_bytes());
        rec.extend_from_slice(period_seed.as_bytes());
        rec.push(EVO_CATALOG_ID);
        let hr = Hash::digest(&rec);
        let fold_salt = u64::from_le_bytes(hr.as_bytes()[8..16].try_into().unwrap());
        Self {
            period,
            scratchpad_size,
            mix_rounds,
            fold_salt,
            role_tilt,
        }
    }

    pub fn id(&self) -> Hash {
        let mut buf = Vec::with_capacity(8 * 4 + 1);
        buf.extend_from_slice(&self.period.to_le_bytes());
        buf.extend_from_slice(&(self.scratchpad_size as u64).to_le_bytes());
        buf.extend_from_slice(&(self.mix_rounds as u64).to_le_bytes());
        buf.extend_from_slice(&self.fold_salt.to_le_bytes());
        buf.push(self.role_tilt);
        Hash::digest(&buf)
    }

    pub fn params(&self) -> MeshHashParams {
        MeshHashParams {
            scratchpad_size: self.scratchpad_size,
            mix_rounds: self.mix_rounds,
            version: 3,
            fold_salt: self.fold_salt,
        }
    }

    /// Assign a role from advertised caps + this period's tilt (Build/31).
    pub fn assign_role(&self, has_cuda: bool, os_matches_f64: bool) -> DeviceRole {
        match (has_cuda, os_matches_f64, self.role_tilt) {
            (true, _, 0) => DeviceRole::AiGpu,
            (true, _, 1) => DeviceRole::Protocol,
            (true, _, 2) => DeviceRole::AiGpu,
            (true, _, _) => DeviceRole::VerifyAssist,
            (false, true, 3) => DeviceRole::VerifyAssist,
            (false, _, _) => DeviceRole::PowCpu,
        }
    }

    pub fn to_hex(&self) -> String {
        hex_32(self.id().as_bytes())
    }
}

fn hex_32(b: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(HEX[(x >> 4) as usize] as char);
        s.push(HEX[(x & 0xf) as usize] as char);
    }
    s
}

/// Seed + params a miner must hash. Matches consensus `pow_hash_header`.
pub fn pow_search_inputs(
    commitment: &Hash,
    light: bool,
    height: u64,
    prev_hash: &Hash,
) -> (Hash, crate::MeshHashParams) {
    if light || !evo_active(height) {
        (*commitment, crate::MeshHashParams::for_pow(light, height))
    } else {
        let recipe = EvoRecipe::derive(height, prev_hash);
        let mut params = recipe.params();
        if crate::fusion_sequential_active(height) {
            params.version = 5;
        } else if crate::fusion_active(height) {
            params.version = 4;
        }
        (
            evo_work_seed(commitment, &recipe, prev_hash),
            params,
        )
    }
}

/// v3 work seed: bind header commitment to recipe + prev hash (hashrate recycle).
pub fn evo_work_seed(commitment: &Hash, recipe: &EvoRecipe, prev_hash: &Hash) -> Hash {
    let mut buf = Vec::with_capacity(96);
    buf.extend_from_slice(commitment.as_bytes());
    buf.extend_from_slice(recipe.id().as_bytes());
    buf.extend_from_slice(prev_hash.as_bytes());
    Hash::digest(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipe_deterministic() {
        let seed = Hash::digest(b"period");
        let a = EvoRecipe::derive(70_000, &seed);
        let b = EvoRecipe::derive(70_000, &seed);
        assert_eq!(a, b);
        let c = EvoRecipe::derive(70_000 + EVO_PERIOD, &seed);
        assert_ne!(a.period, c.period);
    }

    #[test]
    fn search_inputs_recycle_prev_hash() {
        let prev = Hash::digest(b"prev");
        let commitment = Hash::digest(b"commit");
        let (seed, params) = pow_search_inputs(&commitment, false, 1, &prev);
        assert_eq!(params.version, 3);
        assert_ne!(seed, commitment);
        let recipe = EvoRecipe::derive(1, &prev);
        assert_eq!(seed, evo_work_seed(&commitment, &recipe, &prev));
        assert_eq!(params.fold_salt, recipe.fold_salt);
        let genesis = pow_search_inputs(&commitment, false, 0, &Hash::zero());
        assert_eq!(genesis.1.version, 1);
        assert_eq!(genesis.0, commitment);
    }

    #[test]
    fn windows_cuda_not_pow_by_default() {
        let r = EvoRecipe::derive(70_000, &Hash::digest(b"x"));
        let role = r.assign_role(true, false);
        assert_ne!(role, DeviceRole::PowCpu);
    }
}
