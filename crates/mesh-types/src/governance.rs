//! Governance / adaptive envelopes (Build/11, Build/15, Build/21, Build/30).
//! AI may propose; soft envelopes auto-apply as param epochs.
//! Bounded retarget knobs may auto-apply from quantum-gated certificates (Build/30).
//! Humans alone change BPS / crypto.

use serde::{Deserialize, Serialize};

/// Soft + bounded consensus-adjacent envelopes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProtocolEnvelopes {
    pub soft_adapt_signal_threshold: f64,
    pub soft_benchmark_rounds: u32,
    pub min_verifier_weight: u64,
    pub suggested_cpu_diff_bias: i32,
    pub idle_stipend_bps_cap: u16,
    /// 0 = v1 shared brain only; 1 = prefer v2 when capable workers online (Build/24).
    #[serde(default = "default_brain_prefer_v2")]
    pub brain_prefer_v2: u8,
    /// Minimum workers advertising `cuda_v2` before seed enqueues v2 jobs.
    #[serde(default = "default_brain_v2_min_workers")]
    pub brain_v2_min_workers: u32,
    /// Ignore workers below this VRAM when counting v2 capacity.
    #[serde(default = "default_brain_v2_vram_floor_mb")]
    pub brain_v2_vram_floor_mb: u32,
    /// Enable Trilemma Guardian leg training (Build/25).
    #[serde(default = "default_leg_train_enable")]
    pub leg_train_enable: u8,
    /// Max simultaneous leg-train jobs per research tick.
    #[serde(default = "default_leg_parallel")]
    pub leg_parallel: u32,
    /// If Trilemma `sec` is below this (0–100), soft-harden verifier floor.
    #[serde(default = "default_leg_harden_sec_floor")]
    pub leg_harden_sec_floor: u8,
    /// Enable Quantum Research Guardian training (Build/26).
    #[serde(default = "default_quantum_train_enable")]
    pub quantum_train_enable: u8,
    /// Max simultaneous quantum-train jobs per research tick.
    #[serde(default = "default_quantum_parallel")]
    pub quantum_parallel: u32,
    /// Full re-exec 1-in-K for light jobs (protocol/benchmark). `1` = always verify.
    /// Shared brain / guardians always full-verify (integrity).
    #[serde(default = "default_brain_audit_every")]
    pub brain_audit_every: u16,
    /// Blocks between difficulty retargets (Build/30). Consensus-affecting, clamped.
    #[serde(default = "default_retarget_interval")]
    pub retarget_interval: u64,
    /// Max leading-zero bits moved per retarget epoch (1..=2).
    #[serde(default = "default_retarget_step")]
    pub retarget_step: u32,
    /// Floor for consensus difficulty (1..=16).
    #[serde(default = "default_min_difficulty_floor")]
    pub min_difficulty_floor: u32,
}

fn default_brain_prefer_v2() -> u8 {
    1
}
fn default_brain_v2_min_workers() -> u32 {
    1
}
fn default_brain_v2_vram_floor_mb() -> u32 {
    4_096
}
fn default_leg_train_enable() -> u8 {
    1
}
fn default_leg_parallel() -> u32 {
    2
}
fn default_leg_harden_sec_floor() -> u8 {
    55
}
fn default_quantum_train_enable() -> u8 {
    1
}
fn default_quantum_parallel() -> u32 {
    1
}
fn default_brain_audit_every() -> u16 {
    1
}
fn default_retarget_interval() -> u64 {
    15
}
fn default_retarget_step() -> u32 {
    1
}
fn default_min_difficulty_floor() -> u32 {
    1
}

/// Minimum verified `quantum_grover` certificates since last retarget adapt.
pub const MIN_GROVER_CERTS_FOR_RETARGET: u64 = 5;

/// Verified research receipt used as an Improvement Certificate (Build/30).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImprovementCertificate {
    pub scenario: String,
    pub primary: f64,
    pub height: u64,
    pub job_id: String,
    pub verified_at: u64,
}

impl Default for ProtocolEnvelopes {
    fn default() -> Self {
        Self {
            soft_adapt_signal_threshold: 0.5,
            soft_benchmark_rounds: 2_000,
            min_verifier_weight: 1,
            suggested_cpu_diff_bias: 0,
            idle_stipend_bps_cap: 1_000,
            brain_prefer_v2: default_brain_prefer_v2(),
            brain_v2_min_workers: default_brain_v2_min_workers(),
            brain_v2_vram_floor_mb: default_brain_v2_vram_floor_mb(),
            leg_train_enable: default_leg_train_enable(),
            leg_parallel: default_leg_parallel(),
            leg_harden_sec_floor: default_leg_harden_sec_floor(),
            quantum_train_enable: default_quantum_train_enable(),
            quantum_parallel: default_quantum_parallel(),
            brain_audit_every: default_brain_audit_every(),
            retarget_interval: default_retarget_interval(),
            retarget_step: default_retarget_step(),
            min_difficulty_floor: default_min_difficulty_floor(),
        }
    }
}

