use serde::{Deserialize, Serialize};
use std::fmt;

pub const HASH_LEN: usize = 32;

/// 32-byte digest used for block, tx, and PoW hashes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Hash(pub [u8; HASH_LEN]);

impl Hash {
    pub fn zero() -> Self {
        Self([0u8; HASH_LEN])
    }

    pub fn from_bytes(bytes: [u8; HASH_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; HASH_LEN] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        if bytes.len() != HASH_LEN {
            return Err(hex::FromHexError::InvalidStringLength);
        }
        let mut arr = [0u8; HASH_LEN];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    /// Digest arbitrary bytes with Blake3.
    pub fn digest(data: &[u8]) -> Self {
        let out = blake3::hash(data);
        Self(*out.as_bytes())
    }

    /// Leading zero bits — used for PoW difficulty checks.
    pub fn leading_zero_bits(&self) -> u32 {
        let mut bits = 0u32;
        for byte in &self.0 {
            if *byte == 0 {
                bits += 8;
            } else {
                bits += byte.leading_zeros();
                break;
            }
        }
        bits
    }

    pub fn meets_difficulty(&self, leading_zeros: u32) -> bool {
        self.leading_zero_bits() >= leading_zeros
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.to_hex())
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}
