use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use mesh_crypto::verify;
use mesh_types::{Address, Amount, Block, Hash, OutPoint, Transaction, Utxo};
use crate::store::apply_tx_utxos;
use crate::{
    block_reward, cpu_market_reward, gpu_market_reward, node_market_reward, pow_hash_header,
    ChainError,
};

/// Max non-coinbase transactions allowed in a single block.
pub const MAX_BLOCK_TXS: usize = 2_000;
/// Max UTF-8 bytes for a transaction memo.
pub const MAX_MEMO_BYTES: usize = 1_024;
/// Max inputs / outputs per non-coinbase tx.
pub const MAX_TX_IO: usize = 1_024;
/// Reject timestamps more than this far in the future.
pub const MAX_FUTURE_SKEW_SECS: u64 = 2 * 60 * 60;

pub fn validate_block(
    block: &Block,
    prev: Option<&Block>,
    light_pow: bool,
    utxos: &HashMap<OutPoint, Utxo>,
    expected_difficulty: Option<u32>,
    coinbase_heights: &HashMap<OutPoint, u64>,
) -> Result<(), ChainError> {
    validate_block_ex(
        block,
        prev,
        light_pow,
        utxos,
        expected_difficulty,
        coinbase_heights,
        false,
    )
}

/// Same as [`validate_block`] but can skip Fusion PoW (official HTTP replica IBD).
pub fn validate_block_ex(
    block: &Block,
    prev: Option<&Block>,
    light_pow: bool,
    utxos: &HashMap<OutPoint, Utxo>,
    expected_difficulty: Option<u32>,
    coinbase_heights: &HashMap<OutPoint, u64>,
    skip_pow: bool,
) -> Result<(), ChainError> {
    validate_block_header_ex(block, prev, light_pow, expected_difficulty, skip_pow)?;
    validate_block_txs(block, utxos, coinbase_heights)?;
    Ok(())
}

pub fn validate_block_header(
    block: &Block,
    prev: Option<&Block>,
    light_pow: bool,
    expected_difficulty: Option<u32>,
) -> Result<(), ChainError> {
    validate_block_header_ex(block, prev, light_pow, expected_difficulty, false)
}

fn validate_block_header_ex(
    block: &Block,
    prev: Option<&Block>,
    light_pow: bool,
    expected_difficulty: Option<u32>,
    skip_pow: bool,
) -> Result<(), ChainError> {
    if block.txs.is_empty() {
        return Err(ChainError::InvalidBlock("empty block".into()));
    }
    if block.txs.len() > MAX_BLOCK_TXS + 1 {
        return Err(ChainError::InvalidBlock(format!(
            "too many txs (max {})",
            MAX_BLOCK_TXS + 1
        )));
    }

    let cb = &block.txs[0];
    if !cb.is_coinbase() {
        return Err(ChainError::InvalidBlock("first tx must be coinbase".into()));
    }
    if cb.inputs[0].vout != block.header.height as u32 {
        return Err(ChainError::InvalidBlock(
            "coinbase height mismatch (vout)".into(),
        ));
    }
    if cb.outputs.is_empty() || cb.outputs.len() > 16 {
        return Err(ChainError::InvalidBlock("bad coinbase outputs".into()));
    }

    let expected_merkle = Block::merkle_root(&block.txs);
    if block.header.merkle_root != expected_merkle {
        return Err(ChainError::InvalidBlock("bad merkle root".into()));
    }

    if block.header.difficulty < 1 {
        return Err(ChainError::InvalidBlock("difficulty must be >= 1".into()));
    }

    let now = now_secs();
    match prev {
        None => {
            if block.header.height != 0 {
                return Err(ChainError::InvalidBlock("genesis height must be 0".into()));
            }
            if block.header.prev_hash != Hash::zero() {
                return Err(ChainError::InvalidBlock("genesis prev must be zero".into()));
            }
        }
        Some(p) => {
            if block.header.height != p.header.height + 1 {
                return Err(ChainError::InvalidBlock("height mismatch".into()));
            }
            if block.header.prev_hash != p.id() {
                return Err(ChainError::InvalidBlock("prev hash mismatch".into()));
            }
            if block.header.timestamp <= p.header.timestamp {
                return Err(ChainError::InvalidBlock(
                    "timestamp must be strictly greater than previous".into(),
                ));
            }
        }
    }

    if block.header.timestamp > now.saturating_add(MAX_FUTURE_SKEW_SECS) {
        return Err(ChainError::InvalidBlock(
            "timestamp too far in the future".into(),
        ));
    }

    if let Some(exp) = expected_difficulty {
        if block.header.difficulty != exp {
            return Err(ChainError::InvalidBlock(format!(
                "bad difficulty: got {}, expected {exp}",
                block.header.difficulty
            )));
        }
    }

    let expected = block_reward(block.header.height);
    let coinbase_out = cb
        .checked_total_output()
        .map_err(|_| ChainError::InvalidBlock("coinbase output overflow".into()))?;
    // Always enforce full subsidy (CPU+GPU+Node).
    if coinbase_out != expected {
        return Err(ChainError::InvalidBlock(format!(
            "bad coinbase reward: got {coinbase_out}, expected {expected}"
        )));
    }
    validate_pomc_coinbase(cb, block.header.height)?;

    if skip_pow {
        let _ = light_pow;
        return Ok(());
    }

    let commitment = block.header.pre_pow_commitment();
    let pow = pow_hash_header(
        &commitment,
        block.header.nonce,
        light_pow,
        block.header.height,
        &block.header.prev_hash,
    );
    if !pow.meets_difficulty(block.header.difficulty) {
        return Err(ChainError::InvalidBlock(format!(
            "insufficient PoW (need {} leading zeros, got {})",
            block.header.difficulty,
            pow.leading_zero_bits()
        )));
    }

    Ok(())
}

