//! AI orchestration + MeshPulse (Build/14, Build/15, Build/18).
//!
//! GPU jobs feed verified receipts → GPU market scores → MeshPulse telemetry.
//! Soft envelopes auto-apply in bounds; hard BPS stays governance-locked (Build/21).
//! Shared brain: one MLP all workers advance together (Build/23).
//! Trilemma Guardians: four evolving specialist legs (Build/25).
//! Quantum Research Guardians: three post-quantum readiness legs (Build/26).

mod exam;
mod brain;
mod leg_brain;
mod legs;
mod quantum;
mod quantum_brain;
mod marketplace;
mod ml_train;
mod orchestrator;
mod proposer;
mod protocol_sim;
mod pulse;
mod research;
mod work;

pub use brain::{BrainAdvance, BrainError, BrainMeta, SharedBrain};
pub use leg_brain::{LegAdvance, LegBrainError, LegBrainPack, LegMeta};
pub use legs::{
    build_trilemma_board, encode_leg_job, genesis_leg_weights, is_leg_train, legs_priority,
    parse_leg_job, run_leg_train, LegEpochs, LegId, LegSmart, LegTrainResult, TrilemmaBoard,
};
pub use quantum::{
    build_quantum_board, encode_quantum_job, genesis_quantum_weights, is_quantum_train,
    parse_quantum_job, quantum_priority, run_quantum_train, QuantumBoard, QuantumEpochs,
    QuantumError, QuantumId, QuantumSmart, QuantumTrainResult, QuantumTrainSpec,
};
pub use quantum_brain::{
    QuantumAdvance, QuantumBrainError, QuantumBrainPack, QuantumMeta,
};
pub use marketplace::{
    format_settle_amount, settle_amount_for_weight, MarketError, MarketJob, MarketJobStatus,
    MarketService, Marketplace, SettlementStatus, MAX_PROMPT_BYTES, RATE_LIMIT_MAX,
};
pub use orchestrator::{
    brain_verify_batch_max, effective_train_slots, run_cpu_batch, run_cpu_batch_audited, train_slots_for_vram,
    verify_echo_result, Capability, DurableQueueSnap, JobAssignment, JobQueue, OrchError,
    LastSeal, PendingJob, PendingVerify, QuantumActivityItem, QuantumStoryBeat, ResultOffer,
    VerifiedComplete, WorkerId, WorkerRank,
};
pub use proposer::propose_from_pulse;
pub use protocol_sim::{
    eval_research_input, parse_research_input, simulate as simulate_research, ResearchInput,
    ResearchResult, ResearchScores,
};
pub use pulse::{
    build_mesh_pulse, enrich_orch_pulse, quantum_score_inputs, MarketHealth, MeshPulse,
    QuantumScoreInputs, ResearchScoreTrends, ScenarioScoreSnap,
};
pub use exam::{assign_exam, exam_root, exam_units, ExamAssignment};
pub use research::{
    blend_primary_scores, catalog as research_catalog, suggest_scenario, ResearchCatalogEntry,
    ResearchScenario,
};
pub use ml_train::{
    encode_ml_train_input, encode_ml_train_shared_input, genesis_weights, is_ml_train_shared,
    parse_ml_train_input, parse_ml_train_shared_input, run_ml_train, run_ml_train_shared,
    weights_digest, GENESIS_BRAIN_SEED, MlTrainError, MlTrainResult, MlTrainSharedResult,
    MlTrainSharedSpec, MlTrainSpec, WEIGHTS_BLOB_LEN,
};
pub use work::{
    rematch_board_output, run_benchmark, run_ml_train_job, run_ml_train_shared_job,
    run_protocol_eval,
};
pub use mesh_types::{
    ParamEpoch, ParamProposal, ProposalStatus, ProtocolEnvelopes, BPS_CEIL_CPU, BPS_CEIL_GPU,
    BPS_CEIL_NODE, BPS_FLOOR_CPU, BPS_FLOOR_GPU, BPS_FLOOR_NODE,
};
