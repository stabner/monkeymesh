use crate::{
    deferred_slash_vault, genesis_reward_address, BOND_UNLOCK_COOLDOWN_BLOCKS, MIN_NODE_BOND_ATOMIC,
    Chain, LockedBondUtxo, NodeBondRec,
};
use mesh_crypto::Keypair;
use mesh_types::{Amount, Hash, OutPoint, Utxo};

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mm_bond_{}_{}_{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn register_locks_utxos_and_slash_keeps_freeze() {
    let dir = tempfile_dir("lock");
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    let addr = genesis_reward_address();
    let bal_before = chain.balance(&addr).atomic();
    assert!(bal_before >= MIN_NODE_BOND_ATOMIC);

    let rec = chain
        .register_node_bond(addr, "peer-bond-1")
        .expect("bond");
    assert!(rec.locked_atomic() >= MIN_NODE_BOND_ATOMIC);
    assert!(chain.is_node_bond_eligible(&addr));
    let spendable = chain.spendable_balance(&addr).atomic();
    assert!(
        spendable < bal_before,
        "spendable {spendable} should drop after lock (bal {bal_before})"
    );
    assert_eq!(
        spendable,
        bal_before.saturating_sub(rec.locked_atomic()),
        "locked amount excluded from spendable"
    );

    let op = {
        let l = &rec.locked[0];
        mesh_types::OutPoint::new(
            mesh_types::Hash::from_hex(&l.txid_hex).unwrap(),
            l.vout,
        )
    };
    assert!(chain.store().is_outpoint_locked(&op));

    let slashed = chain.slash_node_bond(addr).expect("slash");
    assert!(slashed.slashed);
    assert!(slashed.slashed_to_vault_atomic >= MIN_NODE_BOND_ATOMIC);
    assert_eq!(
        chain.store().slashed_vault_atomic(),
        slashed.slashed_to_vault_atomic
    );
    assert!(!chain.is_node_bond_eligible(&addr));
    assert!(
        chain.store().is_outpoint_locked(&op),
        "slashed collateral must stay frozen"
    );
    // Soft vault address is deterministic; UTXOs remain frozen (no fork).
    let _ = crate::deferred_slash_vault();
}

#[test]
fn slash_settle_accepts_and_clears_locks() {
    let dir = tempfile_dir("settle");
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    let kp = Keypair::generate();
    let addr = kp.address();
    let op = OutPoint::new(Hash::digest(b"bond-utxo-1"), 0);
    let atomic = 25_000_000u64;
    chain.store_mut().test_inject_utxo(
        op,
        Utxo {
            address: addr,
            amount: Amount::from_atomic(atomic),
        },
    );
    chain.store_mut().test_insert_bond(
        &addr,
        NodeBondRec {
            peer_id: "peer-settle".into(),
            bonded_at_height: 0,
            stake_atomic: atomic,
            locked: vec![LockedBondUtxo {
                txid_hex: op.txid.to_hex(),
                vout: op.vout,
                atomic,
            }],
            unlock_after_height: 0,
            slashed: true,
            slashed_to_vault_atomic: atomic,
            slashed_at_height: 0,
        },
    );
    assert!(chain.store().is_outpoint_locked(&op));
    let txid = chain.submit_slash_settle(&kp).expect("settle");
    let settle = chain
        .mempool()
        .iter()
        .find(|t| t.txid() == txid)
        .cloned()
        .expect("settle in mempool");
    assert!(chain.is_valid_slash_settle(&settle));
    assert_eq!(settle.outputs[0].address, deferred_slash_vault());
    assert_eq!(settle.outputs[0].amount.atomic(), atomic);
    chain
        .store_mut()
        .apply_slash_settle_tx(&settle)
        .expect("meta clear");
    assert!(
        !chain.store().is_outpoint_locked(&op),
        "locks cleared after settle meta apply"
    );
    let bond = chain.node_bond(&addr).expect("bond");
    assert!(bond.slashed);
    assert!(bond.locked.is_empty());
}

#[test]
fn cold_prune_writes_checkpoint_reloadable() {
    let dir = tempfile_dir("prune");
    let path = dir.join("chain.bin");
    let genesis = {
        let mut chain = Chain::open_or_genesis(&path).expect("genesis");
        let g = chain.genesis_hash();
        let plan = chain.apply_cold_prune(128).expect("prune");
        assert!(plan.utxo_count > 0);
        assert!(path.with_extension("utxo.ckpt").exists());
        g
    };
    let chain = Chain::open(&path).expect("reopen");
    assert_eq!(chain.genesis_hash(), genesis);
    assert!(chain.balance(&genesis_reward_address()).atomic() > 0);
}

#[test]
fn unbond_cooldown_then_release() {
    let dir = tempfile_dir("unbond");
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    let addr = genesis_reward_address();
    chain
        .register_node_bond(addr, "peer-unbond")
        .expect("bond");
    let req = chain.request_node_unbond(addr).expect("request");
    assert_eq!(req.unlock_after_height, BOND_UNLOCK_COOLDOWN_BLOCKS);
    assert!(!chain.is_node_bond_eligible(&addr));
    assert!(
        chain.finalize_node_unbond(addr).is_err(),
        "cooldown not elapsed"
    );

    let h = chain.height();
    let done = chain
        .store_mut()
        .finalize_node_unbond(&addr, h.saturating_add(BOND_UNLOCK_COOLDOWN_BLOCKS))
        .expect("finalize");
    assert!(done.locked.is_empty());
    assert_eq!(done.unlock_after_height, 0);
    assert_eq!(
        chain.spendable_balance(&addr).atomic(),
        chain.balance(&addr).atomic()
    );
}

#[test]
fn service_attestation_weights_scale() {
    std::env::set_var("MESH_NODE_BOND", "0");
    let dir = tempfile_dir("svc");
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    let addr = genesis_reward_address();
    chain.node_operator = Some(addr);

    assert_eq!(chain.node_reputation_milli(&addr), 0);
    chain
        .credit_local_service(mesh_types::NodeServiceKind::TxRelay, 10)
        .unwrap();
    let tx_w = chain.pending_node_weight(&addr);
    assert_eq!(chain.node_reputation_milli(&addr), 600);
    chain
        .credit_local_service(mesh_types::NodeServiceKind::Archive, 10)
        .unwrap();
    let after = chain.pending_node_weight(&addr);
    assert!(
        after > tx_w,
        "archive 2.0× should credit more than tx 1.0× for same raw weight ({after} vs first {tx_w})"
    );
    assert_eq!(chain.node_reputation_milli(&addr), 800);
    let atts = chain.recent_service_attestations();
    assert_eq!(atts.len(), 2);
    assert_eq!(atts[0].service, mesh_types::NodeServiceKind::TxRelay);
    assert_eq!(atts[1].service, mesh_types::NodeServiceKind::Archive);
    assert!(atts[1].credited > atts[0].credited);
}

#[test]
fn rtt_factor_thresholds() {
    assert_eq!(crate::rtt_factor_milli(None), 1_000);
    assert_eq!(crate::rtt_factor_milli(Some(10)), 1_000);
    assert_eq!(crate::rtt_factor_milli(Some(50)), 1_000);
    assert_eq!(crate::rtt_factor_milli(Some(51)), 850);
    assert_eq!(crate::rtt_factor_milli(Some(200)), 850);
    assert_eq!(crate::rtt_factor_milli(Some(201)), 700);
}

fn inject_slashed_bond(chain: &mut Chain, kp: &Keypair, tag: &[u8]) -> (OutPoint, u64) {
    let addr = kp.address();
    let op = OutPoint::new(Hash::digest(tag), 0);
    let atomic = 25_000_000u64;
    chain.store_mut().test_inject_utxo(
        op,
        Utxo {
            address: addr,
            amount: Amount::from_atomic(atomic),
        },
    );
    chain.store_mut().test_insert_bond(
        &addr,
        NodeBondRec {
            peer_id: "peer-race".into(),
            bonded_at_height: 0,
            stake_atomic: atomic,
            locked: vec![LockedBondUtxo {
                txid_hex: op.txid.to_hex(),
                vout: op.vout,
                atomic,
            }],
            unlock_after_height: 0,
            slashed: true,
            slashed_to_vault_atomic: atomic,
            slashed_at_height: 0,
        },
    );
    (op, atomic)
}

fn resign_settle(kp: &Keypair, mut tx: mesh_types::Transaction) -> mesh_types::Transaction {
    let digest = tx.sighash();
    let pubkey = kp.public_key_bytes().to_vec();
    let sig = kp.sign(digest.as_bytes()).to_vec();
    for inp in &mut tx.inputs {
        inp.pubkey = pubkey.clone();
        inp.signature = sig.clone();
    }
    tx
}

#[test]
fn slash_settle_mempool_first_seen_wins() {
    let dir = tempfile_dir("race_first");
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    let kp = Keypair::generate();
    let (op, atomic) = inject_slashed_bond(&mut chain, &kp, b"race-utxo-1");
    let locked = vec![(
        op,
        Utxo {
            address: kp.address(),
            amount: Amount::from_atomic(atomic),
        },
    )];
    let t1 = crate::build_slash_settle(&kp, &locked).expect("t1");
    let mut t2 = t1.clone();
    t2.memo = format!("{}|alt", t2.memo);
    let t2 = resign_settle(&kp, t2);
    assert_ne!(t1.txid(), t2.txid());

    let id1 = chain.submit_tx(t1.clone()).expect("accept t1");
    assert_eq!(id1, t1.txid());
    let err = chain.submit_tx(t2).expect_err("t2 conflicts");
    assert!(
        err.to_string().contains("mempool input conflict"),
        "got {err}"
    );
    // Second settle submit is idempotent.
    let again = chain.submit_slash_settle(&kp).expect("idempotent");
    assert_eq!(again, t1.txid());
}

#[test]
fn slash_mark_preferred_settle_replaces_race() {
    let dir = tempfile_dir("race_pref");
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    let kp = Keypair::generate();
    let addr = kp.address();
    let (op, atomic) = inject_slashed_bond(&mut chain, &kp, b"race-utxo-2");
    let locked = vec![(
        op,
        Utxo {
            address: addr,
            amount: Amount::from_atomic(atomic),
        },
    )];
    let preferred = crate::build_slash_settle(&kp, &locked).expect("preferred");
    let mut loser = preferred.clone();
    loser.memo = format!("{}|loser", loser.memo);
    let loser = resign_settle(&kp, loser);
    assert_ne!(preferred.txid(), loser.txid());

    chain.submit_tx(loser.clone()).expect("loser first");
    chain
        .apply_slash_mark(addr, 1, atomic, "peer-a", &preferred.txid().to_hex())
        .expect("mark");
    let pref_hex = preferred.txid().to_hex();
    assert_eq!(
        chain.preferred_slash_settle(&addr).as_deref(),
        Some(pref_hex.as_str())
    );
    let id = chain.submit_tx(preferred.clone()).expect("preferred wins");
    assert_eq!(id, preferred.txid());
    assert_eq!(chain.mempool().len(), 1);
    assert_eq!(chain.mempool()[0].txid(), preferred.txid());
}
