//! Adaptive protocol research framework (Build/15 Phase 5 / Build/18 v2).
//!
//! Structured scenarios → GPU `protocol_eval` sims → verified digests → MeshPulse
//! research signals → soft param epochs (Build/21).

use serde::{Deserialize, Serialize};

use crate::protocol_sim::{eval_research_input, ResearchResult};
use crate::work::run_protocol_eval;

/// Named research scenarios paid from the GPU 40% market.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchScenario {
    /// Block create + fan-out under peer/latency pressure.
    BlockPropagation,
    /// Invalid shares / spam / verifier dropout.
    SecurityAdversary,
    /// Gossip metadata linkability (heuristic).
    PrivacyLeakage,
    /// Growth under height + queue load.
    ScaleThroughput,
    /// Propose tighter spam / verifier quorum recovery.
    SpamRecovery,
    /// Task-aware routing / capacity use.
    RoutingEfficiency,
    /// Keep markets balanced without CPU absorbing GPU/Node.
    MarketBalance,
    /// Raise redundant verifier expectations under load.
    VerifierQuorum,
    /// Post-quantum signature / hash migration readiness (Build/26).
    QuantumPqc,
    /// PoW / search resistance under √N speedup modeling (Build/26).
    QuantumGrover,
    /// Harvest-now-decrypt-later long-lived secrecy (Build/26).
    QuantumHarvest,
}

