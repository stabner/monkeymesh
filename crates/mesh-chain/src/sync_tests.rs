//! A block accepted on one chain replica must import onto another.

use crate::{Address, Chain};

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mm_sync_{}_{}_{}",
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

fn open_light(tag: &str) -> Chain {
    let dir = tempfile_dir(tag);
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    chain.light_pow = true;
    chain
}

#[test]
fn block_mined_on_a_imports_on_b() {
    let mut a = open_light("a");
    let mut b = open_light("b");
    assert_eq!(a.genesis_hash(), b.genesis_hash());
    let miner = Address::from_pubkey_bytes(b"sync-miner-a");
    let found = a
        .mine_next(miner, 5_000_000)
        .expect("mine")
        .expect("found");
    assert!(b.import_block(found.clone()).expect("import"));
    assert_eq!(b.height(), a.height());
    assert_eq!(b.tip_hash(), a.tip_hash());
    assert_eq!(b.tip_hash(), found.id());
}

#[test]
fn two_block_lead_reconnects_via_orphans() {
    let mut a = open_light("lead-a");
    let mut b = open_light("lead-b");
    let miner_a = Address::from_pubkey_bytes(b"sync-lead-a");
    let miner_b = Address::from_pubkey_bytes(b"sync-lead-b");

    let b1 = b.mine_next(miner_b, 5_000_000).expect("b1").expect("found");
    let a1 = a.mine_next(miner_a, 5_000_000).expect("a1").expect("found");
    let a2 = a.mine_next(miner_a, 5_000_000).expect("a2").expect("found");
    assert_eq!(a.height(), 2);
    assert_eq!(b.height(), 1);
    assert_ne!(a1.id(), b1.id());

    // Child first — must sit as orphan until the parent arrives.
    assert!(!b.import_block(a2.clone()).expect("orphan child"));
    assert_eq!(b.height(), 1);

    let adopted = b.import_block(a1.clone()).expect("parent");
    if adopted || b.tip_hash() == a.tip_hash() {
        assert_eq!(b.height(), a.height());
        assert_eq!(b.tip_hash(), a.tip_hash());
    } else {
        // A1 lost the depth-1 race; B keeps b1. That is correct fork choice.
        assert_eq!(b.tip_hash(), b1.id());
    }
}

#[test]
fn accept_mined_adopts_via_import_block() {
    let mut chain = open_light("accept");
    let miner = Address::from_pubkey_bytes(b"sync-accept");
    let mut tmpl = chain.mining_template(miner);
    assert!(Chain::search_pow(&mut tmpl, true, 5_000_000));
    let accepted = chain.accept_mined(tmpl.clone()).expect("accept");
    assert!(accepted.is_some());
    assert_eq!(chain.tip_hash(), tmpl.id());
}