fn validate_pomc_coinbase(cb: &Transaction, height: u64) -> Result<(), ChainError> {
    let Some((memo_h, n_gpu, n_node)) = cb.parse_pomc_memo() else {
        return Err(ChainError::InvalidBlock(
            "coinbase must use pomc:v1 market memo".into(),
        ));
    };
    if memo_h != height {
        return Err(ChainError::InvalidBlock(
            "pomc memo height mismatch".into(),
        ));
    }
    let tagged = mesh_types::parse_pomc_layout(&cb.memo).and_then(|l| l.maturity);
    match tagged {
        Some(mat) if mat == mesh_types::COINBASE_MATURITY => {}
        Some(mat) => {
            return Err(ChainError::InvalidBlock(format!(
                "coinbase |mat:{mat} must be {}",
                mesh_types::COINBASE_MATURITY
            )));
        }
        None if height >= mesh_types::MATURITY_TAG_REQUIRED_HEIGHT => {
            return Err(ChainError::InvalidBlock(format!(
                "coinbase missing |mat:{}",
                mesh_types::COINBASE_MATURITY
            )));
        }
        None => {}
    }
    let expected_len = 1 + n_gpu + n_node;
    if cb.outputs.len() != expected_len {
        return Err(ChainError::InvalidBlock(format!(
            "pomc coinbase output count: got {}, expected {expected_len}",
            cb.outputs.len()
        )));
    }
    if n_node == 0 {
        return Err(ChainError::InvalidBlock(
            "pomc coinbase requires at least one Node output".into(),
        ));
    }
    if mesh_types::finder_unify_active(height) {
        if n_gpu != 0 {
            return Err(ChainError::InvalidBlock(
                "finder-unify coinbase has no GPU-lane outputs".into(),
            ));
        }
    } else if n_gpu == 0 {
        return Err(ChainError::InvalidBlock(
            "pomc coinbase requires at least one GPU and one Node output".into(),
        ));
    }
    let gpu_sum = if n_gpu == 0 {
        mesh_types::Amount::ZERO
    } else {
        cb.outputs[1..1 + n_gpu]
            .iter()
            .try_fold(mesh_types::Amount::ZERO, |a, o| a.checked_add(o.amount))
            .map_err(|_| ChainError::InvalidBlock("gpu coinbase overflow".into()))?
    };
    let node_sum = cb.outputs[1 + n_gpu..]
        .iter()
        .try_fold(mesh_types::Amount::ZERO, |a, o| a.checked_add(o.amount))
        .map_err(|_| ChainError::InvalidBlock("node coinbase overflow".into()))?;
    if node_sum != node_market_reward(height) {
        return Err(ChainError::InvalidBlock("bad Node market coinbase amount".into()));
    }
    if mesh_types::finder_unify_active(height) {
        if cb.outputs[0].amount != cpu_market_reward(height) {
            return Err(ChainError::InvalidBlock("bad finder pot coinbase amount".into()));
        }
        if gpu_sum != mesh_types::Amount::ZERO {
            return Err(ChainError::InvalidBlock("finder-unify GPU lane must be empty".into()));
        }
    } else if mesh_types::fair_lane_split_active(height) {
        if cb.outputs[0].amount != cpu_market_reward(height) {
            return Err(ChainError::InvalidBlock("bad CPU lane coinbase amount".into()));
        }
        if gpu_sum != gpu_market_reward(height) {
            return Err(ChainError::InvalidBlock("bad GPU lane coinbase amount".into()));
        }
    } else if mesh_types::shared_contrib_active(height) {
        let contrib = cb.outputs[0]
            .amount
            .checked_add(gpu_sum)
            .map_err(|_| ChainError::InvalidBlock("contrib coinbase overflow".into()))?;
        let expect = block_reward(height).split_bps(mesh_types::CONTRIBUTOR_MARKET_BPS);
        if contrib != expect {
            return Err(ChainError::InvalidBlock("bad contributor pot coinbase amount".into()));
        }
    } else {
        if cb.outputs[0].amount != cpu_market_reward(height) {
            return Err(ChainError::InvalidBlock("bad CPU market coinbase amount".into()));
        }
        if gpu_sum != gpu_market_reward(height) {
            return Err(ChainError::InvalidBlock("bad GPU market coinbase amount".into()));
        }
    }
    Ok(())
}

