//! Shared deterministic GPU stand-in work (worker + orchestrator verify must match).

use crate::protocol_sim::eval_research_input;

/// Cheap CPU stand-in for GPU benchmark: blake3-mix for `rounds` iterations.
/// Input: first 4 bytes = little-endian rounds (clamped 1..=50_000).
pub fn run_benchmark(input: &[u8]) -> [u8; 32] {
    let rounds = if input.len() >= 4 {
        u32::from_le_bytes(input[..4].try_into().unwrap_or([0; 4])).clamp(1, 50_000)
    } else {
        1_000
    };
    mix_rounds(input, rounds)
}

/// Protocol-eval research: scenario sim → blake3(canonical scores).
/// Non-research payloads fall back to a fixed blake3 mix (legacy marketplace stubs).
pub fn run_protocol_eval(input: &[u8]) -> [u8; 32] {
    if let Some(result) = eval_research_input(input) {
        return *blake3::hash(&result.canonical_bytes()).as_bytes();
    }
    mix_rounds(input, 256)
}

/// Real MNIST training job (verified by identical re-run).
pub fn run_ml_train_job(input: &[u8]) -> Vec<u8> {
    match crate::ml_train::run_ml_train(input) {
        Ok(r) => r.output,
        Err(_) => b"mesh-mltrain-result:v1\nerror".to_vec(),
    }
}

/// Shared-brain MNIST train from network weights.
pub fn run_ml_train_shared_job(weights: &[u8], input: &[u8]) -> Vec<u8> {
    match crate::ml_train::run_ml_train_shared(weights, input) {
        Ok(r) => r.output,
        Err(_) => Vec::new(),
    }
}

/// Legacy AgentAssist wire stub (product removed; kept so old receipts can still verify).
pub(crate) fn run_agent_assist(input: &[u8]) -> Vec<u8> {
    let digest = hex::encode(blake3::hash(input).as_bytes());
    let short = &digest[..12.min(digest.len())];
    format!("Agent removed — protocol research only.\nDigest: {short}\n").into_bytes()
}

fn mix_rounds(seed: &[u8], rounds: u32) -> [u8; 32] {
    let mut state = *blake3::hash(seed).as_bytes();
    for i in 0..rounds {
        let mut buf = Vec::with_capacity(36);
        buf.extend_from_slice(&state);
        buf.extend_from_slice(&i.to_le_bytes());
        state = *blake3::hash(&buf).as_bytes();
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::ResearchScenario;

    #[test]
    fn benchmark_and_eval_are_deterministic() {
        let input = 100u32.to_le_bytes();
        assert_eq!(run_benchmark(&input), run_benchmark(&input));
        let feat = b"research:spam_recovery:v1";
        assert_eq!(run_protocol_eval(feat), run_protocol_eval(feat));
        assert_ne!(run_protocol_eval(feat), run_benchmark(&input));
    }

    #[test]
    fn protocol_eval_research_v2_stable_and_scenario_specific() {
        let a = ResearchScenario::BlockPropagation.encode(42, 0.5);
        let b = ResearchScenario::PrivacyLeakage.encode(42, 0.5);
        assert_eq!(run_protocol_eval(&a), run_protocol_eval(&a));
        assert_ne!(run_protocol_eval(&a), run_protocol_eval(&b));
    }
}
