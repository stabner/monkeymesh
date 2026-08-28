//! Bincode meta migrations for ProtocolEnvelopes growth (Build/30).
//! On-disk ChainMeta before Build/30 used envelopes without retarget fields.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use mesh_types::{
    AiJobReceipt, ImprovementCertificate, ParamEpoch, ParamProposal, ProposalStatus, ProposalVote,
    ProtocolEnvelopes, Transaction,
};

use crate::store::{NodeBondRec, NodeBondRecLocked, NodeBondRecSoft};

/// Exact pre-Build/30 envelope layout (14 fields).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct ProtocolEnvelopesPreV30 {
    pub soft_adapt_signal_threshold: f64,
    pub soft_benchmark_rounds: u32,
    pub min_verifier_weight: u64,
    pub suggested_cpu_diff_bias: i32,
    pub idle_stipend_bps_cap: u16,
    pub brain_prefer_v2: u8,
    pub brain_v2_min_workers: u32,
    pub brain_v2_vram_floor_mb: u32,
    pub leg_train_enable: u8,
    pub leg_parallel: u32,
    pub leg_harden_sec_floor: u8,
    pub quantum_train_enable: u8,
    pub quantum_parallel: u32,
    pub brain_audit_every: u16,
}

impl From<ProtocolEnvelopesPreV30> for ProtocolEnvelopes {
    fn from(e: ProtocolEnvelopesPreV30) -> Self {
        ProtocolEnvelopes {
            soft_adapt_signal_threshold: e.soft_adapt_signal_threshold,
            soft_benchmark_rounds: e.soft_benchmark_rounds,
            min_verifier_weight: e.min_verifier_weight,
            suggested_cpu_diff_bias: e.suggested_cpu_diff_bias,
            idle_stipend_bps_cap: e.idle_stipend_bps_cap,
            brain_prefer_v2: e.brain_prefer_v2,
            brain_v2_min_workers: e.brain_v2_min_workers,
            brain_v2_vram_floor_mb: e.brain_v2_vram_floor_mb,
            leg_train_enable: e.leg_train_enable,
            leg_parallel: e.leg_parallel,
            leg_harden_sec_floor: e.leg_harden_sec_floor,
            quantum_train_enable: e.quantum_train_enable,
            quantum_parallel: e.quantum_parallel,
            brain_audit_every: e.brain_audit_every,
            ..ProtocolEnvelopes::default()
        }
        .clamp()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ParamProposalPreV30 {
    pub id: String,
    pub created_at_height: u64,
    pub rationale: String,
    pub envelopes: ProtocolEnvelopesPreV30,
    pub status: ProposalStatus,
    pub suggested_cpu_bps: u16,
    pub suggested_gpu_bps: u16,
    pub suggested_node_bps: u16,
    #[serde(default)]
    pub votes: Vec<ProposalVote>,
}

impl From<ParamProposalPreV30> for ParamProposal {
    fn from(p: ParamProposalPreV30) -> Self {
        ParamProposal {
            id: p.id,
            created_at_height: p.created_at_height,
            rationale: p.rationale,
            envelopes: ProtocolEnvelopes::from(p.envelopes),
            status: p.status,
            suggested_cpu_bps: p.suggested_cpu_bps,
            suggested_gpu_bps: p.suggested_gpu_bps,
            suggested_node_bps: p.suggested_node_bps,
            votes: p.votes,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct ParamEpochPreV30 {
    pub epoch: u64,
    pub height: u64,
    pub proposal_id: String,
    pub rationale: String,
    pub eval_count: u64,
    pub envelopes: ProtocolEnvelopesPreV30,
}

impl From<ParamEpochPreV30> for ParamEpoch {
    fn from(e: ParamEpochPreV30) -> Self {
        ParamEpoch {
            epoch: e.epoch,
            height: e.height,
            proposal_id: e.proposal_id,
            rationale: e.rationale,
            eval_count: e.eval_count,
            envelopes: ProtocolEnvelopes::from(e.envelopes),
        }
    }
}

/// On-disk meta immediately before Build/30 (bonds + 14-field envelopes).
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct ChainMetaPreV30 {
    pub mempool: Vec<Transaction>,
    #[serde(default)]
    pub gpu_scores: HashMap<String, u64>,
    #[serde(default)]
    pub node_scores: HashMap<String, u64>,
    #[serde(default)]
    pub ai_receipts: Vec<AiJobReceipt>,
    #[serde(default)]
    pub proposals: Vec<ParamProposalPreV30>,
    #[serde(default)]
    pub active_envelopes: ProtocolEnvelopesPreV30,
    #[serde(default)]
    pub next_proposal_id: u64,
    #[serde(default)]
    pub last_auto_adapt_at_height: u64,
    #[serde(default)]
    pub last_auto_adapt_proposal_id: String,
    #[serde(default)]
    pub last_auto_adapt_eval_count: u64,
    #[serde(default)]
    pub param_epoch: u64,
    #[serde(default)]
    pub epoch_history: Vec<ParamEpochPreV30>,
    #[serde(default)]
    pub node_bonds: HashMap<String, NodeBondRec>,
}

/// Build/30 meta: retarget envelopes + improvement certificates.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct ChainMeta {
    pub mempool: Vec<Transaction>,
    #[serde(default)]
    pub gpu_scores: HashMap<String, u64>,
    #[serde(default)]
    pub node_scores: HashMap<String, u64>,
    #[serde(default)]
    pub ai_receipts: Vec<AiJobReceipt>,
    #[serde(default)]
    pub proposals: Vec<ParamProposal>,
    #[serde(default)]
    pub active_envelopes: ProtocolEnvelopes,
    #[serde(default)]
    pub next_proposal_id: u64,
    #[serde(default)]
    pub last_auto_adapt_at_height: u64,
    #[serde(default)]
    pub last_auto_adapt_proposal_id: String,
    #[serde(default)]
    pub last_auto_adapt_eval_count: u64,
    #[serde(default)]
    pub param_epoch: u64,
    #[serde(default)]
    pub epoch_history: Vec<ParamEpoch>,
    #[serde(default)]
    pub node_bonds: HashMap<String, NodeBondRec>,
    #[serde(default)]
    pub last_retarget_adapt_grover_count: u64,
    #[serde(default)]
    pub improvement_certs: Vec<ImprovementCertificate>,
}

impl From<ChainMetaPreV30> for ChainMeta {
    fn from(m: ChainMetaPreV30) -> Self {
        Self {
            mempool: m.mempool,
            gpu_scores: m.gpu_scores,
            node_scores: m.node_scores,
            ai_receipts: m.ai_receipts,
            proposals: m.proposals.into_iter().map(ParamProposal::from).collect(),
            active_envelopes: ProtocolEnvelopes::from(m.active_envelopes),
            next_proposal_id: m.next_proposal_id,
            last_auto_adapt_at_height: m.last_auto_adapt_at_height,
            last_auto_adapt_proposal_id: m.last_auto_adapt_proposal_id,
            last_auto_adapt_eval_count: m.last_auto_adapt_eval_count,
            param_epoch: m.param_epoch,
            epoch_history: m.epoch_history.into_iter().map(ParamEpoch::from).collect(),
            node_bonds: m.node_bonds,
            last_retarget_adapt_grover_count: 0,
            improvement_certs: Vec::new(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct ChainMetaPreBond {
    pub mempool: Vec<Transaction>,
    #[serde(default)]
    pub gpu_scores: HashMap<String, u64>,
    #[serde(default)]
    pub node_scores: HashMap<String, u64>,
    #[serde(default)]
    pub ai_receipts: Vec<AiJobReceipt>,
    #[serde(default)]
    pub proposals: Vec<ParamProposalPreV30>,
    #[serde(default)]
    pub active_envelopes: ProtocolEnvelopesPreV30,
    #[serde(default)]
    pub next_proposal_id: u64,
    #[serde(default)]
    pub last_auto_adapt_at_height: u64,
    #[serde(default)]
    pub last_auto_adapt_proposal_id: String,
    #[serde(default)]
    pub last_auto_adapt_eval_count: u64,
    #[serde(default)]
    pub param_epoch: u64,
    #[serde(default)]
    pub epoch_history: Vec<ParamEpochPreV30>,
}

impl From<ChainMetaPreBond> for ChainMeta {
    fn from(m: ChainMetaPreBond) -> Self {
        ChainMeta::from(ChainMetaPreV30 {
            mempool: m.mempool,
            gpu_scores: m.gpu_scores,
            node_scores: m.node_scores,
            ai_receipts: m.ai_receipts,
            proposals: m.proposals,
            active_envelopes: m.active_envelopes,
            next_proposal_id: m.next_proposal_id,
            last_auto_adapt_at_height: m.last_auto_adapt_at_height,
            last_auto_adapt_proposal_id: m.last_auto_adapt_proposal_id,
            last_auto_adapt_eval_count: m.last_auto_adapt_eval_count,
            param_epoch: m.param_epoch,
            epoch_history: m.epoch_history,
            node_bonds: HashMap::new(),
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct ChainMetaSoftBonds {
    pub mempool: Vec<Transaction>,
    #[serde(default)]
    pub gpu_scores: HashMap<String, u64>,
    #[serde(default)]
    pub node_scores: HashMap<String, u64>,
    #[serde(default)]
    pub ai_receipts: Vec<AiJobReceipt>,
    #[serde(default)]
    pub proposals: Vec<ParamProposalPreV30>,
    #[serde(default)]
    pub active_envelopes: ProtocolEnvelopesPreV30,
    #[serde(default)]
    pub next_proposal_id: u64,
    #[serde(default)]
    pub last_auto_adapt_at_height: u64,
    #[serde(default)]
    pub last_auto_adapt_proposal_id: String,
    #[serde(default)]
    pub last_auto_adapt_eval_count: u64,
    #[serde(default)]
    pub param_epoch: u64,
    #[serde(default)]
    pub epoch_history: Vec<ParamEpochPreV30>,
    #[serde(default)]
    pub node_bonds: HashMap<String, NodeBondRecSoft>,
}

impl From<ChainMetaSoftBonds> for ChainMeta {
    fn from(m: ChainMetaSoftBonds) -> Self {
        ChainMeta::from(ChainMetaPreV30 {
            mempool: m.mempool,
            gpu_scores: m.gpu_scores,
            node_scores: m.node_scores,
            ai_receipts: m.ai_receipts,
            proposals: m.proposals,
            active_envelopes: m.active_envelopes,
            next_proposal_id: m.next_proposal_id,
            last_auto_adapt_at_height: m.last_auto_adapt_at_height,
            last_auto_adapt_proposal_id: m.last_auto_adapt_proposal_id,
            last_auto_adapt_eval_count: m.last_auto_adapt_eval_count,
            param_epoch: m.param_epoch,
            epoch_history: m.epoch_history,
            node_bonds: m
                .node_bonds
                .into_iter()
                .map(|(k, v)| (k, NodeBondRec::from(v)))
                .collect(),
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct ChainMetaLockedBonds {
    pub mempool: Vec<Transaction>,
    #[serde(default)]
    pub gpu_scores: HashMap<String, u64>,
    #[serde(default)]
    pub node_scores: HashMap<String, u64>,
    #[serde(default)]
    pub ai_receipts: Vec<AiJobReceipt>,
    #[serde(default)]
    pub proposals: Vec<ParamProposalPreV30>,
    #[serde(default)]
    pub active_envelopes: ProtocolEnvelopesPreV30,
    #[serde(default)]
    pub next_proposal_id: u64,
    #[serde(default)]
    pub last_auto_adapt_at_height: u64,
    #[serde(default)]
    pub last_auto_adapt_proposal_id: String,
    #[serde(default)]
    pub last_auto_adapt_eval_count: u64,
    #[serde(default)]
    pub param_epoch: u64,
    #[serde(default)]
    pub epoch_history: Vec<ParamEpochPreV30>,
    #[serde(default)]
    pub node_bonds: HashMap<String, NodeBondRecLocked>,
}

impl From<ChainMetaLockedBonds> for ChainMeta {
    fn from(m: ChainMetaLockedBonds) -> Self {
        ChainMeta::from(ChainMetaPreV30 {
            mempool: m.mempool,
            gpu_scores: m.gpu_scores,
            node_scores: m.node_scores,
            ai_receipts: m.ai_receipts,
            proposals: m.proposals,
            active_envelopes: m.active_envelopes,
            next_proposal_id: m.next_proposal_id,
            last_auto_adapt_at_height: m.last_auto_adapt_at_height,
            last_auto_adapt_proposal_id: m.last_auto_adapt_proposal_id,
            last_auto_adapt_eval_count: m.last_auto_adapt_eval_count,
            param_epoch: m.param_epoch,
            epoch_history: m.epoch_history,
            node_bonds: m
                .node_bonds
                .into_iter()
                .map(|(k, v)| (k, NodeBondRec::from(v)))
                .collect(),
        })
    }
}

pub(crate) fn deserialize_meta(bytes: &[u8]) -> Result<ChainMeta, String> {
    if let Ok(m) = bincode::deserialize::<ChainMeta>(bytes) {
        return Ok(m);
    }
    if let Ok(m) = bincode::deserialize::<ChainMetaPreV30>(bytes) {
        return Ok(ChainMeta::from(m));
    }
    if let Ok(m) = bincode::deserialize::<ChainMetaLockedBonds>(bytes) {
        return Ok(ChainMeta::from(m));
    }
    if let Ok(m) = bincode::deserialize::<ChainMetaSoftBonds>(bytes) {
        return Ok(ChainMeta::from(m));
    }
    bincode::deserialize::<ChainMetaPreBond>(bytes)
        .map(ChainMeta::from)
        .map_err(|e| e.to_string())
}
