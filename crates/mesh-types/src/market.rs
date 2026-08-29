//! PoMC market proof / receipt types (Build/14, Build/15).

use serde::{Deserialize, Serialize};

use crate::{Address, Hash};

/// Which isolated reward market a contribution belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketKind {
    Cpu,
    Gpu,
    Node,
}

/// Protocol-assigned device role for a MeshHash-Evo period (Build/31).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    PowCpu,
    AiGpu,
    Protocol,
    VerifyAssist,
}

impl DeviceRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PowCpu => "pow_cpu",
            Self::AiGpu => "ai_gpu",
            Self::Protocol => "protocol",
            Self::VerifyAssist => "verify_assist",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pow_cpu" => Some(Self::PowCpu),
            "ai_gpu" => Some(Self::AiGpu),
            "protocol" => Some(Self::Protocol),
            "verify_assist" => Some(Self::VerifyAssist),
            _ => None,
        }
    }
}

/// Verified AI / benchmark job receipt — credits **GPU market only**.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiJobReceipt {
    pub job_id: String,
    /// Worker paid from GPU market.
    pub worker: Address,
    pub input_commitment: Hash,
    pub output_hash: Hash,
    pub latency_ms: u64,
    /// Weight toward `S_gpu` (benchmark units).
    pub weight: u64,
    /// Unix secs when verified.
    pub verified_at: u64,
    pub job_kind: AiJobKind,
    /// Research scenario id when `job_kind == ProtocolEval` (Build/18).
    #[serde(default)]
    pub research_scenario: String,
    /// Primary sim health 0..=1 (ProtocolEval only).
    #[serde(default)]
    pub score_primary: f64,
    #[serde(default)]
    pub score_orphan_risk: f64,
    #[serde(default)]
    pub score_detect_rate: f64,
    #[serde(default)]
    pub score_linkability: f64,
    #[serde(default)]
    pub score_backlog_ratio: f64,
    #[serde(default)]
    pub score_latency_p95_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiJobKind {
    Echo,
    Benchmark,
    /// Protocol-eval research job — still credits GPU market only (Build/15).
    ProtocolEval,
    /// Removed product surface; kept for wire/serde compat with old receipts.
    AgentAssist,
    /// Real MNIST SGD training — verified by re-execution (deterministic f64).
    MlTrain,
}

/// Jobs that earn GPU-lane research units after useful-work height.
pub fn is_paid_research_kind(kind: AiJobKind) -> bool {
    matches!(kind, AiJobKind::ProtocolEval | AiJobKind::MlTrain)
}

/// Placeholder GPU PoW share (MeshHash-GPU) — credits GPU market only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuShare {
    pub worker: Address,
    pub commitment: Hash,
    pub nonce: u64,
    pub pow: Hash,
    pub weight: u64,
}

/// Node service attestation — credits **node market only**.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeServiceAttestation {
    pub operator: Address,
    pub service: NodeServiceKind,
    pub weight: u64,
    /// Soft weight after service BPS multiplier (what was credited).
    #[serde(default)]
    pub credited: u64,
    pub attested_at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeServiceKind {
    TxRelay,
    BlockRelay,
    Snapshot,
    Archive,
    AiRouting,
}

impl NodeServiceKind {
    /// Relative BPS multiplier for node-market credit (1000 = 1.0×).
    pub fn weight_bps(self) -> u64 {
        match self {
            Self::TxRelay => 1_000,
            Self::BlockRelay => 1_500,
            Self::Snapshot => 2_000,
            Self::Archive => 2_000,
            Self::AiRouting => 1_200,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TxRelay => "tx_relay",
            Self::BlockRelay => "block_relay",
            Self::Snapshot => "snapshot",
            Self::Archive => "archive",
            Self::AiRouting => "ai_routing",
        }
    }
}