pub fn validate_block_txs(
    block: &Block,
    base_utxos: &HashMap<OutPoint, Utxo>,
    prior_coinbase_heights: &HashMap<OutPoint, u64>,
) -> Result<(), ChainError> {
    let mut utxos = base_utxos.clone();
    let mut coinbase_heights = prior_coinbase_heights.clone();

    for (i, tx) in block.txs.iter().enumerate() {
        if i == 0 {
            apply_tx_utxos(&mut utxos, tx, false)?;
            let txid = tx.txid();
            for (vout, _) in tx.outputs.iter().enumerate() {
                coinbase_heights.insert(OutPoint::new(txid, vout as u32), block.header.height);
            }
            continue;
        }
        validate_tx_at_height(tx, &utxos, &coinbase_heights, block.header.height)?;
        apply_tx_utxos(&mut utxos, tx, false)?;
    }
    Ok(())
}

pub fn validate_tx(tx: &Transaction, utxos: &HashMap<OutPoint, Utxo>) -> Result<(), ChainError> {
    validate_tx_at_height(tx, utxos, &HashMap::new(), u64::MAX)
}

pub(crate) fn validate_tx_at_height(
    tx: &Transaction,
    utxos: &HashMap<OutPoint, Utxo>,
    coinbase_heights: &HashMap<OutPoint, u64>,
    spend_height: u64,
) -> Result<(), ChainError> {
    if tx.is_coinbase() {
        return Err(ChainError::InvalidTx("coinbase not allowed here".into()));
    }
    if tx.inputs.is_empty() {
        return Err(ChainError::InvalidTx("no inputs".into()));
    }
    if tx.outputs.is_empty() {
        return Err(ChainError::InvalidTx("no outputs".into()));
    }
    if tx.inputs.len() > MAX_TX_IO || tx.outputs.len() > MAX_TX_IO {
        return Err(ChainError::InvalidTx("too many inputs/outputs".into()));
    }
    if tx.memo.len() > MAX_MEMO_BYTES {
        return Err(ChainError::InvalidTx("memo too large".into()));
    }
    for out in &tx.outputs {
        if out.amount == Amount::ZERO {
            return Err(ChainError::InvalidTx("zero-value output".into()));
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut total_in = Amount::ZERO;
    let sighash = tx.sighash();

    for inp in &tx.inputs {
        let op = OutPoint::new(inp.prev_txid, inp.vout);
        if !seen.insert(op) {
            return Err(ChainError::InvalidTx(format!("duplicate input {op}")));
        }
        let utxo = utxos
            .get(&op)
            .ok_or_else(|| ChainError::InvalidTx(format!("missing utxo {op}")))?;

        if let Some(&cb_h) = coinbase_heights.get(&op) {
            if spend_height < cb_h.saturating_add(mesh_types::COINBASE_MATURITY) {
                return Err(ChainError::InvalidTx(format!(
                    "coinbase immature (need {} confirmations)",
                    mesh_types::COINBASE_MATURITY
                )));
            }
        }

        if inp.pubkey.len() != 32 {
            return Err(ChainError::InvalidTx("pubkey must be 32 bytes".into()));
        }
        if inp.signature.len() != 64 {
            return Err(ChainError::InvalidTx("signature must be 64 bytes".into()));
        }

        let mut pk = [0u8; 32];
        pk.copy_from_slice(&inp.pubkey);
        let addr = Address::from_pubkey_bytes(&pk);
        if addr != utxo.address {
            return Err(ChainError::InvalidTx(format!(
                "pubkey does not own utxo {op}"
            )));
        }

        let mut sig = [0u8; 64];
        sig.copy_from_slice(&inp.signature);
        verify(&pk, sighash.as_bytes(), &sig)
            .map_err(|e| ChainError::InvalidTx(format!("bad signature on {op}: {e}")))?;

        total_in = total_in
            .checked_add(utxo.amount)
            .map_err(|_| ChainError::InvalidTx("input amount overflow".into()))?;
    }

    let total_out = tx
        .checked_total_output()
        .map_err(|_| ChainError::InvalidTx("output amount overflow".into()))?;
    if total_out > total_in {
        return Err(ChainError::InvalidTx(format!(
            "outputs {total_out} exceed inputs {total_in}"
        )));
    }

    Ok(())
}

/// Validate a mempool candidate against chain UTXOs + already-queued mempool spends.
pub fn validate_mempool_tx(
    tx: &Transaction,
    chain_utxos: &HashMap<OutPoint, Utxo>,
    mempool: &[Transaction],
) -> Result<(), ChainError> {
    let mut utxos = chain_utxos.clone();
    for pending in mempool {
        for inp in &pending.inputs {
            utxos.remove(&OutPoint::new(inp.prev_txid, inp.vout));
        }
    }
    // Mempool path cannot know coinbase heights cheaply yet — maturity enforced at block time.
    validate_tx(tx, &utxos)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
