//! Deterministic protocol simulations for GPU `protocol_eval` (Build/18 v2).
//!
//! Worker and orchestrator must agree byte-for-byte on digests.

use serde::{Deserialize, Serialize};

/// Measured scores from a research simulation (0..=1 unless noted).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResearchScores {
    /// Overall scenario health (higher = healthier network under the sim).
    pub primary: f64,
    /// Block orphan / tip-split risk.
    pub orphan_risk: f64,
    /// Adversarial share / spam detection rate.
    pub detect_rate: f64,
    /// Gossip metadata linkability (higher = worse privacy).
    pub linkability: f64,
    /// Queue backlog vs capacity.
    pub backlog_ratio: f64,
    /// Simulated p95 latency (milliseconds).
    pub latency_p95_ms: f64,
}

impl Default for ResearchScores {
    fn default() -> Self {
        Self {
            primary: 0.5,
            orphan_risk: 0.5,
            detect_rate: 0.5,
            linkability: 0.5,
            backlog_ratio: 0.5,
            latency_p95_ms: 100.0,
        }
    }
}

impl ResearchScores {
    fn clamp(mut self) -> Self {
        self.primary = self.primary.clamp(0.0, 1.0);
        self.orphan_risk = self.orphan_risk.clamp(0.0, 1.0);
        self.detect_rate = self.detect_rate.clamp(0.0, 1.0);
        self.linkability = self.linkability.clamp(0.0, 1.0);
        self.backlog_ratio = self.backlog_ratio.clamp(0.0, 1.0);
        self.latency_p95_ms = self.latency_p95_ms.clamp(1.0, 60_000.0);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResearchResult {
    pub scenario: String,
    pub height: u64,
    pub pulse_signal: f64,
    pub scores: ResearchScores,
}

impl ResearchResult {
    /// Canonical bytes committed by `run_protocol_eval` digest.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let s = &self.scores;
        format!(
            "mesh-result:v1:scenario={}:h={}:sig={:.6}:p={:.6}:o={:.6}:d={:.6}:l={:.6}:b={:.6}:lat={:.3}",
            self.scenario,
            self.height,
            self.pulse_signal,
            s.primary,
            s.orphan_risk,
            s.detect_rate,
            s.linkability,
            s.backlog_ratio,
            s.latency_p95_ms
        )
        .into_bytes()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ResearchInput {
    pub scenario: &'static str,
    pub height: u64,
    pub pulse_signal: f64,
}

/// Parse `mesh-research:v1|v2:<id>:h=<n>:sig=<f>` (also accepts raw feature blobs as None).
pub fn parse_research_input(input: &[u8]) -> Option<ResearchInput> {
    let text = std::str::from_utf8(input).ok()?.trim();
    let rest = text
        .strip_prefix("mesh-research:v2:")
        .or_else(|| text.strip_prefix("mesh-research:v1:"))?;
    let mut parts = rest.split(':');
    let scenario = parts.next()?.trim();
    if scenario.is_empty() {
        return None;
    }
    let mut height = 0u64;
    let mut pulse_signal = 0.0f64;
    for p in parts {
        if let Some(h) = p.strip_prefix("h=") {
            height = h.parse().unwrap_or(0);
        } else if let Some(s) = p.strip_prefix("sig=") {
            pulse_signal = s.parse().unwrap_or(0.0);
        }
    }
    // Leak scenario string into static via known catalog match; unknown → treat as generic id key.
    let scenario = match scenario {
        "block_propagation" => "block_propagation",
        "security_adversary" => "security_adversary",
        "privacy_leakage" => "privacy_leakage",
        "scale_throughput" => "scale_throughput",
        "spam_recovery" => "spam_recovery",
        "routing_efficiency" => "routing_efficiency",
        "market_balance" => "market_balance",
        "verifier_quorum" => "verifier_quorum",
        "quantum_pqc" => "quantum_pqc",
        "quantum_grover" => "quantum_grover",
        "quantum_harvest" => "quantum_harvest",
        _ => return None,
    };
    Some(ResearchInput {
        scenario,
        height,
        pulse_signal,
    })
}

/// Run the scenario simulation. Pure / deterministic.
pub fn simulate(input: ResearchInput) -> ResearchResult {
    let scores = match input.scenario {
        "block_propagation" => sim_block_propagation(input.height, input.pulse_signal),
        "security_adversary" => sim_security_adversary(input.height, input.pulse_signal),
        "privacy_leakage" => sim_privacy_leakage(input.height, input.pulse_signal),
        "scale_throughput" => sim_scale_throughput(input.height, input.pulse_signal),
        "spam_recovery" => sim_spam_recovery(input.height, input.pulse_signal),
        "routing_efficiency" => sim_routing(input.height, input.pulse_signal),
        "market_balance" => sim_market_balance(input.height, input.pulse_signal),
        "verifier_quorum" => sim_verifier_quorum(input.height, input.pulse_signal),
        "quantum_pqc" => sim_quantum_pqc(input.height, input.pulse_signal),
        "quantum_grover" => sim_quantum_grover(input.height, input.pulse_signal),
        "quantum_harvest" => sim_quantum_harvest(input.height, input.pulse_signal),
        _ => ResearchScores::default(),
    }
    .clamp();
    ResearchResult {
        scenario: input.scenario.into(),
        height: input.height,
        pulse_signal: input.pulse_signal,
        scores,
    }
}

/// Full eval path used by worker/orch: parse → sim → result (None = legacy mix payload).
pub fn eval_research_input(input: &[u8]) -> Option<ResearchResult> {
    let parsed = parse_research_input(input)?;
    Some(simulate(parsed))
}

fn mix01(height: u64, signal: f64, salt: u64) -> f64 {
    let h = height.wrapping_mul(0x9E37_79B9).wrapping_add(salt);
    let bits = (h ^ ((signal * 1_000_000.0) as u64).wrapping_mul(0x85EB_CA6B)) as f64;
    ((bits % 10_000.0) / 10_000.0).clamp(0.0, 1.0)
}

fn sim_block_propagation(height: u64, signal: f64) -> ResearchScores {
    // More peers implied by height; higher signal = more GPU load / congestion.
    let peers = 4.0 + (height as f64).ln().max(0.0) * 2.0;
    let hop_ms = 40.0 + signal * 120.0 + mix01(height, signal, 1) * 30.0;
    let majority_ms = hop_ms * (peers.log2().max(1.0));
    let orphan_risk = (majority_ms / 2_000.0).clamp(0.05, 0.95);
    let primary = (1.0 - orphan_risk * 0.85).clamp(0.05, 0.99);
    ResearchScores {
        primary,
        orphan_risk,
        detect_rate: 0.5 + mix01(height, signal, 2) * 0.1,
        linkability: 0.35 + mix01(height, signal, 3) * 0.1,
        backlog_ratio: (signal / 3.0).clamp(0.05, 0.95),
        latency_p95_ms: majority_ms.clamp(20.0, 8_000.0),
    }
}

fn sim_security_adversary(height: u64, signal: f64) -> ResearchScores {
    let attack_pressure = (0.2 + signal * 0.25 + mix01(height, signal, 4) * 0.2).clamp(0.1, 0.9);
    let detect_rate = (0.92 - attack_pressure * 0.35 + mix01(height, signal, 5) * 0.05).clamp(0.4, 0.99);
    let false_pos = (0.08 + (1.0 - detect_rate) * 0.2).clamp(0.02, 0.4);
    let primary = (detect_rate * (1.0 - false_pos * 0.5)).clamp(0.1, 0.99);
    ResearchScores {
        primary,
        orphan_risk: 0.15 + (1.0 - detect_rate) * 0.4,
        detect_rate,
        linkability: 0.4,
        backlog_ratio: attack_pressure,
        latency_p95_ms: 80.0 + attack_pressure * 400.0,
    }
}

fn sim_privacy_leakage(height: u64, signal: f64) -> ResearchScores {
    // Larger mesh + higher chatter → more metadata linkability.
    let chatter = (height as f64 / 500.0 + signal * 0.3).clamp(0.05, 1.5);
    let linkability = (0.25 + chatter * 0.35 + mix01(height, signal, 6) * 0.15).clamp(0.1, 0.95);
    let primary = (1.0 - linkability * 0.8).clamp(0.1, 0.95);
    ResearchScores {
        primary,
        orphan_risk: 0.2,
        detect_rate: 0.55,
        linkability,
        backlog_ratio: (signal / 2.5).clamp(0.05, 0.9),
        latency_p95_ms: 60.0 + linkability * 200.0,
    }
}

fn sim_scale_throughput(height: u64, signal: f64) -> ResearchScores {
    let load = (height as f64 / 200.0 + signal).max(0.1);
    let capacity = 2.0 + (height as f64).sqrt() / 20.0;
    let backlog_ratio = (load / capacity).clamp(0.05, 0.99);
    let latency_p95_ms = (50.0 + backlog_ratio * 2_500.0 + mix01(height, signal, 7) * 100.0)
        .clamp(20.0, 12_000.0);
    let primary = (1.0 - backlog_ratio * 0.7 - (latency_p95_ms / 15_000.0)).clamp(0.05, 0.98);
    ResearchScores {
        primary,
        orphan_risk: (backlog_ratio * 0.45).clamp(0.05, 0.9),
        detect_rate: 0.6,
        linkability: 0.35 + backlog_ratio * 0.2,
        backlog_ratio,
        latency_p95_ms,
    }
}

fn sim_spam_recovery(height: u64, signal: f64) -> ResearchScores {
    let spam = (0.3 + signal * 0.2 + mix01(height, signal, 8) * 0.25).clamp(0.1, 0.95);
    let detect_rate = (0.88 - spam * 0.25).clamp(0.45, 0.98);
    ResearchScores {
        primary: detect_rate,
        orphan_risk: spam * 0.3,
        detect_rate,
        linkability: 0.4,
        backlog_ratio: spam,
        latency_p95_ms: 70.0 + spam * 300.0,
    }
}

fn sim_routing(height: u64, signal: f64) -> ResearchScores {
    let efficiency = (0.75 - (signal - 1.0).abs() * 0.15 + mix01(height, signal, 9) * 0.1)
        .clamp(0.2, 0.98);
    ResearchScores {
        primary: efficiency,
        orphan_risk: 0.2,
        detect_rate: 0.55,
        linkability: 0.35,
        backlog_ratio: (1.0 - efficiency).clamp(0.05, 0.9),
        latency_p95_ms: 90.0 + (1.0 - efficiency) * 600.0,
    }
}

fn sim_market_balance(_height: u64, signal: f64) -> ResearchScores {
    // High signal = GPU heavy; low = GPU starved — both are imbalance.
    let imbalance = (signal - 1.0).abs().clamp(0.0, 3.0) / 3.0;
    let primary = (1.0 - imbalance * 0.7).clamp(0.15, 0.98);
    ResearchScores {
        primary,
        orphan_risk: imbalance * 0.25,
        detect_rate: 0.55,
        linkability: 0.35,
        backlog_ratio: imbalance,
        latency_p95_ms: 100.0 + imbalance * 400.0,
    }
}

fn sim_quantum_pqc(height: u64, signal: f64) -> ResearchScores {
    // PQC migration progress rises slowly with height; detect_rate models classical fragility.
    let migration = (height as f64 / 10_000.0 + mix01(height, signal, 20) * 0.05).clamp(0.05, 0.85);
    let primary = (0.25 + migration * 0.65 - signal * 0.08).clamp(0.1, 0.95);
    let detect_rate =
        (0.35 + signal * 0.25 + mix01(height, signal, 21) * 0.2).clamp(0.15, 0.92);
    let linkability = (0.38 + mix01(height, signal, 22) * 0.12).clamp(0.2, 0.6);
    ResearchScores {
        primary,
        orphan_risk: detect_rate * 0.35,
        detect_rate,
        linkability,
        backlog_ratio: (signal / 3.5).clamp(0.05, 0.85),
        latency_p95_ms: 80.0 + detect_rate * 350.0,
    }
}

fn sim_quantum_grover(height: u64, signal: f64) -> ResearchScores {
    // √N speedup pressure on search / PoW budget.
    let n_eff = (height as f64 + 1.0).sqrt();
    let grover_pressure =
        (signal * 0.3 + n_eff / 100.0 + mix01(height, signal, 23) * 0.15).clamp(0.1, 0.95);
    let orphan_risk = (grover_pressure * 0.55).clamp(0.05, 0.9);
    let backlog_ratio =
        (grover_pressure * 0.45 + mix01(height, signal, 24) * 0.1).clamp(0.05, 0.9);
    let primary = (1.0 - grover_pressure * 0.75).clamp(0.1, 0.98);
    ResearchScores {
        primary,
        orphan_risk,
        detect_rate: 0.5 + mix01(height, signal, 25) * 0.1,
        linkability: 0.35,
        backlog_ratio,
        latency_p95_ms: 100.0 + grover_pressure * 800.0,
    }
}

fn sim_quantum_harvest(height: u64, signal: f64) -> ResearchScores {
    // Recorded traffic ages with height; linkability drives harvest-now risk.
    let age = (height as f64 / 800.0 + signal * 0.2).clamp(0.05, 2.0);
    let linkability = (0.2 + age * 0.35 + mix01(height, signal, 26) * 0.15).clamp(0.1, 0.95);
    let secrecy_risk =
        (linkability * 0.7 + signal * 0.15 + mix01(height, signal, 27) * 0.1).clamp(0.1, 0.95);
    let primary = (1.0 - secrecy_risk).clamp(0.05, 0.95);
    ResearchScores {
        primary,
        orphan_risk: secrecy_risk * 0.25,
        detect_rate: 0.5,
        linkability,
        backlog_ratio: (signal / 2.5).clamp(0.05, 0.9),
        latency_p95_ms: 60.0 + linkability * 250.0,
    }
}

fn sim_verifier_quorum(height: u64, signal: f64) -> ResearchScores {
    let congestion = (signal * 0.4 + mix01(height, signal, 10) * 0.3).clamp(0.05, 0.95);
    let detect_rate = (0.7 + (1.0 - congestion) * 0.25).clamp(0.4, 0.99);
    ResearchScores {
        primary: detect_rate,
        orphan_risk: congestion * 0.35,
        detect_rate,
        linkability: 0.4,
        backlog_ratio: congestion,
        latency_p95_ms: 120.0 + congestion * 900.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_differs_by_height() {
        let a = simulate(ResearchInput {
            scenario: "scale_throughput",
            height: 10,
            pulse_signal: 0.5,
        });
        let b = simulate(ResearchInput {
            scenario: "scale_throughput",
            height: 500,
            pulse_signal: 0.5,
        });
        assert_ne!(a.canonical_bytes(), b.canonical_bytes());
        assert!(b.scores.backlog_ratio >= a.scores.backlog_ratio - 0.01);
    }

    #[test]
    fn scenarios_differ() {
        let a = simulate(ResearchInput {
            scenario: "block_propagation",
            height: 42,
            pulse_signal: 0.5,
        });
        let b = simulate(ResearchInput {
            scenario: "privacy_leakage",
            height: 42,
            pulse_signal: 0.5,
        });
        assert_ne!(a.canonical_bytes(), b.canonical_bytes());
    }

    #[test]
    fn parse_v2() {
        let raw = b"mesh-research:v2:scale_throughput:h=12:sig=0.250000";
        let p = parse_research_input(raw).expect("parse");
        assert_eq!(p.scenario, "scale_throughput");
        assert_eq!(p.height, 12);
    }
}
