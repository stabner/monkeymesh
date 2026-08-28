//! MeshPulse — read-only network health blob from receipts + chain metrics.

use std::collections::BTreeMap;

use mesh_types::AiJobReceipt;
use serde::{Deserialize, Serialize};

use crate::research::{research_progress, ResearchScenario};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ResearchScoreTrends {
    /// Mean primary health across recent protocol_eval receipts (0..=1).
    pub mean_primary: f64,
    pub mean_orphan_risk: f64,
    pub mean_detect_rate: f64,
    pub mean_linkability: f64,
    pub mean_backlog_ratio: f64,
    pub mean_latency_p95_ms: f64,
    /// Distinct research_scenario ids seen on receipts.
    pub scenarios_touched: u32,
    /// Per-scenario means (ProtocolEval receipts only).
    #[serde(default)]
    pub by_scenario: BTreeMap<String, ScenarioScoreSnap>,
}

/// Aggregated scores for one research scenario id.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ScenarioScoreSnap {
    pub scenario: String,
    pub receipts: u32,
    pub mean_primary: f64,
    pub mean_orphan_risk: f64,
    pub mean_detect_rate: f64,
    pub mean_linkability: f64,
    pub mean_backlog_ratio: f64,
    pub last_primary: f64,
}

/// Inputs for Absolute Quantum Board — prefer real quantum_* protocol_eval receipts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuantumScoreInputs {
    pub pqc_primary: f64,
    pub pqc_detect: f64,
    pub grover_primary: f64,
    pub grover_orphan: f64,
    pub grover_backlog: f64,
    pub harvest_primary: f64,
    pub harvest_linkability: f64,
    /// True when at least one quantum_* ProtocolEval receipt contributed.
    pub from_receipts: bool,
    pub protocol: BTreeMap<String, ScenarioScoreSnap>,
    pub honesty: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketHealth {
    pub pending_gpu_weight: u64,
    pub pending_node_weight: u64,
    pub gpu_receipts: usize,
    pub avg_latency_ms: f64,
    pub echo_ok_rate: f64,
    /// Verified protocol_eval / research receipts observed on-chain.
    pub research_eval_receipts: usize,
    /// 0..=1 coverage×volume hint for Phase-5 research framework.
    pub research_progress: f64,
    #[serde(default)]
    pub research_scores: ResearchScoreTrends,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeshPulse {
    pub version: u32,
    pub height: u64,
    pub tip: String,
    pub markets: MarketHealth,
    /// Soft imbalance hint: gpu_weight / max(cpu_proxy,1) — informational only.
    pub gpu_vs_height_signal: f64,
    pub note: String,
    /// Shared network brain epoch (seed AI board).
    #[serde(default)]
    pub brain_epoch: u64,
    #[serde(default)]
    pub brain_digest_hex: String,
    #[serde(default)]
    pub brain_acc: f64,
    #[serde(default)]
    pub brain_advances: u64,
    /// Absolute Trilemma Board (Build/25) — integers 0–100.
    #[serde(default)]
    pub trilemma: Option<crate::legs::TrilemmaBoard>,
    /// Absolute Quantum Board (Build/26) — integers 0–100.
    #[serde(default)]
    pub quantum: Option<crate::quantum::QuantumBoard>,
}

fn scenario_snaps(evals: &[&AiJobReceipt]) -> BTreeMap<String, ScenarioScoreSnap> {
    let mut buckets: BTreeMap<String, Vec<&AiJobReceipt>> = BTreeMap::new();
    for r in evals {
        if r.research_scenario.is_empty() {
            continue;
        }
        buckets
            .entry(r.research_scenario.clone())
            .or_default()
            .push(*r);
    }
    buckets
        .into_iter()
        .map(|(scenario, rows)| {
            let n = rows.len() as f64;
            let last = rows.last().map(|r| r.score_primary).unwrap_or(0.0);
            (
                scenario.clone(),
                ScenarioScoreSnap {
                    scenario,
                    receipts: rows.len() as u32,
                    mean_primary: rows.iter().map(|r| r.score_primary).sum::<f64>() / n,
                    mean_orphan_risk: rows.iter().map(|r| r.score_orphan_risk).sum::<f64>() / n,
                    mean_detect_rate: rows.iter().map(|r| r.score_detect_rate).sum::<f64>() / n,
                    mean_linkability: rows.iter().map(|r| r.score_linkability).sum::<f64>() / n,
                    mean_backlog_ratio: rows.iter().map(|r| r.score_backlog_ratio).sum::<f64>() / n,
                    last_primary: last,
                },
            )
        })
        .collect()
}

fn trends_from_receipts(receipts: &[AiJobReceipt]) -> ResearchScoreTrends {
    let evals: Vec<_> = receipts
        .iter()
        .filter(|r| matches!(r.job_kind, mesh_types::AiJobKind::ProtocolEval))
        .filter(|r| !r.research_scenario.is_empty() || r.score_primary > 0.0)
        .collect();
    let mut scenarios = std::collections::BTreeSet::new();
    for r in &evals {
        if !r.research_scenario.is_empty() {
            scenarios.insert(r.research_scenario.as_str());
        }
    }
    let by_scenario = scenario_snaps(&evals);
    let n = evals.len() as f64;
    if n <= 0.0 {
        return ResearchScoreTrends {
            scenarios_touched: scenarios.len() as u32,
            by_scenario,
            ..Default::default()
        };
    }
    ResearchScoreTrends {
        mean_primary: evals.iter().map(|r| r.score_primary).sum::<f64>() / n,
        mean_orphan_risk: evals.iter().map(|r| r.score_orphan_risk).sum::<f64>() / n,
        mean_detect_rate: evals.iter().map(|r| r.score_detect_rate).sum::<f64>() / n,
        mean_linkability: evals.iter().map(|r| r.score_linkability).sum::<f64>() / n,
        mean_backlog_ratio: evals.iter().map(|r| r.score_backlog_ratio).sum::<f64>() / n,
        mean_latency_p95_ms: evals.iter().map(|r| r.score_latency_p95_ms).sum::<f64>() / n,
        scenarios_touched: scenarios.len() as u32,
        by_scenario,
    }
}

/// Prefer quantum_* ProtocolEval receipts; fall back to live deterministic sims.
pub fn quantum_score_inputs(
    receipts: &[AiJobReceipt],
    height: u64,
    pulse_signal: f64,
) -> QuantumScoreInputs {
    let trends = trends_from_receipts(receipts);
    let mut protocol = BTreeMap::new();
    for id in ["quantum_pqc", "quantum_grover", "quantum_harvest"] {
        if let Some(s) = trends.by_scenario.get(id) {
            protocol.insert(id.to_string(), s.clone());
        }
    }
    let from_receipts = !protocol.is_empty();

    let pqc_sim = ResearchScenario::QuantumPqc.simulate(height, pulse_signal);
    let grover_sim = ResearchScenario::QuantumGrover.simulate(height, pulse_signal);
    let harvest_sim = ResearchScenario::QuantumHarvest.simulate(height, pulse_signal);

    let pqc = protocol.get("quantum_pqc");
    let grover = protocol.get("quantum_grover");
    let harvest = protocol.get("quantum_harvest");

    let honesty = if from_receipts {
        "Needles from verified quantum protocol_eval receipts (plus guardian smarts)"
            .into()
    } else {
        "Provisional — live quantum sims until quantum_* protocol_eval receipts land".into()
    };

    QuantumScoreInputs {
        pqc_primary: pqc.map(|s| s.mean_primary).unwrap_or(pqc_sim.scores.primary),
        pqc_detect: pqc
            .map(|s| s.mean_detect_rate)
            .unwrap_or(pqc_sim.scores.detect_rate),
        grover_primary: grover
            .map(|s| s.mean_primary)
            .unwrap_or(grover_sim.scores.primary),
        grover_orphan: grover
            .map(|s| s.mean_orphan_risk)
            .unwrap_or(grover_sim.scores.orphan_risk),
        grover_backlog: grover
            .map(|s| s.mean_backlog_ratio)
            .unwrap_or(grover_sim.scores.backlog_ratio),
        harvest_primary: harvest
            .map(|s| s.mean_primary)
            .unwrap_or(harvest_sim.scores.primary),
        harvest_linkability: harvest
            .map(|s| s.mean_linkability)
            .unwrap_or(harvest_sim.scores.linkability),
        from_receipts,
        protocol,
        honesty,
    }
}

/// Build a MeshPulse feature blob. Never activates consensus params.
pub fn build_mesh_pulse(
    height: u64,
    tip: String,
    pending_gpu_weight: u64,
    pending_node_weight: u64,
    receipts: &[AiJobReceipt],
) -> MeshPulse {
    let n = receipts.len();
    let avg_latency = if n == 0 {
        0.0
    } else {
        receipts.iter().map(|r| r.latency_ms as f64).sum::<f64>() / n as f64
    };
    let echo_ok_rate = if n == 0 { 1.0 } else { 1.0 };
    let research_eval_receipts = receipts
        .iter()
        .filter(|r| matches!(r.job_kind, mesh_types::AiJobKind::ProtocolEval))
        .count();
    let trends = trends_from_receipts(receipts);
    let research_progress = research_progress(
        research_eval_receipts as u64,
        trends.scenarios_touched,
    );
    MeshPulse {
        version: 3,
        height,
        tip,
        markets: MarketHealth {
            pending_gpu_weight,
            pending_node_weight,
            gpu_receipts: n,
            avg_latency_ms: avg_latency,
            echo_ok_rate,
            research_eval_receipts,
            research_progress,
            research_scores: trends,
        },
        gpu_vs_height_signal: pending_gpu_weight as f64 / (height.max(1) as f64),
        note: "read-only MeshPulse — research scores feed soft param epochs; BPS human-only"
            .into(),
        brain_epoch: 0,
        brain_digest_hex: String::new(),
        brain_acc: 0.0,
        brain_advances: 0,
        trilemma: None,
        quantum: None,
    }
}

/// Orchestrator-local MeshPulse enrichment (queue depth + live verify rate).
pub fn enrich_orch_pulse(
    mut pulse: MeshPulse,
    queue_depth: usize,
    verify_ok_rate: f64,
    research_scenarios_touched: u32,
    protocol_eval_ok: u64,
) -> MeshPulse {
    pulse.markets.echo_ok_rate = verify_ok_rate;
    pulse.markets.research_eval_receipts = protocol_eval_ok as usize;
    if research_scenarios_touched > pulse.markets.research_scores.scenarios_touched {
        pulse.markets.research_scores.scenarios_touched = research_scenarios_touched;
    }
    pulse.markets.research_progress =
        research_progress(protocol_eval_ok, pulse.markets.research_scores.scenarios_touched);
    if queue_depth > 0 {
        pulse.note = format!(
            "{} | orch queue_depth={queue_depth} research_progress={:.2}",
            pulse.note, pulse.markets.research_progress
        );
    }
    pulse
}
