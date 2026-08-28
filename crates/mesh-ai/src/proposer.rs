//! Adaptive proposer — soft envelopes from MeshPulse (Build/15, Build/21).
//! Soft params auto-apply on-chain; BPS suggestions stay propose-only.

use mesh_types::{
    ParamProposal, ProposalStatus, ProtocolEnvelopes, BPS_CEIL_CPU, BPS_CEIL_GPU, BPS_CEIL_NODE,
    BPS_FLOOR_CPU, BPS_FLOOR_GPU, BPS_FLOOR_NODE,
};

use crate::pulse::MeshPulse;

/// Build a proposal from MeshPulse. Does **not** activate anything.
pub fn propose_from_pulse(pulse: &MeshPulse, next_id: u64) -> ParamProposal {
    let mut env = ProtocolEnvelopes::default();
    let mut rationale: Vec<String> = Vec::new();
    let scores = &pulse.markets.research_scores;

    if pulse.gpu_vs_height_signal < 0.3 {
        env.soft_adapt_signal_threshold = 0.8;
        env.soft_benchmark_rounds = 5_000;
        rationale.push("low GPU signal → propose more GPU workload".into());
    } else if pulse.gpu_vs_height_signal > 2.0 {
        env.soft_adapt_signal_threshold = 0.2;
        env.soft_benchmark_rounds = 500;
        rationale.push("high GPU backlog → propose cooler soft-adapt".into());
    }

    if pulse.markets.avg_latency_ms > 2_000.0 || scores.mean_latency_p95_ms > 1_500.0 {
        env.min_verifier_weight = 2;
        rationale.push("high latency (jobs or scale sim) → raise verifier weight floor".into());
    }

    if scores.mean_detect_rate > 0.0 && scores.mean_detect_rate < 0.7 {
        env.min_verifier_weight = env.min_verifier_weight.max(3);
        rationale.push("security sim weak detect_rate → raise min verifier weight".into());
    }

    // Build/25: Trilemma security needle below soft floor → harden defenses.
    if let Some(board) = &pulse.trilemma {
        if board.sec < env.leg_harden_sec_floor {
            env.min_verifier_weight = env.min_verifier_weight.max(4);
            env.soft_adapt_signal_threshold = env.soft_adapt_signal_threshold.max(0.7);
            env.leg_train_enable = 1;
            env.leg_parallel = env.leg_parallel.max(3);
            rationale.push(format!(
                "trilemma sec={} below floor {} → harden verifiers + feed guardian legs",
                board.sec, env.leg_harden_sec_floor
            ));
        }
        if board.balance < 55 {
            env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(5_000);
            rationale.push(format!(
                "trilemma balance={} skewed (weakest={}) → more research budget",
                board.balance, board.weakest
            ));
        }
    }

    if scores.mean_orphan_risk > 0.45 {
        env.soft_adapt_signal_threshold = env.soft_adapt_signal_threshold.max(0.6);
        env.suggested_cpu_diff_bias = -1;
        rationale.push("block_propagation orphan risk high → cooler adapt + soft CPU ease".into());
    }

    if scores.mean_linkability > 0.55 {
        env.idle_stipend_bps_cap = env.idle_stipend_bps_cap.min(750);
        rationale.push("privacy linkability elevated → tighten idle stipend hint".into());
    }

    if scores.mean_backlog_ratio > 0.55 {
        env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(4_000);
        rationale.push("scale backlog high → more GPU research budget".into());
    }

    if pulse.markets.research_progress < 0.2 && pulse.height > 5 {
        env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(3_000);
        rationale.push("low research progress → propose more GPU research budget".into());
    } else if pulse.markets.research_progress > 0.7 && scores.mean_primary > 0.65 {
        env.suggested_cpu_diff_bias = 0;
        rationale.push("healthy research coverage + scores — keep soft envelopes steady".into());
    }

    if pulse.markets.echo_ok_rate < 0.9 {
        env.min_verifier_weight = env.min_verifier_weight.max(2);
        rationale.push("verify failures → raise min verifier weight (spam recovery)".into());
    }

    if pulse.markets.pending_node_weight == 0 && pulse.height > 10 {
        rationale.push("node score empty — keep node vault filling via relay credits".into());
    }

    if pulse.markets.gpu_receipts == 0 && pulse.height > 5 {
        env.idle_stipend_bps_cap = 500;
        rationale.push("no GPU receipts yet — keep idle stipend tight".into());
    }

    if scores.mean_primary > 0.0 && scores.mean_primary < 0.45 {
        env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(5_000);
        rationale.push("low mean research primary → intensify protocol sims".into());
    }

    // Quantum board (Build/26) when MeshPulse carries it.
    if let Some(q) = &pulse.quantum {
        if q.readiness < 45 {
            env.quantum_train_enable = 1;
            env.quantum_parallel = env.quantum_parallel.max(2);
            env.soft_benchmark_rounds = env.soft_benchmark_rounds.max(4_000);
            rationale.push(format!(
                "quantum readiness {}/100 (weakest={}) → more quantum research intensity",
                q.readiness, q.weakest
            ));
        }
        // Build/30: grover needle drives bounded retarget posture (chain gate still required).
        if q.grover < 50 {
            env.min_difficulty_floor = env.min_difficulty_floor.max(8);
            env.retarget_step = 2;
            env.suggested_cpu_diff_bias = env.suggested_cpu_diff_bias.max(0);
            rationale.push(format!(
                "quantum grover needle {}/100 → harden retarget floor/step (gated on-chain)",
                q.grover
            ));
        } else if q.grover >= 70 && scores.mean_orphan_risk < 0.25 {
            env.retarget_interval = 15;
            rationale.push(format!(
                "quantum grover needle {}/100 + stable mesh → tighten retarget interval (gated)",
                q.grover
            ));
        }
    }

    // Absolute-best soft performance — ease soft mining hint when mesh is healthy.
    let healthy = scores.mean_orphan_risk < 0.25
        && scores.mean_backlog_ratio < 0.45
        && pulse.markets.echo_ok_rate >= 0.95
        && (scores.mean_primary <= 0.0 || scores.mean_primary >= 0.5);
    if healthy && pulse.height > 50 {
        env.brain_prefer_v2 = 1;
        env.idle_stipend_bps_cap = env.idle_stipend_bps_cap.max(1_000);
        if scores.mean_orphan_risk < 0.15 && pulse.markets.research_progress > 0.45 {
            env.suggested_cpu_diff_bias = -2;
            rationale.push(
                "peak soft performance — max soft CPU ease (consensus rules unchanged)".into(),
            );
        } else {
            env.suggested_cpu_diff_bias = env.suggested_cpu_diff_bias.min(-1);
            rationale.push(
                "healthy mesh → soft CPU ease + full research stipend for best throughput".into(),
            );
        }
    }

    // Scale verify: when research backlog is high, audit light jobs 1-in-4 (brains still full-verify).
    if scores.mean_backlog_ratio > 0.55 || pulse.markets.research_eval_receipts > 200 {
        env.brain_audit_every = env.brain_audit_every.max(4);
        rationale.push(
            "heavy research load → audit light jobs 1-in-K (shared brain always full-verify)".into(),
        );
    }

    if rationale.is_empty() {
        rationale.push("steady MeshPulse — propose default envelopes".into());
    }

    let (cpu, gpu, node) = (4_000u16, 4_000u16, 2_000u16);

    ParamProposal {
        id: format!("prop-{next_id}"),
        created_at_height: pulse.height,
        rationale: rationale.join("; "),
        envelopes: env.clamp(),
        status: ProposalStatus::Pending,
        suggested_cpu_bps: cpu.clamp(BPS_FLOOR_CPU, BPS_CEIL_CPU),
        suggested_gpu_bps: gpu.clamp(BPS_FLOOR_GPU, BPS_CEIL_GPU),
        suggested_node_bps: node.clamp(BPS_FLOOR_NODE, BPS_CEIL_NODE),
        votes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pulse::{MarketHealth, MeshPulse, ResearchScoreTrends};

    #[test]
    fn propose_pending_within_floors() {
        let pulse = MeshPulse {
            version: 3,
            height: 100,
            tip: "x".into(),
            markets: MarketHealth {
                pending_gpu_weight: 0,
                pending_node_weight: 0,
                gpu_receipts: 0,
                avg_latency_ms: 50.0,
                echo_ok_rate: 1.0,
                research_eval_receipts: 0,
                research_progress: 0.0,
                research_scores: ResearchScoreTrends::default(),
            },
            gpu_vs_height_signal: 0.0,
            note: String::new(),
            brain_epoch: 0,
            brain_digest_hex: String::new(),
            brain_acc: 0.0,
            brain_advances: 0,
            trilemma: None,
            quantum: None,
        };
        let p = propose_from_pulse(&pulse, 1);
        assert!(p.suggested_cpu_bps >= BPS_FLOOR_CPU);
        assert!(matches!(p.status, ProposalStatus::Pending));
    }

    #[test]
    fn weak_security_raises_verifier_weight() {
        let mut pulse = MeshPulse {
            version: 3,
            height: 50,
            tip: "x".into(),
            markets: MarketHealth {
                pending_gpu_weight: 10,
                pending_node_weight: 1,
                gpu_receipts: 3,
                avg_latency_ms: 40.0,
                echo_ok_rate: 1.0,
                research_eval_receipts: 3,
                research_progress: 0.4,
                research_scores: ResearchScoreTrends {
                    mean_primary: 0.5,
                    mean_detect_rate: 0.55,
                    ..Default::default()
                },
            },
            gpu_vs_height_signal: 1.0,
            note: String::new(),
            brain_epoch: 0,
            brain_digest_hex: String::new(),
            brain_acc: 0.0,
            brain_advances: 0,
            trilemma: None,
            quantum: None,
        };
        let p = propose_from_pulse(&pulse, 2);
        assert!(p.envelopes.min_verifier_weight >= 3);
        assert!(p.rationale.contains("detect_rate"));
        let _ = &mut pulse;
    }
}
