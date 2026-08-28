//! Display labels for PoMC coinbase outputs (not consensus).
//!
//! Layout: output 0 = CPU finder, then `n_gpu` GPU outs, then `n_node` node outs.
//! Helper-floor GPU outs are exam helpers first, Fusion finder last.
//! New coinbases tag that split as `pomc:v1:{h}:{n_gpu}:{n_node}|mat:20` (optional `|exam:{n}`).
//! Older memos without `exam:` keep a single "GPU lane" label so we never guess.

use serde::Serialize;

use crate::Address;

/// Pubkey tag for the deferred GPU vault (must match `mesh_chain::deferred_gpu_vault`).
pub const GPU_VAULT_PUBKEY_TAG: &[u8] = b"MonkeyMesh/vault/gpu/v1";
/// Pubkey tag for the deferred node vault (must match `mesh_chain::deferred_node_vault`).
pub const NODE_VAULT_PUBKEY_TAG: &[u8] = b"MonkeyMesh/vault/node/v1";

pub fn gpu_vault_address() -> Address {
    Address::from_pubkey_bytes(GPU_VAULT_PUBKEY_TAG)
}

pub fn node_vault_address() -> Address {
    Address::from_pubkey_bytes(NODE_VAULT_PUBKEY_TAG)
}

pub fn is_gpu_vault_address(addr: &Address) -> bool {
    *addr == gpu_vault_address()
}

pub fn is_node_vault_address(addr: &Address) -> bool {
    *addr == node_vault_address()
}

/// Work that produced a coinbase output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoinbaseLane {
    CpuFind,
    GpuExam,
    GpuFusion,
    GpuLane,
    NodeWork,
    Other,
}

