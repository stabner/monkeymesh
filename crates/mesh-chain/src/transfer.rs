//! Build and sign payment transactions.

use mesh_crypto::Keypair;
use mesh_types::{Address, Amount, OutPoint, Transaction, TxInput, TxOutput, Utxo};

use crate::{deferred_slash_vault, ChainError};

/// Select UTXOs (largest-first), build a signed payment with change.
pub fn build_signed_payment(
    keypair: &Keypair,
    utxos: &[(OutPoint, Utxo)],
    to: Address,
    amount: Amount,
    memo: impl Into<String>,
) -> Result<Transaction, ChainError> {
    let from = keypair.address();
    if amount == Amount::ZERO {
        return Err(ChainError::InvalidTx("amount must be > 0".into()));
    }

    let mut owned: Vec<(OutPoint, Utxo)> = utxos
        .iter()
        .filter(|(_, u)| u.address == from)
        .cloned()
        .collect();
    owned.sort_by(|a, b| b.1.amount.atomic().cmp(&a.1.amount.atomic()));

    let mut selected = Vec::new();
    let mut total_in = Amount::ZERO;
    for (op, u) in owned {
        total_in = total_in
            .checked_add(u.amount)
            .map_err(|_| ChainError::InvalidTx("input overflow".into()))?;
        selected.push((op, u));
        if total_in >= amount {
            break;
        }
    }

    if total_in < amount {
        return Err(ChainError::InsufficientFunds {
            have: total_in,
            need: amount,
        });
    }

    let change = total_in
        .checked_sub(amount)
        .map_err(|_| ChainError::InvalidTx("change underflow".into()))?;

    let mut outputs = vec![TxOutput { address: to, amount }];
    if change > Amount::ZERO {
        outputs.push(TxOutput {
            address: from,
            amount: change,
        });
    }

    let mut tx = Transaction {
        inputs: selected
            .iter()
            .map(|(op, _)| TxInput {
                prev_txid: op.txid,
                vout: op.vout,
                pubkey: Vec::new(),
                signature: Vec::new(),
            })
            .collect(),
        outputs,
        memo: memo.into(),
    };

    let digest = tx.sighash();
    let pubkey = keypair.public_key_bytes().to_vec();
    let sig = keypair.sign(digest.as_bytes()).to_vec();
    for inp in &mut tx.inputs {
        inp.pubkey = pubkey.clone();
        inp.signature = sig.clone();
    }

    Ok(tx)
}

/// Spend locked bond outs → [`deferred_slash_vault`] (memo `slash:v1:{addr}`).
pub fn build_slash_settle(
    keypair: &Keypair,
    locked: &[(OutPoint, Utxo)],
) -> Result<Transaction, ChainError> {
    let from = keypair.address();
    if locked.is_empty() {
        return Err(ChainError::InvalidTx("slash settle needs locked UTXOs".into()));
    }
    let mut total = Amount::ZERO;
    let mut inputs = Vec::with_capacity(locked.len());
    for (op, u) in locked {
        if u.address != from {
            return Err(ChainError::InvalidTx(
                "slash settle input not owned by signer".into(),
            ));
        }
        total = total
            .checked_add(u.amount)
            .map_err(|_| ChainError::InvalidTx("slash settle overflow".into()))?;
        inputs.push(TxInput {
            prev_txid: op.txid,
            vout: op.vout,
            pubkey: Vec::new(),
            signature: Vec::new(),
        });
    }
    if total == Amount::ZERO {
        return Err(ChainError::InvalidTx("slash settle amount zero".into()));
    }
    let mut tx = Transaction {
        inputs,
        outputs: vec![TxOutput {
            address: deferred_slash_vault(),
            amount: total,
        }],
        memo: Transaction::slash_settle_memo(&from),
    };
    let digest = tx.sighash();
    let pubkey = keypair.public_key_bytes().to_vec();
    let sig = keypair.sign(digest.as_bytes()).to_vec();
    for inp in &mut tx.inputs {
        inp.pubkey = pubkey.clone();
        inp.signature = sig.clone();
    }
    Ok(tx)
}
