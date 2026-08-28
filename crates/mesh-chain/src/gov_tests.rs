use crate::Chain;
use mesh_types::{Address, AiJobKind, AiJobReceipt, Hash, ProposalStatus, ProtocolEnvelopes};

#[test]
fn soft_envelopes_auto_apply_after_protocol_evals() {
    let dir = tempfile_dir("autoadapt");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let before = chain.active_envelopes();
    let worker = Address::from_pubkey_bytes(b"auto-adapt-gpu");
    for i in 0..3 {
        let receipt = AiJobReceipt {
            job_id: format!("eval-{i}"),
            worker,
            input_commitment: Hash::digest(format!("in-{i}").as_bytes()),
            output_hash: Hash::digest(format!("out-{i}").as_bytes()),
            latency_ms: 8,
            weight: 25,
            verified_at: i as u64 + 1,
            job_kind: AiJobKind::ProtocolEval,
            research_scenario: String::new(),
            score_primary: 0.0,
            score_orphan_risk: 0.0,
            score_detect_rate: 0.0,
            score_linkability: 0.0,
            score_backlog_ratio: 0.0,
            score_latency_p95_ms: 0.0,
        };
        chain.record_ai_receipt(receipt).expect("receipt");
    }
    assert!(
        !chain.last_auto_adapt_proposal_id().is_empty(),
        "expected auto-adapt proposal id"
    );
    assert_eq!(chain.last_auto_adapt_eval_count(), 3);
    assert_eq!(chain.param_epoch(), 1);
    let after = chain.active_envelopes();
    // Height 0 + GPU weight → high signal → cooler soft-adapt vs defaults.
    assert_ne!(after.soft_adapt_signal_threshold, before.soft_adapt_signal_threshold);
    assert!(chain
        .proposals()
        .iter()
        .any(|p| p.rationale.contains("auto-adapt")
            && matches!(p.status, ProposalStatus::Activated)));
}

#[test]
fn one_vote_per_node_id() {
    let dir = tempfile_dir("vote1");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let p = chain.generate_adaptive_proposal().expect("propose");
    let out = chain
        .cast_proposal_vote(&p.id, "node-A", mesh_types::VoteChoice::Yes)
        .expect("first vote");
    assert!(matches!(out.status, ProposalStatus::Activated));
    let err = chain
        .cast_proposal_vote(&p.id, "node-A", mesh_types::VoteChoice::No)
        .expect_err("duplicate");
    let msg = err.to_string();
    assert!(
        msg.contains("already voted"),
        "unexpected err: {msg}"
    );
}

#[test]
fn two_nodes_majority_no_rejects() {
    let dir = tempfile_dir("vote2");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let p = chain.generate_adaptive_proposal().expect("propose");
    // Need a pending proposal where no wins: first cast no (activates reject with majority 1).
    chain
        .cast_proposal_vote(&p.id, "node-A", mesh_types::VoteChoice::No)
        .expect("no");
    assert!(chain
        .proposals()
        .iter()
        .any(|x| x.id == p.id && matches!(x.status, ProposalStatus::Rejected)));
}



#[test]
fn reject_proposal_leaves_default_envelopes() {
    let dir = tempfile_dir("rej");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let before = chain.active_envelopes();
    let p = chain.generate_adaptive_proposal().expect("propose");
    chain.reject_proposal(&p.id).expect("reject");
    assert_eq!(chain.active_envelopes(), before);
    assert!(chain
        .proposals()
        .iter()
        .any(|x| x.id == p.id && matches!(x.status, ProposalStatus::Rejected)));
}

#[test]
fn min_verifier_weight_filters_gpu_credit() {
    let dir = tempfile_dir("minw");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let mut env = ProtocolEnvelopes::default();
    env.min_verifier_weight = 10;
    chain.set_active_envelopes(env).expect("env");

    let worker = Address::from_pubkey_bytes(b"gpu-test-worker");
    chain.credit_gpu_score(worker, 5).expect("credit low");
    assert!(
        chain.store().gpu_scores().is_empty(),
        "weight below min must not credit"
    );
    chain.credit_gpu_score(worker, 10).expect("credit ok");
    assert_eq!(
        chain.store().gpu_scores().get(&worker.to_hex()).copied(),
        Some(10)
    );
}

