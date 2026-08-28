use serde::{Deserialize, Serialize};
use std::fmt;

use crate::{Address, Amount, TxId};

/// Reference to a specific transaction output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OutPoint {
    pub txid: TxId,
    pub vout: u32,
}

impl OutPoint {
    pub fn new(txid: TxId, vout: u32) -> Self {
        Self { txid, vout }
    }
}

impl fmt::Display for OutPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.txid, self.vout)
    }
}

/// An unspent output in the UTXO set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Utxo {
    pub address: Address,
    pub amount: Amount,
}
