//! SLIP-0010 hierarchical derivation for Ed25519.
//!
//! Spec: https://github.com/satoshilabs/slips/blob/master/slip-0010.md

use hmac::{Hmac, Mac};
use sha2::Sha512;

use crate::CryptoError;

type HmacSha512 = Hmac<Sha512>;

/// MonkeyMesh BIP44 coin type (provisional private-use id until registered).
pub const MESH_COIN_TYPE: u32 = 999_778;

/// Default account path for the first receiving key.
/// `m/44'/999778'/0'/0'/0'`
pub const MESH_ACCOUNT_PATH: &str = "m/44'/999778'/0'/0'/0'";

/// Hardened address path for index `n`.
pub fn account_path(index: u32) -> String {
    format!("m/44'/{MESH_COIN_TYPE}'/0'/0'/{index}'")
}

const ED25519_SEED: &[u8] = b"ed25519 seed";

#[derive(Clone)]
struct Node {
    key: [u8; 32],
    chain: [u8; 32],
}

/// Derive an Ed25519 private key along a hardened-only SLIP-0010 path.
pub fn derive_ed25519_key(seed: &[u8], path: &str) -> Result<[u8; 32], CryptoError> {
    let mut node = master_key(seed)?;
    for segment in parse_path(path)? {
        node = ckd_hardened(&node, segment)?;
    }
    Ok(node.key)
}

fn master_key(seed: &[u8]) -> Result<Node, CryptoError> {
    let mut mac =
        HmacSha512::new_from_slice(ED25519_SEED).map_err(|e| CryptoError::Derivation(e.to_string()))?;
    mac.update(seed);
    let result = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    let mut chain = [0u8; 32];
    key.copy_from_slice(&result[..32]);
    chain.copy_from_slice(&result[32..]);
    Ok(Node { key, chain })
}

fn ckd_hardened(parent: &Node, index: u32) -> Result<Node, CryptoError> {
    // Hardened only: data = 0x00 || ser256(k_par) || ser32(i)
    let mut data = [0u8; 1 + 32 + 4];
    data[0] = 0x00;
    data[1..33].copy_from_slice(&parent.key);
    data[33..37].copy_from_slice(&index.to_be_bytes());

    let mut mac = HmacSha512::new_from_slice(&parent.chain)
        .map_err(|e| CryptoError::Derivation(e.to_string()))?;
    mac.update(&data);
    let result = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    let mut chain = [0u8; 32];
    key.copy_from_slice(&result[..32]);
    chain.copy_from_slice(&result[32..]);
    Ok(Node { key, chain })
}

fn parse_path(path: &str) -> Result<Vec<u32>, CryptoError> {
    let path = path.trim();
    if path == "m" || path.is_empty() {
        return Ok(Vec::new());
    }
    let rest = path
        .strip_prefix("m/")
        .ok_or_else(|| CryptoError::Derivation("path must start with m/".into()))?;
    let mut out = Vec::new();
    for part in rest.split('/') {
        let hardened = part.ends_with('\'') || part.ends_with('h') || part.ends_with('H');
        if !hardened {
            return Err(CryptoError::Derivation(
                "Ed25519 SLIP-0010 supports hardened segments only".into(),
            ));
        }
        let num = part
            .trim_end_matches(['\'', 'h', 'H'])
            .parse::<u32>()
            .map_err(|_| CryptoError::Derivation(format!("bad path segment {part}")))?;
        out.push(num | 0x8000_0000);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slip10_vector_1() {
        // Official SLIP-0010 Ed25519 test vector 1 — chain m/0'/1'
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let key = derive_ed25519_key(&seed, "m/0'/1'").unwrap();
        assert_eq!(
            hex::encode(key),
            "b1d0bad404bf35da785a64ca1ac54b2617211d2777696fbffaf208f746ae84f2"
        );
    }
}