impl ResearchScenario {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BlockPropagation => "block_propagation",
            Self::SecurityAdversary => "security_adversary",
            Self::PrivacyLeakage => "privacy_leakage",
            Self::ScaleThroughput => "scale_throughput",
            Self::SpamRecovery => "spam_recovery",
            Self::RoutingEfficiency => "routing_efficiency",
            Self::MarketBalance => "market_balance",
            Self::VerifierQuorum => "verifier_quorum",
            Self::QuantumPqc => "quantum_pqc",
            Self::QuantumGrover => "quantum_grover",
            Self::QuantumHarvest => "quantum_harvest",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "block_propagation" | "block" | "propagation" => Some(Self::BlockPropagation),
            "security_adversary" | "security" | "adversary" => Some(Self::SecurityAdversary),
            "privacy_leakage" | "privacy" => Some(Self::PrivacyLeakage),
            "scale_throughput" | "scale" | "throughput" => Some(Self::ScaleThroughput),
            "spam_recovery" | "spam" => Some(Self::SpamRecovery),
            "routing_efficiency" | "routing" => Some(Self::RoutingEfficiency),
            "market_balance" | "balance" => Some(Self::MarketBalance),
            "verifier_quorum" | "verifier" => Some(Self::VerifierQuorum),
            "quantum_pqc" | "pqc" => Some(Self::QuantumPqc),
            "quantum_grover" | "grover" => Some(Self::QuantumGrover),
            "quantum_harvest" | "harvest" | "hndl" => Some(Self::QuantumHarvest),
            _ => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::BlockPropagation,
            Self::SecurityAdversary,
            Self::PrivacyLeakage,
            Self::ScaleThroughput,
            Self::SpamRecovery,
            Self::RoutingEfficiency,
            Self::MarketBalance,
            Self::VerifierQuorum,
            Self::QuantumPqc,
            Self::QuantumGrover,
            Self::QuantumHarvest,
        ]
    }

    /// Miner-facing title (what the GPU is actually testing).
    pub fn title(self) -> &'static str {
        match self {
            Self::BlockPropagation => "Propagation / orphan risk",
            Self::SecurityAdversary => "Spam / invalid shares",
            Self::PrivacyLeakage => "Gossip privacy leak",
            Self::ScaleThroughput => "Scale / backlog",
            Self::SpamRecovery => "Spam recovery",
            Self::RoutingEfficiency => "Routing efficiency",
            Self::MarketBalance => "CPU/GPU market firewall",
            Self::VerifierQuorum => "Verifier quorum",
            Self::QuantumPqc => "PQC migration readiness",
            Self::QuantumGrover => "Grover / search pressure",
            Self::QuantumHarvest => "Harvest-now decrypt-later",
        }
    }

    /// Hot-path immune exam — this chain, not quantum theater.
    pub fn classical() -> &'static [Self] {
        &[
            Self::BlockPropagation,
            Self::SecurityAdversary,
            Self::PrivacyLeakage,
            Self::ScaleThroughput,
            Self::SpamRecovery,
            Self::RoutingEfficiency,
            Self::MarketBalance,
            Self::VerifierQuorum,
        ]
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::BlockPropagation => "block creation - propagation / orphan risk under peer growth",
            Self::SecurityAdversary => "security - adversarial shares, spam, verifier dropout",
            Self::PrivacyLeakage => "privacy - gossip metadata linkability heuristic",
            Self::ScaleThroughput => "scale - throughput / backlog as the mesh grows",
            Self::SpamRecovery => "abuse recovery - tighter spam / verifier quorum proposals",
            Self::RoutingEfficiency => "GPU capacity - task-aware soft routing signals",
            Self::MarketBalance => "PoMC firewall - markets stay isolated under load",
            Self::VerifierQuorum => "verification - raise weight floor under congestion",
            Self::QuantumPqc => "quantum PQC - classical→post-quantum migration readiness",
            Self::QuantumGrover => "quantum Grover - PoW resilience under √N search speedup",
            Self::QuantumHarvest => "quantum harvest - recorded ciphertext aging / HNDL risk",
        }
    }

    /// Wire payload for `protocol_eval` (deterministic v2).
    pub fn encode(self, height: u64, pulse_signal: f64) -> Vec<u8> {
        format!(
            "mesh-research:v2:{}:h={}:sig={:.6}",
            self.as_str(),
            height,
            pulse_signal
        )
        .into_bytes()
    }

    /// Expected digest after GPU protocol sim.
    pub fn expected_digest(self, height: u64, pulse_signal: f64) -> [u8; 32] {
        run_protocol_eval(&self.encode(height, pulse_signal))
    }

    /// Run sim and return structured result (same path as digest).
    pub fn simulate(self, height: u64, pulse_signal: f64) -> ResearchResult {
        eval_research_input(&self.encode(height, pulse_signal))
            .expect("v2 encode always parses")
    }

    /// Soft score contribution (0.0..=1.0) used in MeshPulse / proposer.
    pub fn score_weight(self) -> f64 {
        match self {
            Self::BlockPropagation => 0.18,
            Self::SecurityAdversary => 0.18,
            Self::PrivacyLeakage => 0.12,
            Self::ScaleThroughput => 0.18,
            Self::SpamRecovery => 0.10,
            Self::RoutingEfficiency => 0.08,
            Self::MarketBalance => 0.08,
            Self::VerifierQuorum => 0.08,
            Self::QuantumPqc => 0.10,
            Self::QuantumGrover => 0.10,
            Self::QuantumHarvest => 0.10,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResearchCatalogEntry {
    pub id: String,
    pub description: String,
}

pub fn catalog() -> Vec<ResearchCatalogEntry> {
    ResearchScenario::all()
        .iter()
        .map(|s| ResearchCatalogEntry {
            id: s.as_str().into(),
            description: s.description().into(),
        })
        .collect()
}

/// Pick a scenario from MeshPulse-like signals (control-plane only).
pub fn suggest_scenario(gpu_vs_height: f64, avg_latency_ms: f64, verify_ok_rate: f64) -> ResearchScenario {
    if verify_ok_rate < 0.95 {
        return ResearchScenario::SecurityAdversary;
    }
    if avg_latency_ms > 2_000.0 || gpu_vs_height > 2.5 {
        return ResearchScenario::ScaleThroughput;
    }
    if avg_latency_ms > 1_500.0 {
        return ResearchScenario::BlockPropagation;
    }
    if gpu_vs_height < 0.35 {
        return ResearchScenario::RoutingEfficiency;
    }
    if gpu_vs_height > 2.0 {
        return ResearchScenario::MarketBalance;
    }
    // Rotate growth-oriented research when steady.
    if (gpu_vs_height * 1000.0) as u64 % 3 == 0 {
        ResearchScenario::PrivacyLeakage
    } else if (gpu_vs_height * 1000.0) as u64 % 3 == 1 {
        ResearchScenario::BlockPropagation
    } else {
        ResearchScenario::ScaleThroughput
    }
}

/// Aggregate research progress for MeshPulse (verified protocol_eval receipts).
pub fn research_progress(verified_eval_count: u64, scenarios_touched: u32) -> f64 {
    let coverage = (scenarios_touched as f64 / ResearchScenario::all().len() as f64).min(1.0);
    let volume = (verified_eval_count as f64 / 10.0).min(1.0);
    0.6 * coverage + 0.4 * volume
}

/// Blend scenario primary scores into a single research health hint (0..=1).
pub fn blend_primary_scores(scores: &[(ResearchScenario, f64)]) -> f64 {
    if scores.is_empty() {
        return 0.0;
    }
    let mut wsum = 0.0;
    let mut acc = 0.0;
    for (sc, primary) in scores {
        let w = sc.score_weight();
        acc += primary.clamp(0.0, 1.0) * w;
        wsum += w;
    }
    if wsum <= 0.0 {
        0.0
    } else {
        (acc / wsum).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_roundtrip_ids() {
        for s in ResearchScenario::all() {
            assert_eq!(ResearchScenario::parse(s.as_str()), Some(*s));
        }
    }

    #[test]
    fn expected_digest_stable() {
        let a = ResearchScenario::SpamRecovery.expected_digest(42, 0.5);
        let b = ResearchScenario::SpamRecovery.expected_digest(42, 0.5);
        assert_eq!(a, b);
        let c = ResearchScenario::MarketBalance.expected_digest(42, 0.5);
        assert_ne!(a, c);
    }

    #[test]
    fn suggest_prefers_security_on_bad_verify() {
        assert_eq!(
            suggest_scenario(1.0, 10.0, 0.5),
            ResearchScenario::SecurityAdversary
        );
    }

    #[test]
    fn scale_digest_changes_with_height() {
        let a = ResearchScenario::ScaleThroughput.expected_digest(10, 0.5);
        let b = ResearchScenario::ScaleThroughput.expected_digest(500, 0.5);
        assert_ne!(a, b);
    }
}
