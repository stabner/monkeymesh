//! Lab finality (Build/36 F2). Env-gated; serialized so other tests stay default-off.

use std::sync::Mutex;

use crate::{
    build_market_coinbase, finality_activation_height, FinalityAttestation, DEFAULT_FINALITY_HEIGHT,
    Chain,
};
use mesh_crypto::Keypair;
use mesh_types::{Address, Block, Hash};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    keys: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set(pairs: &[(&'static str, &str)]) -> Self {
        let keys = pairs
            .iter()
            .map(|(k, v)| {
                let prev = std::env::var(k).ok();
                std::env::set_var(k, v);
                (*k, prev)
            })
            .collect();
        Self { keys }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, prev) in self.keys.drain(..) {
            match prev {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mm_finality_{}_{}_{}",
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

fn open_light(dir: &std::path::Path) -> Chain {
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    chain.light_pow = true;
    chain
}

fn mine_n(chain: &mut Chain, miner: Address, n: usize) {
    for _ in 0..n {
        chain
            .mine_next(miner, 5_000_000)
            .expect("mine")
            .expect("found");
    }
}

fn mine_child(chain: &Chain, parent: &Block, miner: Address) -> Block {
    let height = parent.header.height.saturating_add(1);
    let mut b = chain.mining_template(miner);
    b.header.height = height;
    b.header.prev_hash = parent.id();
    b.header.timestamp = parent.header.timestamp.saturating_add(5);
    b.header.difficulty = parent.header.difficulty;
    b.txs = vec![build_market_coinbase(
        height,
        miner,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
    )];
    b.header.merkle_root = Block::merkle_root(&b.txs);
    assert!(
        Chain::search_pow(&mut b, true, 5_000_000),
        "failed to mine fork child at {height}"
    );
    b
}

fn lab_env() -> EnvGuard {
    EnvGuard::set(&[
        ("MESH_FINALITY_HEIGHT", "1"),
        ("MESH_FINALITY_WINDOW", "2"),
        ("MESH_FINALITY_MIN_ATTESTORS", "1"),
        ("MESH_FINALITY_THRESHOLD_BPS", "1"),
        ("MESH_FINALITY_MIN_BOND_ATOMIC", "1"),
        ("MESH_FINALITY_BOND_AGE", "0"),
    ])
}

fn bond_operator(chain: &mut Chain) -> Keypair {
    let kp = Keypair::generate();
    let addr = kp.address();
    mine_n(chain, addr, 1);
    chain
        .register_node_bond(addr, "finality-peer")
        .expect("bond");
    assert!(chain.is_finality_attestor(&addr));
    kp
}

fn sign_h(chain: &Chain, kp: &Keypair, h: u64) -> FinalityAttestation {
    let block = chain.get_block(h).expect("block");
    FinalityAttestation::sign(kp, chain.genesis_hash(), h, block.id())
}

#[test]
fn default_gate_is_off() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _env = EnvGuard::set(&[]);
    std::env::remove_var("MESH_FINALITY_HEIGHT");
    std::env::remove_var("MESH_FINALITY_WINDOW");
    std::env::remove_var("MESH_FINALITY_MIN_ATTESTORS");
    assert_eq!(finality_activation_height(), DEFAULT_FINALITY_HEIGHT);

    let dir = tempfile_dir("off");
    let mut chain = open_light(&dir);
    let kp = Keypair::generate();
    mine_n(&mut chain, kp.address(), 3);
    let _ = chain.register_node_bond(kp.address(), "off-peer");
    let att = sign_h(&chain, &kp, 1);
    let ingest = chain.record_finality_attestation(att);
    match ingest {
        Ok(ing) => assert!(!ing.advanced, "default-off must not finalize"),
        Err(_) => {}
    }
    assert_eq!(chain.finalized_height(), 0);
}

#[test]
fn attest_then_refuse_reorg_of_finalized() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _env = lab_env();

    let dir = tempfile_dir("on");
    let mut chain = open_light(&dir);
    let kp = bond_operator(&mut chain);
    mine_n(&mut chain, kp.address(), 2);
    assert!(chain.height() >= 3);

    let ingest = chain
        .record_finality_attestation(sign_h(&chain, &kp, 1))
        .expect("attest");
    assert!(ingest.new_vote);
    assert!(ingest.advanced);
    assert_eq!(chain.finalized_height(), 1);
    let cand = chain.get_block(1).unwrap().id();
    assert_eq!(chain.finalized_hash(), cand);

    drop(chain);
    let mut chain = open_light(&dir);
    assert_eq!(chain.finalized_height(), 1);
    assert_eq!(chain.finalized_hash(), cand);
    assert!(!chain.pending_finality_attestations().is_empty());

    chain.test_lock_tip_finality();
    let tip_h = chain.height();
    let tip_id = chain.tip_hash();
    let parent = chain.get_block(tip_h.saturating_sub(1)).expect("parent");
    let mut refused = false;
    for i in 0..40 {
        let sibling = mine_child(
            &chain,
            &parent,
            Address::from_pubkey_bytes(format!("fs{i}").as_bytes()),
        );
        assert_eq!(sibling.header.height, tip_h);
        match chain.import_block(sibling) {
            Ok(true) => panic!("must not replace a finalized tip"),
            Ok(false) => assert_eq!(chain.tip_hash(), tip_id),
            Err(e) => {
                assert!(
                    e.to_string().contains("finalized") || e.to_string().contains("pop"),
                    "expected finalized refusal, got {e}"
                );
                refused = true;
                break;
            }
        }
    }
    assert!(
        refused,
        "a better-work sibling of a finalized tip must be refused"
    );
}

#[test]
fn equivocation_slashes_bond() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _env = lab_env();

    let dir = tempfile_dir("slash");
    let mut chain = open_light(&dir);
    let kp = bond_operator(&mut chain);
    mine_n(&mut chain, kp.address(), 2);
    chain
        .record_finality_attestation(sign_h(&chain, &kp, 1))
        .expect("first vote");
    let flip = FinalityAttestation::sign(&kp, chain.genesis_hash(), 1, Hash::digest(b"other-fork"));
    let ingest = chain.record_finality_attestation(flip).expect("slash path");
    assert!(ingest.slashed.is_some());
    assert!(!chain.is_finality_attestor(&kp.address()));
    assert!(!chain.is_node_bond_eligible(&kp.address()));
}

#[test]
fn wrong_genesis_is_rejected() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _env = lab_env();
    let dir = tempfile_dir("genesis");
    let mut chain = open_light(&dir);
    let kp = bond_operator(&mut chain);
    mine_n(&mut chain, kp.address(), 2);
    let block = chain.get_block(1).unwrap();
    let att = FinalityAttestation::sign(&kp, Hash::digest(b"other-net"), 1, block.id());
    let err = chain.record_finality_attestation(att).expect_err("genesis");
    assert!(err.to_string().contains("genesis") || err.to_string().contains("signature"));
}