impl ProtocolEnvelopes {
    pub fn clamp(mut self) -> Self {
        self.soft_adapt_signal_threshold = self.soft_adapt_signal_threshold.clamp(0.05, 5.0);
        self.soft_benchmark_rounds = self.soft_benchmark_rounds.clamp(100, 50_000);
        self.min_verifier_weight = self.min_verifier_weight.clamp(1, 1_000_000);
        self.suggested_cpu_diff_bias = self.suggested_cpu_diff_bias.clamp(-2, 2);
        self.idle_stipend_bps_cap = self.idle_stipend_bps_cap.min(2_000);
        self.brain_prefer_v2 = if self.brain_prefer_v2 > 0 { 1 } else { 0 };
        self.brain_v2_min_workers = self.brain_v2_min_workers.clamp(1, 64);
        self.brain_v2_vram_floor_mb = self.brain_v2_vram_floor_mb.clamp(1_024, 98_304);
        self.leg_train_enable = if self.leg_train_enable > 0 { 1 } else { 0 };
        self.leg_parallel = self.leg_parallel.clamp(1, 4);
        self.leg_harden_sec_floor = self.leg_harden_sec_floor.clamp(10, 95);
        self.quantum_train_enable = if self.quantum_train_enable > 0 { 1 } else { 0 };
        self.quantum_parallel = self.quantum_parallel.clamp(1, 3);
        self.brain_audit_every = self.brain_audit_every.clamp(1, 64);
        self.retarget_interval = self.retarget_interval.clamp(10, 40);
        self.retarget_step = self.retarget_step.clamp(1, 2);
        self.min_difficulty_floor = self.min_difficulty_floor.clamp(1, 16);
        self
    }

    /// Copy retarget knobs from `other` (freeze when quantum gate not met).
    pub fn freeze_retarget_from(&mut self, other: &Self) {
        self.retarget_interval = other.retarget_interval;
        self.retarget_step = other.retarget_step;
        self.min_difficulty_floor = other.min_difficulty_floor;
    }

    /// Limit retarget knob movement to one safe step vs `prev`.
    pub fn limit_retarget_jump(&mut self, prev: &Self) {
        let pi = prev.retarget_interval;
        self.retarget_interval = self
            .retarget_interval
            .clamp(pi.saturating_sub(5), pi.saturating_add(5))
            .clamp(10, 40);
        if self.retarget_step > prev.retarget_step {
            self.retarget_step = prev.retarget_step.saturating_add(1).min(2);
        } else if self.retarget_step < prev.retarget_step {
            self.retarget_step = prev.retarget_step.saturating_sub(1).max(1);
        }
        if self.min_difficulty_floor > prev.min_difficulty_floor {
            self.min_difficulty_floor = prev.min_difficulty_floor.saturating_add(1).min(16);
        } else if self.min_difficulty_floor < prev.min_difficulty_floor {
            self.min_difficulty_floor = prev.min_difficulty_floor.saturating_sub(1).max(1);
        }
    }

    pub fn retarget_changed(&self, other: &Self) -> bool {
        self.retarget_interval != other.retarget_interval
            || self.retarget_step != other.retarget_step
            || self.min_difficulty_floor != other.min_difficulty_floor
    }
}

/// One soft-envelope adaptation step (not a tip fork).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ParamEpoch {
    pub epoch: u64,
    pub height: u64,
    pub proposal_id: String,
    pub rationale: String,
    pub eval_count: u64,
    pub envelopes: ProtocolEnvelopes,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Activated,
    Rejected,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VoteChoice {
    Yes,
    No,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposalVote {
    /// Stable node identity (libp2p peer id). One vote per proposal.
    pub node_id: String,
    pub choice: VoteChoice,
    pub at_height: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParamProposal {
    pub id: String,
    pub created_at_height: u64,
    pub rationale: String,
    pub envelopes: ProtocolEnvelopes,
    pub status: ProposalStatus,
    pub suggested_cpu_bps: u16,
    pub suggested_gpu_bps: u16,
    pub suggested_node_bps: u16,
    /// Soft-envelope votes — at most one entry per `node_id`.
    #[serde(default)]
    pub votes: Vec<ProposalVote>,
}

impl ParamProposal {
    pub fn vote_counts(&self) -> (usize, usize) {
        let yes = self
            .votes
            .iter()
            .filter(|v| matches!(v.choice, VoteChoice::Yes))
            .count();
        let no = self.votes.len().saturating_sub(yes);
        (yes, no)
    }

    pub fn vote_of(&self, node_id: &str) -> Option<VoteChoice> {
        self.votes
            .iter()
            .find(|v| v.node_id == node_id)
            .map(|v| v.choice)
    }
}

pub const BPS_FLOOR_CPU: u16 = 2_500;
pub const BPS_FLOOR_GPU: u16 = 2_500;
pub const BPS_FLOOR_NODE: u16 = 1_000;
pub const BPS_CEIL_CPU: u16 = 5_000;
pub const BPS_CEIL_GPU: u16 = 5_000;
pub const BPS_CEIL_NODE: u16 = 3_000;
