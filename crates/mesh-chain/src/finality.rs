//! Economic finality (Build/36 F2) — default **off**.
//!
//! Fusion still picks the tip. After a window, bonded attestors sign a
//! genesis-bound checkpoint. A finalized height cannot be popped.
//! Equivocation slashes the bond and is gossiped.
//!
//! Production knobs (window / min attestors / threshold / bond floor / age)
//! are compile-time. Only `MESH_FINALITY_HEIGHT` is an activation gate.
//! Tests may override the rest via env.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mesh_crypto::Keypair;
use mesh_types::{Address, Hash};

use crate::store::NodeBondRec;
use crate::ChainError;

pub const DEFAULT_FINALITY_HEIGHT: u64 = u64::MAX;
pub const DEFAULT_FINALITY_WINDOW: u64 = 1_000;
pub const DEFAULT_FINALITY_MIN_ATTESTORS: u64 = 2;
pub const DEFAULT_FINALITY_THRESHOLD_BPS: u16 = 6_700;
/// 100 MESH locked — door-fee 0.1 MESH bonds cannot vote.
pub const DEFAULT_FINALITY_MIN_BOND_ATOMIC: u64 = 10_000_000_000;
/// Bond must be this old (blocks) before a vote counts.
pub const DEFAULT_FINALITY_BOND_AGE: u64 = 200;
const MAX_PENDING_HEIGHTS: usize = 128;
const MAX_VOTES_PER_HEIGHT: usize = 256;
const CATCHUP_HEIGHTS: u64 = 64;

pub fn finality_activation_height() -> u64 {
    parse_u64_env("MESH_FINALITY_HEIGHT", DEFAULT_FINALITY_HEIGHT)
}

pub fn finality_window() -> u64 {
    prod_or_test_u64("MESH_FINALITY_WINDOW", DEFAULT_FINALITY_WINDOW).max(1)
}

pub fn finality_min_attestors() -> u64 {
    prod_or_test_u64(
        "MESH_FINALITY_MIN_ATTESTORS",
        DEFAULT_FINALITY_MIN_ATTESTORS,
    )
    .max(1)
}

pub fn finality_threshold_bps() -> u16 {
    prod_or_test_u64(
        "MESH_FINALITY_THRESHOLD_BPS",
        u64::from(DEFAULT_FINALITY_THRESHOLD_BPS),
    )
    .min(10_000) as u16
}

pub fn min_finality_bond_atomic() -> u64 {
    prod_or_test_u64(
        "MESH_FINALITY_MIN_BOND_ATOMIC",
        DEFAULT_FINALITY_MIN_BOND_ATOMIC,
    )
    .max(1)
}

pub fn finality_bond_age() -> u64 {
    prod_or_test_u64("MESH_FINALITY_BOND_AGE", DEFAULT_FINALITY_BOND_AGE)
}

pub fn finality_active_at(height: u64) -> bool {
    height >= finality_activation_height()
}

/// Production binaries ignore override envs (same pattern as `MESH_LIGHT_POW`).
fn prod_or_test_u64(key: &str, default: u64) -> u64 {
    if cfg!(test) {
        parse_u64_env(key, default)
    } else {
        default
    }
}

fn parse_u64_env(key: &str, default: u64) -> u64 {
    match std::env::var(key) {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                default
            } else {
                t.parse().unwrap_or(default)
            }
        }
        Err(_) => default,
    }
}

pub fn attestation_message(genesis: &Hash, height: u64, block_hash: &Hash) -> Vec<u8> {
    let mut m = Vec::with_capacity(13 + 32 + 8 + 32);
    m.extend_from_slice(b"mesh-final:v2");
    m.extend_from_slice(genesis.as_bytes());
    m.extend_from_slice(&height.to_le_bytes());
    m.extend_from_slice(block_hash.as_bytes());
    m
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FinalityAttestation {
    pub genesis: Hash,
    pub height: u64,
    pub block_hash: Hash,
    pub pubkey_hex: String,
    pub signature_hex: String,
}

impl FinalityAttestation {
    pub fn sign(kp: &Keypair, genesis: Hash, height: u64, block_hash: Hash) -> Self {
        let sig = kp.sign(&attestation_message(&genesis, height, &block_hash));
        Self {
            genesis,
            height,
            block_hash,
            pubkey_hex: hex::encode(kp.public_key_bytes()),
            signature_hex: hex::encode(sig),
        }
    }

    pub fn operator(&self) -> Option<Address> {
        let pk = decode32(&self.pubkey_hex)?;
        Some(Address::from_pubkey_bytes(&pk))
    }

    pub fn verify(&self) -> bool {
        let Some(pk) = decode32(&self.pubkey_hex) else {
            return false;
        };
        let Some(sig) = decode64(&self.signature_hex) else {
            return false;
        };
        mesh_crypto::verify(
            &pk,
            &attestation_message(&self.genesis, self.height, &self.block_hash),
            &sig,
        )
        .is_ok()
    }
}

fn decode32(hex_s: &str) -> Option<[u8; 32]> {
    let b = hex::decode(hex_s.trim()).ok()?;
    if b.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&b);
    Some(out)
}

fn decode64(hex_s: &str) -> Option<[u8; 64]> {
    let b = hex::decode(hex_s.trim()).ok()?;
    if b.len() != 64 {
        return None;
    }
    let mut out = [0u8; 64];
    out.copy_from_slice(&b);
    Some(out)
}