impl CoinbaseLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CpuFind => "cpu_find",
            Self::GpuExam => "gpu_exam",
            Self::GpuFusion => "gpu_fusion",
            Self::GpuLane => "gpu_lane",
            Self::NodeWork => "node_work",
            Self::Other => "other",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::CpuFind => "Fusion seal · 45%",
            Self::GpuExam => "GPU work · helper share",
            Self::GpuFusion => "GPU work · 45%",
            Self::GpuLane => "GPU work · 45%",
            Self::NodeWork => "Node work · 10%",
            Self::Other => "Payment",
        }
    }

    /// What the miner/node did to earn this output.
    pub fn paid_for(self) -> &'static str {
        match self {
            Self::CpuFind => {
                "CPU sealed the Fusion digest on this tip. Always 45% to the finder."
            }
            Self::GpuExam => {
                "You MATCH’d the immune exam. Helpers share the GPU 45% lane."
            }
            Self::GpuFusion => {
                "GPU work on the sealed pad (finder share of the GPU 45% lane)."
            }
            Self::GpuLane => {
                "GPU work lane (legacy coinbase, or no exam-count tag on the memo)."
            }
            Self::NodeWork => {
                "Attested node useful work (relay / routing). Not a mining find."
            }
            Self::Other => "Incoming payment.",
        }
    }

    pub fn is_gpu(self) -> bool {
        matches!(self, Self::GpuExam | Self::GpuFusion | Self::GpuLane)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PomcLayout {
    pub height: u64,
    pub n_gpu: usize,
    pub n_node: usize,
    /// Exam helper outputs inside the GPU slice. `None` = do not split GPU labels.
    pub n_exam: Option<usize>,
    /// `|mat:{n}` — must match [`crate::COINBASE_MATURITY`] when present.
    pub maturity: Option<u64>,
}

/// Parse `pomc:v1:{height}:{n_gpu}:{n_node}` (optional `|…` suffix).
pub fn parse_pomc_layout(memo: &str) -> Option<PomcLayout> {
    let mut segs = memo.split('|');
    let main = segs.next()?;
    let rest = main.strip_prefix("pomc:v1:")?;
    let mut parts = rest.split(':');
    let height: u64 = parts.next()?.parse().ok()?;
    let n_gpu: usize = parts.next()?.parse().ok()?;
    let n_node: usize = parts.next()?.parse().ok()?;
    let mut n_exam = None;
    let mut maturity = None;
    for extra in segs {
        if let Some(n) = extra.strip_prefix("exam:") {
            if let Ok(v) = n.parse::<usize>() {
                n_exam = Some(v);
            }
        } else if let Some(n) = extra.strip_prefix("mat:") {
            if let Ok(v) = n.parse::<u64>() {
                maturity = Some(v);
            }
        }
    }
    Some(PomcLayout {
        height,
        n_gpu,
        n_node,
        n_exam,
        maturity,
    })
}

pub fn coinbase_lane(memo: &str, vout: u32, n_outputs: usize) -> CoinbaseLane {
    let Some(lay) = parse_pomc_layout(memo) else {
        return CoinbaseLane::Other;
    };
    let idx = vout as usize;
    if idx >= n_outputs {
        return CoinbaseLane::Other;
    }
    if idx == 0 {
        return CoinbaseLane::CpuFind;
    }
    if idx <= lay.n_gpu {
        return match lay.n_exam {
            Some(n_exam) if n_exam > 0 && idx <= n_exam => CoinbaseLane::GpuExam,
            Some(_) => CoinbaseLane::GpuFusion,
            None => CoinbaseLane::GpuLane,
        };
    }
    CoinbaseLane::NodeWork
}

#[derive(Clone, Copy, Debug)]
pub struct CoinbasePayoutLabel {
    pub lane: CoinbaseLane,
    pub title: &'static str,
    pub paid_for: &'static str,
    pub vault: bool,
}

pub fn coinbase_payout_label(
    memo: &str,
    vout: u32,
    n_outputs: usize,
    address: Option<&Address>,
) -> CoinbasePayoutLabel {
    let lane = coinbase_lane(memo, vout, n_outputs);
    let vault = match address {
        Some(a) if lane.is_gpu() && is_gpu_vault_address(a) => true,
        Some(a) if lane == CoinbaseLane::NodeWork && is_node_vault_address(a) => true,
        _ => false,
    };
    let (title, paid_for) = if vault {
        match lane {
            CoinbaseLane::GpuExam => (
                "Unclaimed GPU helpers (vault)",
                "No rematched exam this window — that share of the GPU 45% sits in the GPU vault.",
            ),
            CoinbaseLane::GpuFusion | CoinbaseLane::GpuLane => (
                "Unclaimed GPU lane (vault)",
                "No GPU-lane wallet this window — this slice sits in the GPU vault.",
            ),
            CoinbaseLane::NodeWork => (
                "Unclaimed node pot (vault)",
                "No attested node scores this window — the 10% sits in the node vault.",
            ),
            _ => (lane.title(), lane.paid_for()),
        }
    } else {
        (lane.title(), lane.paid_for())
    };
    CoinbasePayoutLabel {
        lane,
        title,
        paid_for,
        vault,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_three_outs_are_undivided_gpu() {
        let memo = "pomc:v1:10:1:1";
        assert_eq!(coinbase_lane(memo, 0, 3), CoinbaseLane::CpuFind);
        assert_eq!(coinbase_lane(memo, 1, 3), CoinbaseLane::GpuLane);
        assert_eq!(coinbase_lane(memo, 2, 3), CoinbaseLane::NodeWork);
    }

    #[test]
    fn helper_floor_exam_suffix() {
        let memo = "pomc:v1:80:2:1|exam:1";
        assert_eq!(coinbase_lane(memo, 0, 4), CoinbaseLane::CpuFind);
        assert_eq!(coinbase_lane(memo, 1, 4), CoinbaseLane::GpuExam);
        assert_eq!(coinbase_lane(memo, 2, 4), CoinbaseLane::GpuFusion);
        assert_eq!(coinbase_lane(memo, 3, 4), CoinbaseLane::NodeWork);
    }

    #[test]
    fn several_exam_helpers_then_fusion() {
        let memo = "pomc:v1:90:4:1|exam:3";
        assert_eq!(coinbase_lane(memo, 1, 6), CoinbaseLane::GpuExam);
        assert_eq!(coinbase_lane(memo, 3, 6), CoinbaseLane::GpuExam);
        assert_eq!(coinbase_lane(memo, 4, 6), CoinbaseLane::GpuFusion);
    }

    #[test]
    fn genesis_suffix_is_ignored() {
        let memo = "pomc:v1:0:1:1|MonkeyMesh genesis — adaptive compute network";
        assert_eq!(coinbase_lane(memo, 1, 3), CoinbaseLane::GpuLane);
        assert!(parse_pomc_layout(memo).unwrap().n_exam.is_none());
        assert!(parse_pomc_layout(memo).unwrap().maturity.is_none());
    }

    #[test]
    fn maturity_tag_parses() {
        let memo = "pomc:v1:10:1:1|mat:20";
        let lay = parse_pomc_layout(memo).unwrap();
        assert_eq!(lay.maturity, Some(20));
        assert_eq!(coinbase_lane(memo, 0, 3), CoinbaseLane::CpuFind);
    }

    #[test]
    fn vault_overlay() {
        let memo = "pomc:v1:1:2:1|exam:1";
        let lab = coinbase_payout_label(memo, 1, 4, Some(&gpu_vault_address()));
        assert!(lab.vault);
        assert_eq!(lab.lane, CoinbaseLane::GpuExam);
        assert!(lab.title.contains("vault"));
    }
}