#[test]
fn fresh_tip_cannot_be_attested() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _env = lab_env();
    let dir = tempfile_dir("fresh");
    let mut chain = open_light(&dir);
    let kp = bond_operator(&mut chain);
    mine_n(&mut chain, kp.address(), 2);
    let tip = chain.height();
    let err = chain
        .record_finality_attestation(sign_h(&chain, &kp, tip))
        .expect_err("tip too fresh");
    assert!(err.to_string().contains("window"));
}

#[test]
fn door_fee_bond_cannot_vote() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _env = EnvGuard::set(&[
        ("MESH_FINALITY_HEIGHT", "1"),
        ("MESH_FINALITY_WINDOW", "2"),
        ("MESH_FINALITY_MIN_ATTESTORS", "1"),
        ("MESH_FINALITY_THRESHOLD_BPS", "1"),
        ("MESH_FINALITY_BOND_AGE", "0"),
        ("MESH_FINALITY_MIN_BOND_ATOMIC", "10000000000"),
    ]);
    let dir = tempfile_dir("door");
    let mut chain = open_light(&dir);
    let kp = Keypair::generate();
    mine_n(&mut chain, kp.address(), 1);
    chain
        .register_node_bond(kp.address(), "door-peer")
        .expect("0.1-class bond ok");
    assert!(chain.is_node_bond_eligible(&kp.address()));
    assert!(!chain.is_finality_attestor(&kp.address()));
    mine_n(&mut chain, kp.address(), 2);
    let err = chain
        .record_finality_attestation(sign_h(&chain, &kp, 1))
        .expect_err("door fee");
    assert!(err.to_string().contains("bond"));
}

#[test]
fn attestation_sign_verify_roundtrip() {
    let kp = Keypair::generate();
    let genesis = Hash::digest(b"genesis");
    let h = Hash::digest(b"finality-roundtrip");
    let att = FinalityAttestation::sign(&kp, genesis, 42, h);
    assert!(att.verify());
    assert_eq!(att.operator(), Some(kp.address()));
    let mut bad = att.clone();
    bad.height = 43;
    assert!(!bad.verify());
    let other = FinalityAttestation::sign(&kp, Hash::digest(b"else"), 42, h);
    assert!(other.verify());
    assert_ne!(other.signature_hex, att.signature_hex);
}