#[derive(Clone, Debug, Default)]
pub struct FinalityIngest {
    pub new_vote: bool,
    pub advanced: bool,
    pub slashed: Option<Address>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct FinalitySidecar {
    #[serde(default)]
    genesis: String,
    #[serde(default)]
    finalized_height: u64,
    #[serde(default)]
    finalized_hash: String,
    /// Old sidecar field.
    #[serde(default)]
    height: u64,
    #[serde(default)]
    hash: String,
    #[serde(default)]
    pending: Vec<FinalityAttestation>,
}

#[derive(Clone, Debug, Default)]
pub struct FinalityState {
    pub finalized_height: u64,
    pub finalized_hash: Hash,
    /// (height, hash) → operator hex → attestation
    pub(crate) pending: HashMap<(u64, Hash), HashMap<String, FinalityAttestation>>,
}

impl FinalityState {
    pub fn load(store_path: &Path, genesis: Hash) -> Self {
        let path = sidecar_path(store_path);
        let Ok(bytes) = fs::read(&path) else {
            return Self::default();
        };
        let Ok(ck) = serde_json::from_slice::<FinalitySidecar>(&bytes) else {
            return Self::default();
        };
        if !ck.genesis.is_empty() {
            if Hash::from_hex(&ck.genesis).ok() != Some(genesis) {
                tracing::warn!("finality sidecar genesis mismatch — ignoring");
                return Self::default();
            }
        }
        let fin_h = if ck.finalized_height > 0 {
            ck.finalized_height
        } else {
            ck.height
        };
        let fin_hex = if !ck.finalized_hash.is_empty() {
            ck.finalized_hash
        } else {
            ck.hash
        };
        let hash = Hash::from_hex(&fin_hex).unwrap_or_else(|_| Hash::zero());
        let mut pending = HashMap::new();
        for att in ck.pending {
            if !att.verify() || att.genesis != genesis {
                continue;
            }
            let Some(op) = att.operator() else {
                continue;
            };
            pending
                .entry((att.height, att.block_hash))
                .or_insert_with(HashMap::new)
                .insert(op.to_hex(), att);
        }
        Self {
            finalized_height: fin_h,
            finalized_hash: hash,
            pending,
        }
    }

    pub fn persist(&self, store_path: &Path, genesis: Hash) -> Result<(), ChainError> {
        if self.finalized_height == 0
            && self.finalized_hash == Hash::zero()
            && self.pending.is_empty()
        {
            return Ok(());
        }
        let path = sidecar_path(store_path);
        let mut pending = Vec::new();
        for m in self.pending.values() {
            pending.extend(m.values().cloned());
        }
        pending.sort_by(|a, b| a.height.cmp(&b.height).then(a.pubkey_hex.cmp(&b.pubkey_hex)));
        let ck = FinalitySidecar {
            genesis: genesis.to_hex(),
            finalized_height: self.finalized_height,
            finalized_hash: self.finalized_hash.to_hex(),
            height: self.finalized_height,
            hash: self.finalized_hash.to_hex(),
            pending,
        };
        let tmp = path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec(&ck).map_err(|e| ChainError::Store(e.to_string()))?,
        )?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn would_pop_finalized(&self, ancestor_height: u64) -> bool {
        self.finalized_height > 0 && self.finalized_height > ancestor_height
    }

    pub(crate) fn prune(&mut self, tip: u64) {
        let floor = self
            .finalized_height
            .max(tip.saturating_sub(finality_window().saturating_add(CATCHUP_HEIGHTS)));
        self.pending.retain(|(h, _), _| *h > floor);
        if self.pending.len() > MAX_PENDING_HEIGHTS {
            let mut keys: Vec<(u64, Hash)> = self.pending.keys().copied().collect();
            keys.sort_by_key(|(h, _)| *h);
            let drop_n = keys.len() - MAX_PENDING_HEIGHTS;
            for k in keys.into_iter().take(drop_n) {
                self.pending.remove(&k);
            }
        }
    }
}

fn sidecar_path(store_path: &Path) -> PathBuf {
    store_path.with_extension("finality.json")
}

pub fn is_finality_attestor(rec: &NodeBondRec, tip: u64) -> bool {
    if rec.slashed || rec.unlock_after_height != 0 {
        return false;
    }
    if rec.locked_atomic() < min_finality_bond_atomic() {
        return false;
    }
    if tip.saturating_sub(rec.bonded_at_height) < finality_bond_age() {
        return false;
    }
    true
}

pub fn live_finality_weight(bonds: &HashMap<String, NodeBondRec>, tip: u64) -> u64 {
    bonds
        .values()
        .filter(|b| is_finality_attestor(b, tip))
        .map(|b| b.locked_atomic())
        .sum()
}

pub fn live_finality_count(bonds: &HashMap<String, NodeBondRec>, tip: u64) -> u64 {
    bonds.values().filter(|b| is_finality_attestor(b, tip)).count() as u64
}

/// Vote is deep enough (at or behind the window) and not ancient.
pub fn attest_height_ok(height: u64, tip: u64, finalized: u64) -> bool {
    if height == 0 {
        return false;
    }
    if height <= finalized {
        return false;
    }
    let window = finality_window();
    if tip < window || height > tip.saturating_sub(window) {
        return false;
    }
    let oldest = tip.saturating_sub(window.saturating_add(CATCHUP_HEIGHTS));
    height > oldest
}

pub(crate) fn max_votes_per_height() -> usize {
    MAX_VOTES_PER_HEIGHT
}