#[test]
fn ai_receipt_credits_gpu_market() {
    let dir = tempfile_dir("rcpt");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let worker = Address::from_pubkey_bytes(b"receipt-worker");
    let receipt = AiJobReceipt {
        job_id: mesh_types::exam_job_id(1, &worker),
        worker,
        input_commitment: Hash::digest(b"in"),
        output_hash: Hash::digest(b"out"),
        latency_ms: 12,
        weight: 3,
        verified_at: 1,
        job_kind: AiJobKind::ProtocolEval,
        research_scenario: String::new(),
        score_primary: 0.0,
        score_orphan_risk: 0.0,
        score_detect_rate: 0.0,
        score_linkability: 0.0,
        score_backlog_ratio: 0.0,
        score_latency_p95_ms: 0.0,
    };
    chain.record_ai_receipt(receipt).expect("receipt");
    assert_eq!(
        chain.store().gpu_scores().get(&worker.to_hex()).copied(),
        Some(crate::MAX_CREDIT_PER_EVENT)
    );
    assert_eq!(chain.store().ai_receipts().len(), 1);
}

#[test]
fn market_coinbase_pays_gpu_scorer_not_cpu() {
    let dir = tempfile_dir("cb");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let gpu = Address::from_pubkey_bytes(b"gpu-winner");
    let cpu = Address::from_pubkey_bytes(b"cpu-miner");
    chain.credit_gpu_score(gpu, 100).expect("score");
    let block = chain.mining_template(cpu);
    let cb = &block.txs[0];
    assert_eq!(cb.outputs[0].address, cpu);
    // Clean 45/45/10: GPU work pays the finder, not the exam ledger.
    assert_eq!(cb.outputs[1].address, cpu);
    assert_eq!(cb.outputs[1].amount, crate::gpu_market_reward(block.header.height));
}

#[test]
fn soft_diff_hint_applies_bias_without_changing_consensus() {
    let dir = tempfile_dir("bias");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let consensus = chain.next_difficulty();
    assert_eq!(chain.soft_mining_diff_hint(), consensus);

    let mut env = ProtocolEnvelopes::default();
    env.suggested_cpu_diff_bias = 2;
    chain.set_active_envelopes(env).expect("env");
    assert_eq!(chain.next_difficulty(), consensus);
    assert_eq!(chain.soft_mining_diff_hint(), consensus.saturating_add(2));
}

fn push_protocol_eval(chain: &mut Chain, worker: Address, i: u64, scenario: &str, primary: f64) {
    let receipt = AiJobReceipt {
        job_id: format!("eval-{scenario}-{i}"),
        worker,
        input_commitment: Hash::digest(format!("in-{i}").as_bytes()),
        output_hash: Hash::digest(format!("out-{i}").as_bytes()),
        latency_ms: 8,
        weight: 25,
        verified_at: i + 1,
        job_kind: AiJobKind::ProtocolEval,
        research_scenario: scenario.into(),
        score_primary: primary,
        score_orphan_risk: 0.1,
        score_detect_rate: 0.8,
        score_linkability: 0.1,
        score_backlog_ratio: 0.1,
        score_latency_p95_ms: 40.0,
    };
    chain.record_ai_receipt(receipt).expect("receipt");
}

#[test]
fn non_grover_evals_cannot_move_retarget() {
    let dir = tempfile_dir("noretarget");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let before = chain.active_envelopes();
    let worker = Address::from_pubkey_bytes(b"noretarget-gpu");
    // Soft adapt will fire after 3 evals, but retarget must stay frozen (no grover certs).
    for i in 0..3 {
        push_protocol_eval(&mut chain, worker, i, "security_adversary", 0.2);
    }
    assert!(!chain.last_auto_adapt_proposal_id().is_empty());
    let after = chain.active_envelopes();
    assert_eq!(after.retarget_interval, before.retarget_interval);
    assert_eq!(after.retarget_step, before.retarget_step);
    assert_eq!(after.min_difficulty_floor, before.min_difficulty_floor);
    assert_eq!(chain.grover_eval_count(), 0);
}

