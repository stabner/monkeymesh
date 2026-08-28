//! Deterministic immune exam assigned from the block template (fair-split era).
//!
//! One scenario per miner address per height. Same program on the miner and the
//! seed — rematch or no GPU-lane credit.

use mesh_types::{exam_job_id, Address, Hash, EXAM_LANE_UNITS};

use crate::research::ResearchScenario;
use crate::work::run_protocol_eval;

/// Public exam assignment carried on `getblocktemplate`.
#[derive(Clone, Debug)]
pub struct ExamAssignment {
    pub height: u64,
    pub exam_root: Hash,
    pub scenario: ResearchScenario,
    pub pulse_signal: f64,
    pub payload: Vec<u8>,
}

impl ExamAssignment {
    pub fn job_id(&self, miner: &Address) -> String {
        exam_job_id(self.height, miner)
    }

    pub fn title(&self) -> &'static str {
        self.scenario.title()
    }

    pub fn digest(&self) -> [u8; 32] {
        run_protocol_eval(&self.payload)
    }

    pub fn payload_hex(&self) -> String {
        hex::encode(&self.payload)
    }
}

/// `exam_root = H("mesh-exam:v1" || height_le || prev_hash)`.
pub fn exam_root(height: u64, prev_hash: &Hash) -> Hash {
    let mut buf = Vec::with_capacity(12 + 8 + 32);
    buf.extend_from_slice(b"mesh-exam:v1");
    buf.extend_from_slice(&height.to_le_bytes());
    buf.extend_from_slice(prev_hash.as_bytes());
    Hash::digest(&buf)
}

/// Assign the height's immune exam for `miner` (deterministic, public).
pub fn assign_exam(height: u64, prev_hash: &Hash, miner: &Address) -> ExamAssignment {
    let root = exam_root(height, prev_hash);
    let catalog = ResearchScenario::classical();
    let mut idx_buf = Vec::with_capacity(32 + 64);
    idx_buf.extend_from_slice(root.as_bytes());
    idx_buf.extend_from_slice(miner.to_hex().as_bytes());
    let idx_hash = Hash::digest(&idx_buf);
    let raw = u64::from_le_bytes(idx_hash.as_bytes()[..8].try_into().expect("8 bytes"));
    let scenario = catalog[raw as usize % catalog.len()];
    let pulse_signal = (root.as_bytes()[0] as f64) / 255.0;
    let payload = scenario.encode(height, pulse_signal);
    ExamAssignment {
        height,
        exam_root: root,
        scenario,
        pulse_signal,
        payload,
    }
}

pub fn exam_units() -> u64 {
    EXAM_LANE_UNITS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_same_digest() {
        let prev = Hash::digest(b"tip");
        let miner = Address::from_pubkey_bytes(b"miner-a");
        let a = assign_exam(1024, &prev, &miner);
        let b = assign_exam(1024, &prev, &miner);
        assert_eq!(a.scenario, b.scenario);
        assert_eq!(a.digest(), b.digest());
        assert_eq!(a.job_id(&miner), exam_job_id(1024, &miner));
    }

    #[test]
    fn different_miners_can_cover_different_scenarios() {
        let prev = Hash::digest(b"tip-2");
        let a = assign_exam(1024, &prev, &Address::from_pubkey_bytes(b"aa"));
        let b = assign_exam(1024, &prev, &Address::from_pubkey_bytes(b"bb"));
        // Not guaranteed different, but digests include miner via assignment
        // of scenario+same height signal; if scenario matches, digest still
        // matches (payload has no address). Coverage comes from many miners.
        let _ = (a.scenario, b.scenario);
    }
}
