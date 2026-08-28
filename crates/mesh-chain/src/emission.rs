use mesh_types::{
    Amount, CONTRIB_BLOCK_UNITS, CONTRIBUTOR_MARKET_BPS, CPU_MARKET_BPS, FAIR_CPU_LANE_BPS,
    FAIR_GPU_LANE_BPS, GPU_MARKET_BPS, SUPPLY_CAP_MESH, DECIMALS, fair_lane_split_active,
    node_market_bps_at, shared_contrib_active,
};

/// ~4 years at 5 s (`365.25 × 24 × 3600 / 5 ≈ 6_311_520` × 4).
pub const ERA_BLOCKS: u64 = 25_228_800;

/// Total atomic units in the supply cap (`SUPPLY_CAP_MESH × 10^DECIMALS`).
pub fn supply_cap_atomic() -> u64 {
    SUPPLY_CAP_MESH.saturating_mul(10u64.pow(DECIMALS))
}

fn initial_subsidy_atomic() -> u64 {
    50 * 10u64.pow(DECIMALS)
}

fn scheduled_subsidy_atomic(height: u64) -> u64 {
    initial_subsidy_atomic() >> (height / ERA_BLOCKS).min(63)
}

/// Atomic MESH minted by blocks `0 .. height` (exclusive of `height`).
pub fn emitted_before_atomic(height: u64) -> u128 {
    if height == 0 {
        return 0;
    }
    let last = height - 1;
    let last_era = last / ERA_BLOCKS;
    let initial = initial_subsidy_atomic();
    let mut total = 0u128;
    for era in 0..=last_era {
        let start = era * ERA_BLOCKS;
        let end = last.min((era + 1) * ERA_BLOCKS - 1);
        let n = end.saturating_sub(start).saturating_add(1) as u128;
        let sub = (initial >> era.min(63)) as u128;
        total = total.saturating_add(n.saturating_mul(sub));
    }
    total
}

/// Subsidy at `height`, clamped so cumulative issuance never exceeds the cap.
pub fn block_reward(height: u64) -> Amount {
    let scheduled = scheduled_subsidy_atomic(height);
    let already = emitted_before_atomic(height);
    let cap = u128::from(supply_cap_atomic());
    let room = cap.saturating_sub(already);
    let pay = scheduled.min(u64::try_from(room).unwrap_or(u64::MAX));
    Amount::from_atomic(pay)
}

/// CPU / block-finder share of the block subsidy.
/// Legacy: fixed 40%. Build/31: share of the 90% pot. Fair split: fixed 45%.
pub fn cpu_market_reward(height: u64) -> Amount {
    cpu_market_reward_with(height, 0)
}

/// GPU / Fusion-lane-B + exam share. Legacy: 40%. Fair split: fixed 45%.
pub fn gpu_market_reward(height: u64) -> Amount {
    gpu_market_reward_with(height, 0)
}

/// Node market share (20% legacy, 10% after Build/31 gate).
pub fn node_market_reward(height: u64) -> Amount {
    block_reward(height).split_bps(node_market_bps_at(height))
}

/// Block-finder amount given pending GPU unit sum.
/// After the fair-split height, `gpu_units` is ignored — CPU always gets 45%.
pub fn cpu_market_reward_with(height: u64, gpu_units: u64) -> Amount {
    if fair_lane_split_active(height) {
        return block_reward(height).split_bps(FAIR_CPU_LANE_BPS);
    }
    if !shared_contrib_active(height) {
        return block_reward(height).split_bps(CPU_MARKET_BPS);
    }
    let (miner, _) = split_contributor(height, gpu_units);
    miner
}

pub fn gpu_market_reward_with(height: u64, gpu_units: u64) -> Amount {
    if fair_lane_split_active(height) {
        return block_reward(height).split_bps(FAIR_GPU_LANE_BPS);
    }
    if !shared_contrib_active(height) {
        return block_reward(height).split_bps(GPU_MARKET_BPS);
    }
    let (_, gpu) = split_contributor(height, gpu_units);
    gpu
}

/// Copy GPU scores and add the finder's Fusion lane-B credit (fair split only).
///
/// When `gpu_pay_requires_exam` is on, a finder with **no** exam score does not
/// receive Fusion GPU units — CPU-only shops cannot vacuum the GPU 45%.
pub fn gpu_scores_with_fusion_credit(
    height: u64,
    finder: mesh_types::Address,
    gpu_scores: &std::collections::HashMap<String, u64>,
) -> std::collections::HashMap<String, u64> {
    let mut out = gpu_scores.clone();
    if !fair_lane_split_active(height) {
        return out;
    }
    if mesh_types::gpu_pay_requires_exam(height) && !out.contains_key(&finder.to_hex()) {
        return out;
    }
    let e = out.entry(finder.to_hex()).or_insert(0);
    *e = e.saturating_add(mesh_types::FUSION_GPU_UNITS);
    out
}

