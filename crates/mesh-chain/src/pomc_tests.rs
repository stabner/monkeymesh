use crate::{
    block_reward, build_market_coinbase, cpu_market_reward, cpu_market_reward_with,
    deferred_gpu_vault, deferred_node_vault, genesis_reward_address, gpu_market_reward,
    gpu_market_reward_with, node_market_reward,
};
use mesh_types::{Address, Amount};
use std::collections::HashMap;

#[test]
fn empty_scores_pay_vaults() {
    let finder = genesis_reward_address();
    let tx = build_market_coinbase(1, finder, &HashMap::new(), &HashMap::new());
    let (h, n_gpu, n_node) = tx.parse_pomc_memo().expect("memo");
    assert_eq!(h, 1);
    assert_eq!(n_node, 1);
    assert_eq!(tx.outputs[0].amount, cpu_market_reward(1));
    let gpu = gpu_market_reward(1);
    if mesh_types::helper_floor_active(1) {
        let exam_amt = gpu.split_bps(mesh_types::HELPER_EXAM_FLOOR_BPS);
        let fusion_amt = Amount::from_atomic(gpu.atomic().saturating_sub(exam_amt.atomic()));
        assert_eq!(n_gpu, 2);
        assert_eq!(tx.outputs.len(), 4);
        assert_eq!(tx.outputs[1].address, deferred_gpu_vault());
        assert_eq!(tx.outputs[1].amount, exam_amt);
        assert_eq!(tx.outputs[2].address, finder);
        assert_eq!(tx.outputs[2].amount, fusion_amt);
        assert_eq!(tx.outputs[3].address, deferred_node_vault());
        assert_eq!(tx.outputs[3].amount, node_market_reward(1));
        assert_eq!(tx.parse_pomc_exam_count(), Some(1));
        assert!(tx.memo.contains("|exam:1"));
    } else {
        assert_eq!(n_gpu, 1);
        assert_eq!(tx.outputs.len(), 3);
        assert_eq!(tx.outputs[1].address, finder);
        assert_eq!(tx.outputs[1].amount, gpu);
        assert_eq!(tx.outputs[2].address, deferred_node_vault());
    }
    assert_eq!(tx.total_output(), block_reward(1));
    assert!(
        tx.memo.contains(&format!("|mat:{}", mesh_types::COINBASE_MATURITY)),
        "coinbase must stamp consensus maturity: {}",
        tx.memo
    );
}

#[test]
fn genesis_gpu_scores_do_not_eat_cpu_market() {
    let mut scores = HashMap::new();
    let a = Address::from_pubkey_bytes(b"gpu-worker-a");
    scores.insert(a.to_hex(), 3u64);
    let tx = build_market_coinbase(0, genesis_reward_address(), &scores, &HashMap::new());
    assert_eq!(tx.outputs[0].amount, cpu_market_reward(0));
    assert_eq!(tx.outputs[0].amount, cpu_market_reward_with(0, 3));
    assert_eq!(tx.outputs[1].address, a);
    assert_eq!(tx.outputs[1].amount, gpu_market_reward_with(0, 3));
    assert_eq!(tx.outputs[2].address, deferred_node_vault());
}

#[test]
fn helper_floor_cpu_exam_and_gpu_finder_share_gpu_lane() {
    let h = 80u64;
    if !mesh_types::helper_floor_active(h) {
        return;
    }
    let finder = genesis_reward_address();
    let helper = Address::from_pubkey_bytes(b"cpu-helper");
    let mut scores = HashMap::new();
    scores.insert(helper.to_hex(), mesh_types::EXAM_LANE_UNITS);
    let tx = build_market_coinbase(h, finder, &scores, &HashMap::new());
    assert_eq!(tx.outputs[0].address, finder);
    assert_eq!(tx.outputs[0].amount, cpu_market_reward(h));
    let gpu = gpu_market_reward(h);
    let exam_amt = gpu.split_bps(mesh_types::HELPER_EXAM_FLOOR_BPS);
    let fusion_amt = Amount::from_atomic(gpu.atomic().saturating_sub(exam_amt.atomic()));
    assert_eq!(tx.outputs[1].address, helper);
    assert_eq!(tx.outputs[1].amount, exam_amt);
    assert_eq!(tx.outputs[2].address, finder);
    assert_eq!(tx.outputs[2].amount, fusion_amt);
    assert_eq!(tx.parse_pomc_exam_count(), Some(1));
    assert_eq!(tx.total_output(), block_reward(h));
}

#[test]
fn fair_split_gpu_units_cannot_eat_cpu_lane() {
    let h = mesh_types::DEFAULT_FAIR_SPLIT_HEIGHT;
    if !mesh_types::fair_lane_split_active(h) {
        return;
    }
    let finder = genesis_reward_address();
    let mut scores = HashMap::new();
    let a = Address::from_pubkey_bytes(b"gpu-exam-worker");
    scores.insert(a.to_hex(), 50_000u64);
    let tx = build_market_coinbase(h, finder, &scores, &HashMap::new());
    assert_eq!(tx.outputs[0].amount, cpu_market_reward(h));
    assert_eq!(tx.outputs[0].amount, cpu_market_reward_with(h, 50_000));
    let gpu = gpu_market_reward(h);
    if mesh_types::helper_floor_active(h) {
        let exam_amt = gpu.split_bps(mesh_types::HELPER_EXAM_FLOOR_BPS);
        assert_eq!(tx.outputs[1].address, a);
        assert_eq!(tx.outputs[1].amount, exam_amt);
        assert_eq!(tx.outputs[2].address, finder);
    } else {
        assert_eq!(tx.outputs[1].address, finder);
        assert_eq!(tx.outputs[1].amount, gpu);
    }
    assert_eq!(tx.total_output(), block_reward(h));
}
