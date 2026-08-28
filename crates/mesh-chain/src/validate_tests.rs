//! Consensus header / block validation coverage (Build/05).

use crate::validate::{validate_block_header, MAX_FUTURE_SKEW_SECS};
use crate::{build_market_coinbase, genesis_reward_address, Chain};
use mesh_types::{Address, Block, BlockHeader, Hash};
use meshhash_cpu::{meshhash_cpu_with_params, MeshHashParams};
use std::collections::HashMap;

fn coinbase(height: u64) -> mesh_types::Transaction {
    build_market_coinbase(
        height,
        genesis_reward_address(),
        &HashMap::new(),
        &HashMap::new(),
    )
}

fn block_at(height: u64, prev_hash: Hash, timestamp: u64, difficulty: u32) -> Block {
    let txs = vec![coinbase(height)];
    let merkle_root = Block::merkle_root(&txs);
    Block {
        header: BlockHeader {
            version: 1,
            prev_hash,
            merkle_root,
            timestamp,
            height,
            difficulty,
            nonce: 0,
        },
        txs,
    }
}

fn mine_light(block: &mut Block) {
    let commitment = block.header.pre_pow_commitment();
    let params = MeshHashParams::light();
    for nonce in 0..500_000u64 {
        block.header.nonce = nonce;
        let pow = meshhash_cpu_with_params(&commitment, nonce, &params);
        if pow.meets_difficulty(block.header.difficulty) {
            return;
        }
    }
    panic!("failed to mine light PoW");
}

#[test]
fn rejects_empty_block() {
    let b = Block {
        header: BlockHeader {
            version: 1,
            prev_hash: Hash::zero(),
            merkle_root: Hash::zero(),
            timestamp: 1_700_000_000,
            height: 0,
            difficulty: 1,
            nonce: 0,
        },
        txs: vec![],
    };
    let err = validate_block_header(&b, None, true, None).expect_err("empty");
    assert!(err.to_string().contains("empty"));
}

#[test]
fn rejects_bad_merkle_root() {
    let mut b = block_at(0, Hash::zero(), 1_700_000_000, 1);
    b.header.merkle_root = Hash::digest(b"tampered");
    mine_light(&mut b);
    let err = validate_block_header(&b, None, true, None).expect_err("merkle");
    assert!(err.to_string().contains("merkle"));
}

#[test]
fn rejects_height_and_prev_mismatch() {
    let mut genesis = block_at(0, Hash::zero(), 1_700_000_000, 1);
    mine_light(&mut genesis);
    validate_block_header(&genesis, None, true, None).expect("genesis ok");

    let mut next = block_at(2, genesis.id(), 1_700_000_001, 1);
    mine_light(&mut next);
    let err = validate_block_header(&next, Some(&genesis), true, Some(1)).expect_err("height");
    assert!(err.to_string().contains("height"));

    let mut bad_prev = block_at(1, Hash::digest(b"wrong"), 1_700_000_001, 1);
    mine_light(&mut bad_prev);
    let err = validate_block_header(&bad_prev, Some(&genesis), true, Some(1)).expect_err("prev");
    assert!(err.to_string().contains("prev"));
}

#[test]
fn rejects_non_increasing_timestamp() {
    let mut genesis = block_at(0, Hash::zero(), 1_700_000_000, 1);
    mine_light(&mut genesis);
    let mut next = block_at(1, genesis.id(), genesis.header.timestamp, 1);
    mine_light(&mut next);
    let err = validate_block_header(&next, Some(&genesis), true, Some(1)).expect_err("ts");
    assert!(err.to_string().contains("timestamp"));
}

#[test]
fn rejects_far_future_timestamp() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut b = block_at(0, Hash::zero(), now + MAX_FUTURE_SKEW_SECS + 60, 1);
    mine_light(&mut b);
    let err = validate_block_header(&b, None, true, None).expect_err("future");
    assert!(err.to_string().contains("future"));
}

#[test]
fn import_rejects_non_extending_height() {
    std::env::set_var("MESH_LIGHT_POW", "1");
    let dir = std::env::temp_dir().join(format!(
        "mm_val_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    chain.light_pow = true;
    let tip = chain.height();
    let miner = Address::from_pubkey_bytes(b"validate-miner");
    let mined = chain
        .mine_next(miner, 2_000_000)
        .expect("mine")
        .expect("found");
    assert_eq!(mined.header.height, tip + 1);

    // Equal-or-lower / gap heights are parked as orphans (no hard error).
    let mut stale = chain.mining_template(miner);
    stale.header.height = tip;
    assert!(Chain::search_pow(&mut stale, true, 2_000_000));
    assert_eq!(chain.import_block(stale).expect("ignore stale"), false);

    let mut gap = chain.mining_template(miner);
    gap.header.height = tip + 3;
    gap.txs[0] = coinbase(tip + 3);
    gap.header.merkle_root = Block::merkle_root(&gap.txs);
    assert!(Chain::search_pow(&mut gap, true, 2_000_000));
    assert_eq!(chain.import_block(gap).expect("orphan gap"), false);
}

#[test]
fn rejects_wrong_coinbase_maturity_tag() {
    let mut b = block_at(0, Hash::zero(), 1_700_000_000, 1);
    assert!(
        b.txs[0].memo.contains("|mat:20"),
        "builder must stamp |mat:20: {}",
        b.txs[0].memo
    );
    b.txs[0].memo = b.txs[0].memo.replace("|mat:20", "|mat:1");
    b.header.merkle_root = Block::merkle_root(&b.txs);
    mine_light(&mut b);
    let err = crate::validate::validate_block_header(&b, None, true, Some(1)).expect_err("mat");
    assert!(
        err.to_string().contains("mat:"),
        "expected maturity reject, got {err}"
    );
}