#[test]
fn quantum_grover_certs_do_not_move_consensus_retarget() {
    let dir = tempfile_dir("grovergate");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let before = chain.active_envelopes();
    let worker = Address::from_pubkey_bytes(b"grover-gpu");
    for i in 0..5 {
        push_protocol_eval(&mut chain, worker, i, "quantum_grover", 0.2);
    }
    assert_eq!(chain.grover_eval_count(), 5);
    let after = chain.active_envelopes();
    assert_eq!(after.min_difficulty_floor, before.min_difficulty_floor);
    assert_eq!(after.retarget_step, before.retarget_step);
    assert_eq!(after.retarget_interval, before.retarget_interval);
    assert!(
        chain
            .proposals()
            .iter()
            .any(|p| p.rationale.contains("retarget frozen")),
        "expected retarget frozen tag, got {:?}",
        chain.proposals().last().map(|p| &p.rationale)
    );
}

#[test]
fn retarget_envelope_floor_clamps_next_difficulty() {
    let dir = tempfile_dir("floorclamp");
    let path = dir.join("chain.bin");
    let mut chain = Chain::open_or_genesis(&path).expect("genesis");
    let mut env = ProtocolEnvelopes::default();
    env.min_difficulty_floor = 12;
    chain.set_active_envelopes(env).expect("env");
    assert!(chain.next_difficulty() >= 12);
    assert_eq!(chain.retarget_params().min_floor, 12);
}

#[test]
fn imported_exam_receipt_does_not_credit_gpu() {
    let dir = tempfile_dir("examcheat");
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    let worker = Address::from_pubkey_bytes(b"cheat-exam-gpu");
    let receipt = AiJobReceipt {
        job_id: mesh_types::exam_job_id(1, &worker),
        worker,
        input_commitment: Hash::digest(b"in"),
        output_hash: Hash::digest(b"out"),
        latency_ms: 1,
        weight: 99_999,
        verified_at: 1,
        job_kind: AiJobKind::ProtocolEval,
        research_scenario: String::new(),
        score_primary: 0.0,
        score_orphan_risk: 0.0,
        score_detect_rate: 0.0,
        score_linkability: 0.0,
        score_backlog_ratio: 0.0,
        score_latency_p95_ms: 0.0,
    };
    assert!(chain.record_ai_receipt_imported(receipt).expect("import"));
    assert_eq!(
        chain
            .store()
            .gpu_scores()
            .get(&worker.to_hex())
            .copied()
            .unwrap_or(0),
        0,
        "gossip /aireceipt must not mint exam GPU units"
    );
}

#[test]
fn verified_exam_receipt_credits_gpu() {
    let dir = tempfile_dir("examok");
    let mut chain = Chain::open_or_genesis(dir.join("chain.bin")).expect("genesis");
    let worker = Address::from_pubkey_bytes(b"honest-exam-gpu");
    let receipt = AiJobReceipt {
        job_id: mesh_types::exam_job_id(1, &worker),
        worker,
        input_commitment: Hash::digest(b"in"),
        output_hash: Hash::digest(b"out"),
        latency_ms: 1,
        weight: mesh_types::EXAM_LANE_UNITS,
        verified_at: 1,
        job_kind: AiJobKind::ProtocolEval,
        research_scenario: String::new(),
        score_primary: 0.0,
        score_orphan_risk: 0.0,
        score_detect_rate: 0.0,
        score_linkability: 0.0,
        score_backlog_ratio: 0.0,
        score_latency_p95_ms: 0.0,
    };
    assert!(chain.record_ai_receipt(receipt).expect("verify"));
    let credited = chain
        .store()
        .gpu_scores()
        .get(&worker.to_hex())
        .copied()
        .unwrap_or(0);
    assert!(
        credited > 0 && credited <= mesh_types::EXAM_LANE_UNITS,
        "verified exam must credit GPU (capped per event), got {credited}"
    );
}

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mm_gov_{}_{}_{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
