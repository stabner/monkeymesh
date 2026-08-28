use serde::{Deserialize, Serialize};

use crate::{Address, Amount, Hash};

pub type TxId = Hash;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TxInput {
    /// Previous transaction id (zero for coinbase).
    pub prev_txid: TxId,
    /// Output index in previous tx.
    pub vout: u32,
    /// Spender public key (32 bytes). Empty for coinbase.
    pub pubkey: Vec<u8>,
    /// Ed25519 signature over [`Transaction::sighash`] (64 bytes). Empty for coinbase.
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TxOutput {
    pub address: Address,
    pub amount: Amount,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Transaction {
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,
    /// Optional memo / coinbase tag.
    pub memo: String,
}

impl Transaction {
    /// Legacy single-output coinbase (CPU-only era). Prefer [`Self::market_coinbase`].
    pub fn coinbase(reward: Amount, miner: Address, height: u64) -> Self {
        Self {
            inputs: vec![TxInput {
                prev_txid: Hash::zero(),
                vout: height as u32,
                pubkey: Vec::new(),
                signature: Vec::new(),
            }],
            outputs: vec![TxOutput {
                address: miner,
                amount: reward,
            }],
            memo: format!("coinbase:{height}"),
        }
    }

    /// PoMC multi-market coinbase: CPU + GPU + Node outputs (Build/14).
    ///
    /// Layout: 1 CPU output, then `gpu.len()` GPU outputs, then `node.len()` Node outputs.
    /// Memo: `pomc:v1:{height}:{n_gpu}:{n_node}|mat:20` with optional `|exam:{n}`
    /// and other `|…` suffixes. Display labels: [`crate::coinbase_payout_label`].
    pub fn market_coinbase(
        height: u64,
        cpu: (Address, Amount),
        gpu: &[(Address, Amount)],
        node: &[(Address, Amount)],
    ) -> Self {
        let mut outputs = Vec::with_capacity(1 + gpu.len() + node.len());
        outputs.push(TxOutput {
            address: cpu.0,
            amount: cpu.1,
        });
        for (addr, amt) in gpu {
            outputs.push(TxOutput {
                address: *addr,
                amount: *amt,
            });
        }
        for (addr, amt) in node {
            outputs.push(TxOutput {
                address: *addr,
                amount: *amt,
            });
        }
        Self {
            inputs: vec![TxInput {
                prev_txid: Hash::zero(),
                vout: height as u32,
                pubkey: Vec::new(),
                signature: Vec::new(),
            }],
            outputs,
            memo: format!(
                "pomc:v1:{height}:{}:{}|mat:{}",
                gpu.len(),
                node.len(),
                crate::COINBASE_MATURITY
            ),
        }
    }

    /// Parse `pomc:v1:{height}:{n_gpu}:{n_node}` memo (optional `|…` suffix allowed).
    pub fn parse_pomc_memo(&self) -> Option<(u64, usize, usize)> {
        crate::parse_pomc_layout(&self.memo).map(|l| (l.height, l.n_gpu, l.n_node))
    }

    /// Exam-helper count from `|exam:{n}` (helper-floor coinbases).
    pub fn parse_pomc_exam_count(&self) -> Option<usize> {
        crate::parse_pomc_layout(&self.memo).and_then(|l| l.n_exam)
    }

    /// Memo for on-chain slash settle: `slash:v1:{mesh_address}`.
    pub fn slash_settle_memo(address: &Address) -> String {
        format!("slash:v1:{}", address.to_hex())
    }

    /// Parse slash-settle address from memo (optional `|…` suffix allowed).
    pub fn parse_slash_settle_memo(&self) -> Option<String> {
        let main = self.memo.split('|').next()?;
        main.strip_prefix("slash:v1:")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn is_slash_settle(&self) -> bool {
        self.parse_slash_settle_memo().is_some() && !self.is_coinbase()
    }

    pub fn is_coinbase(&self) -> bool {
        // Structural only — memo is never consensus-critical.
        self.inputs.len() == 1
            && self.inputs[0].prev_txid == Hash::zero()
            && self.inputs[0].pubkey.is_empty()
            && self.inputs[0].signature.is_empty()
    }

    /// Digest used for txid and for signing (signatures excluded).
    pub fn sighash(&self) -> Hash {
        Hash::digest(&encode_for_sighash(self))
    }

    pub fn txid(&self) -> TxId {
        self.sighash()
    }

    /// Sum outputs; fails closed on overflow (prevents inflation tricks).
    pub fn checked_total_output(&self) -> Result<Amount, crate::AmountError> {
        let mut acc = Amount::ZERO;
        for o in &self.outputs {
            acc = acc.checked_add(o.amount)?;
        }
        Ok(acc)
    }

    pub fn total_output(&self) -> Amount {
        self.checked_total_output().unwrap_or(Amount::ZERO)
    }

    pub fn clear_witnesses(&mut self) {
        for inp in &mut self.inputs {
            inp.pubkey.clear();
            inp.signature.clear();
        }
    }
}

fn encode_for_sighash(tx: &Transaction) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(tx.inputs.len() as u32).to_le_bytes());
    for inp in &tx.inputs {
        buf.extend_from_slice(inp.prev_txid.as_bytes());
        buf.extend_from_slice(&inp.vout.to_le_bytes());
        // pubkey + signature intentionally omitted from sighash/txid
    }
    buf.extend_from_slice(&(tx.outputs.len() as u32).to_le_bytes());
    for out in &tx.outputs {
        buf.extend_from_slice(&out.address.as_bytes());
        buf.extend_from_slice(&out.amount.atomic().to_le_bytes());
    }
    buf.extend_from_slice(&(tx.memo.len() as u32).to_le_bytes());
    buf.extend_from_slice(tx.memo.as_bytes());
    buf
}
