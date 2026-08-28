use serde::{Deserialize, Serialize};

use crate::{Hash, Transaction};

pub type BlockId = Hash;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockHeader {
    pub version: u32,
    pub prev_hash: Hash,
    pub merkle_root: Hash,
    pub timestamp: u64,
    pub height: u64,
    /// Required leading zero bits in the PoW hash.
    pub difficulty: u32,
    pub nonce: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Block {
    pub header: BlockHeader,
    pub txs: Vec<Transaction>,
}

impl BlockHeader {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + 32 + 32 + 8 + 8 + 4 + 8);
        buf.extend_from_slice(&self.version.to_le_bytes());
        buf.extend_from_slice(self.prev_hash.as_bytes());
        buf.extend_from_slice(self.merkle_root.as_bytes());
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf.extend_from_slice(&self.height.to_le_bytes());
        buf.extend_from_slice(&self.difficulty.to_le_bytes());
        buf.extend_from_slice(&self.nonce.to_le_bytes());
        buf
    }

    pub fn pre_pow_commitment(&self) -> Hash {
        // Hash everything except nonce so miner can iterate nonce cheaply at PoW layer.
        let mut buf = Vec::with_capacity(4 + 32 + 32 + 8 + 8 + 4);
        buf.extend_from_slice(&self.version.to_le_bytes());
        buf.extend_from_slice(self.prev_hash.as_bytes());
        buf.extend_from_slice(self.merkle_root.as_bytes());
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf.extend_from_slice(&self.height.to_le_bytes());
        buf.extend_from_slice(&self.difficulty.to_le_bytes());
        Hash::digest(&buf)
    }
}

impl Block {
    pub fn id(&self) -> BlockId {
        Hash::digest(&self.header.encode())
    }

    pub fn merkle_root(txs: &[Transaction]) -> Hash {
        if txs.is_empty() {
            return Hash::zero();
        }
        let mut layer: Vec<Hash> = txs.iter().map(|t| t.txid()).collect();
        while layer.len() > 1 {
            let mut next = Vec::with_capacity(layer.len().div_ceil(2));
            for chunk in layer.chunks(2) {
                let left = chunk[0];
                let right = if chunk.len() == 2 { chunk[1] } else { chunk[0] };
                let mut data = [0u8; 64];
                data[..32].copy_from_slice(left.as_bytes());
                data[32..].copy_from_slice(right.as_bytes());
                next.push(Hash::digest(&data));
            }
            layer = next;
        }
        layer[0]
    }
}
