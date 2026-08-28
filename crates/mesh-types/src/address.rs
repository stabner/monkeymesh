use serde::{Deserialize, Serialize};
use std::fmt;

use crate::Hash;

const ADDRESS_VERSION: u8 = 0x01;
const ADDRESS_PAYLOAD_LEN: usize = 20;

/// Versioned address derived from a public key hash (20 bytes + version).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    version: u8,
    payload: [u8; ADDRESS_PAYLOAD_LEN],
}

impl Address {
    pub fn from_pubkey_bytes(pubkey: &[u8]) -> Self {
        let hash = Hash::digest(pubkey);
        let mut payload = [0u8; ADDRESS_PAYLOAD_LEN];
        payload.copy_from_slice(&hash.as_bytes()[..ADDRESS_PAYLOAD_LEN]);
        Self {
            version: ADDRESS_VERSION,
            payload,
        }
    }

    pub fn to_hex(&self) -> String {
        let mut bytes = Vec::with_capacity(1 + ADDRESS_PAYLOAD_LEN);
        bytes.push(self.version);
        bytes.extend_from_slice(&self.payload);
        format!("mesh{}", hex::encode(bytes))
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let hex_part = s.strip_prefix("mesh")?;
        let bytes = hex::decode(hex_part).ok()?;
        if bytes.len() != 1 + ADDRESS_PAYLOAD_LEN {
            return None;
        }
        let version = bytes[0];
        let mut payload = [0u8; ADDRESS_PAYLOAD_LEN];
        payload.copy_from_slice(&bytes[1..]);
        Some(Self { version, payload })
    }

    pub fn as_bytes(&self) -> [u8; 1 + ADDRESS_PAYLOAD_LEN] {
        let mut out = [0u8; 1 + ADDRESS_PAYLOAD_LEN];
        out[0] = self.version;
        out[1..].copy_from_slice(&self.payload);
        out
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({})", self.to_hex())
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Default for Address {
    fn default() -> Self {
        Self {
            version: ADDRESS_VERSION,
            payload: [0u8; ADDRESS_PAYLOAD_LEN],
        }
    }
}