fn split_contributor(height: u64, gpu_units: u64) -> (Amount, Amount) {
    let pot = block_reward(height).split_bps(CONTRIBUTOR_MARKET_BPS);
    let miner_u = CONTRIB_BLOCK_UNITS;
    let den = miner_u.saturating_add(gpu_units.max(0));
    if den == 0 {
        return (pot, Amount::ZERO);
    }
    let miner_atomic = pot.atomic().saturating_mul(miner_u) / den;
    let miner = Amount::from_atomic(miner_atomic);
    let gpu = Amount::from_atomic(pot.atomic().saturating_sub(miner_atomic));
    (miner, gpu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_types::DEFAULT_SHARED_BPS_HEIGHT;
    use std::collections::HashMap;

    #[test]
    fn markets_sum_to_full_subsidy() {
        let full = block_reward(0);
        let cpu = cpu_market_reward(0);
        let gpu = gpu_market_reward(0);
        let node = node_market_reward(0);
        assert_eq!(cpu.atomic(), full.atomic() * 40 / 100);
        assert_eq!(gpu.atomic(), full.atomic() * 40 / 100);
        assert_eq!(node.atomic(), full.atomic() * 20 / 100);
        assert_eq!(
            cpu.atomic() + gpu.atomic() + node.atomic(),
            full.atomic()
        );
    }

    #[test]
    fn shared_pot_sums_to_full_subsidy() {
        let h = DEFAULT_SHARED_BPS_HEIGHT;
        let full = block_reward(h);
        let node = node_market_reward(h);
        assert_eq!(node.atomic(), full.atomic() * 10 / 100);
        let cpu = cpu_market_reward_with(h, 0);
        let gpu = gpu_market_reward_with(h, 0);
        assert_eq!(cpu.atomic() + gpu.atomic() + node.atomic(), full.atomic());
        let cpu2 = cpu_market_reward_with(h, 1_000);
        let gpu2 = gpu_market_reward_with(h, 1_000);
        assert_eq!(cpu2.atomic() + gpu2.atomic() + node.atomic(), full.atomic());
        if mesh_types::fair_lane_split_active(h) {
            assert_eq!(cpu2.atomic(), cpu.atomic());
            assert_eq!(gpu2.atomic(), gpu.atomic());
        } else {
            assert!(cpu2.atomic() < cpu.atomic());
        }
    }

    #[test]
    fn fair_lanes_are_equal_and_isolated() {
        let h = mesh_types::DEFAULT_FAIR_SPLIT_HEIGHT;
        if !mesh_types::fair_lane_split_active(h) {
            return;
        }
        let full = block_reward(h);
        let cpu = cpu_market_reward_with(h, 0);
        let gpu = gpu_market_reward_with(h, 0);
        let cpu2 = cpu_market_reward_with(h, 50_000);
        let gpu2 = gpu_market_reward_with(h, 50_000);
        let node = node_market_reward(h);
        assert_eq!(cpu.atomic(), full.atomic() * 45 / 100);
        assert_eq!(gpu.atomic(), full.atomic() * 45 / 100);
        assert_eq!(node.atomic(), full.atomic() * 10 / 100);
        assert_eq!(cpu.atomic(), cpu2.atomic());
        assert_eq!(gpu.atomic(), gpu2.atomic());
        assert_eq!(
            cpu.atomic() + gpu.atomic() + node.atomic(),
            full.atomic()
        );
    }

    #[test]
    fn honest_cap_is_two_times_first_era() {
        assert_eq!(SUPPLY_CAP_MESH, 50 * ERA_BLOCKS * 2);
        assert_eq!(emitted_before_atomic(0), 0);
        assert_eq!(emitted_before_atomic(1), 50 * 10u64.pow(DECIMALS) as u128);
        assert_eq!(block_reward(0).atomic(), 50 * 10u64.pow(DECIMALS));
        let era0 = u128::from(50 * 10u64.pow(DECIMALS)) * u128::from(ERA_BLOCKS);
        assert_eq!(emitted_before_atomic(ERA_BLOCKS), era0);
        let after = emitted_before_atomic(ERA_BLOCKS) + u128::from(block_reward(ERA_BLOCKS).atomic());
        assert!(after <= u128::from(supply_cap_atomic()));
    }

    #[test]
    fn fusion_credit_without_exam_until_gate() {
        let finder = mesh_types::Address::from_pubkey_bytes(b"fusion-finder");
        let empty = HashMap::new();
        let out = gpu_scores_with_fusion_credit(80, finder, &empty);
        if mesh_types::gpu_pay_requires_exam(80) {
            assert!(
                out.is_empty(),
                "exam gate must not gift GPU 45% to a CPU-only finder"
            );
        } else {
            assert_eq!(
                out.get(&finder.to_hex()).copied(),
                Some(mesh_types::FUSION_GPU_UNITS)
            );
        }
    }
}
