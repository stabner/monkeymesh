//! Difficulty retarget toward [`TARGET_BLOCK_TIME_SECS`] (5s).
//! Retarget interval / step / min floor may be bounded by active envelopes (Build/30).

use mesh_types::{Block, ProtocolEnvelopes, TARGET_BLOCK_TIME_SECS};

/// Leading-zero bits used for the first post-genesis blocks.
pub const INITIAL_DIFFICULTY: u32 = 10;

/// Default retarget every N blocks. Live public testnet is **15**;
/// the old 20 default forked testers at height 150.
pub const RETARGET_INTERVAL: u64 = 15;

pub const MIN_DIFFICULTY: u32 = 1;
pub const MAX_DIFFICULTY: u32 = 48;

/// Consensus retarget knobs (from envelopes or defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetargetParams {
    pub interval: u64,
    pub step: u32,
    pub min_floor: u32,
}

impl Default for RetargetParams {
    fn default() -> Self {
        Self {
            interval: RETARGET_INTERVAL,
            step: 1,
            min_floor: MIN_DIFFICULTY,
        }
    }
}

impl RetargetParams {
    pub fn from_envelopes(env: &ProtocolEnvelopes) -> Self {
        let clamped = env.clone().clamp();
        Self {
            interval: clamped.retarget_interval,
            step: clamped.retarget_step,
            min_floor: clamped.min_difficulty_floor,
        }
    }

    pub fn min_diff(self) -> u32 {
        self.min_floor.max(MIN_DIFFICULTY).min(MAX_DIFFICULTY)
    }
}

/// Compute the difficulty required for the next block given the current tip chain.
///
/// `blocks` must be ordered by height ascending and include genesis..tip.
pub fn next_difficulty(blocks: &[Block]) -> u32 {
    next_difficulty_with(blocks, RetargetParams::default())
}

pub fn next_difficulty_with(blocks: &[Block], params: RetargetParams) -> u32 {
    let Some(tip) = blocks.last() else {
        return 1; // pre-genesis
    };

    if tip.header.height == 0 {
        return INITIAL_DIFFICULTY.max(params.min_diff());
    }

    let next_height = tip.header.height + 1;
    let interval = params.interval.max(1);
    if next_height % interval != 0 {
        return tip
            .header
            .difficulty
            .clamp(params.min_diff(), MAX_DIFFICULTY);
    }

    let start_height = next_height.saturating_sub(interval);
    let Some(start) = blocks.iter().find(|b| b.header.height == start_height) else {
        return tip.header.difficulty.max(params.min_diff());
    };

    next_difficulty_from_window_with(tip, start, params)
}

/// Retarget using only tip + epoch-start block (O(1) — avoid cloning the full chain).
pub fn next_difficulty_from_window(tip: &Block, start: &Block) -> u32 {
    next_difficulty_from_window_with(tip, start, RetargetParams::default())
}

pub fn next_difficulty_from_window_with(tip: &Block, start: &Block, params: RetargetParams) -> u32 {
    if tip.header.height == 0 {
        return INITIAL_DIFFICULTY.max(params.min_diff());
    }
    let next_height = tip.header.height + 1;
    let interval = params.interval.max(1);
    if next_height % interval != 0 {
        return tip
            .header
            .difficulty
            .clamp(params.min_diff(), MAX_DIFFICULTY);
    }

    let actual = tip
        .header
        .timestamp
        .saturating_sub(start.header.timestamp)
        .max(1);
    // Clamp observed span so a timestamp grind cannot collapse difficulty in one epoch.
    let expected = interval.saturating_mul(TARGET_BLOCK_TIME_SECS);
    let actual = actual.clamp(expected / 4, expected.saturating_mul(4).max(1));

    adjust(tip.header.difficulty, actual, expected, params)
}

fn adjust(current: u32, actual_secs: u64, expected_secs: u64, params: RetargetParams) -> u32 {
    let step = params.step.clamp(1, 2);
    let mut diff = current;
    if actual_secs < expected_secs.saturating_mul(3) / 4 {
        diff = diff.saturating_add(step);
    } else if actual_secs > expected_secs.saturating_mul(4) / 3 {
        diff = diff.saturating_sub(step);
    }
    diff.clamp(params.min_diff(), MAX_DIFFICULTY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_types::{Address, BlockHeader, Hash, Transaction};

    fn dummy_block(height: u64, timestamp: u64, difficulty: u32) -> Block {
        let tx = Transaction::coinbase(
            mesh_types::Amount::from_atomic(1),
            Address::default(),
            height,
        );
        Block {
            header: BlockHeader {
                version: 1,
                prev_hash: Hash::zero(),
                merkle_root: Hash::zero(),
                timestamp,
                height,
                difficulty,
                nonce: 0,
            },
            txs: vec![tx],
        }
    }

    #[test]
    fn after_genesis_uses_initial() {
        let blocks = vec![dummy_block(0, 1_700_000_000, 1)];
        assert_eq!(next_difficulty(&blocks), INITIAL_DIFFICULTY);
    }

    #[test]
    fn fast_epoch_raises_difficulty() {
        let mut blocks = vec![dummy_block(0, 1000, 1)];
        // heights 1..14 at difficulty 10, timestamps 1s apart (very fast vs 5s target)
        for h in 1..15 {
            blocks.push(dummy_block(h, 1000 + h, 10));
        }
        // next height 15 → retarget
        assert_eq!(next_difficulty(&blocks), 11);
    }

    #[test]
    fn envelope_floor_raises_min() {
        let mut blocks = vec![dummy_block(0, 1000, 1)];
        for h in 1..20 {
            blocks.push(dummy_block(h, 1000 + h * 10, 5));
        }
        let params = RetargetParams {
            interval: 20,
            step: 1,
            min_floor: 8,
        };
        let d = next_difficulty_with(&blocks, params);
        assert!(d >= 8);
    }

    #[test]
    fn step_two_can_jump_two() {
        let mut blocks = vec![dummy_block(0, 1000, 1)];
        for h in 1..20 {
            blocks.push(dummy_block(h, 1000 + h, 10)); // very fast
        }
        let params = RetargetParams {
            interval: 20,
            step: 2,
            min_floor: 1,
        };
        assert_eq!(next_difficulty_with(&blocks, params), 12);
    }
}
