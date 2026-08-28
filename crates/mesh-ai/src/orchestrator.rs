//! Local job queue: advertise → assign echo/benchmark → verify → receipt.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use mesh_types::{Address, AiJobKind, AiJobReceipt, Hash};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::brain::{BrainAdvance, SharedBrain};
use crate::leg_brain::{LegAdvance, LegBrainPack};
use crate::legs::{is_leg_train, LegId};
use crate::quantum::{is_quantum_train, QuantumId};
use crate::quantum_brain::{QuantumAdvance, QuantumBrainPack};
use crate::ml_train::{is_ml_train_shared, run_ml_train_shared};
use crate::research::ResearchScenario;
use crate::work::{run_agent_assist, run_benchmark, run_ml_train_job, run_protocol_eval};
use mesh_ai_v2::{is_ml_train_shared_v2, run_job as run_job_v2, BrainAdvanceV2, SharedBrainV2};

#[derive(Debug, Error)]
pub enum OrchError {
    #[error("unknown worker")]
    UnknownWorker,
    #[error("no pending job")]
    NoJob,
    #[error("job mismatch")]
    JobMismatch,
    #[error("echo verification failed")]
    EchoFailed,
    #[error("benchmark verification failed")]
    BenchmarkFailed,
    #[error("protocol_eval verification failed")]
    ProtocolEvalFailed,
    #[error("agent_assist verification failed")]
    AgentAssistFailed,
    #[error("ml_train verification failed")]
    MlTrainFailed,
    #[error("stale shared brain epoch")]
    StaleBrain,
    #[error("invalid address")]
    BadAddress,
}

/// Heavy verify work extracted under the AI lock; runs CPU off-mutex (Build/27 N9).
pub enum PendingVerify {
    Light {
        job: PendingJob,
        output: Vec<u8>,
    },
    Shared {
        job: PendingJob,
        output: Vec<u8>,
        weights: Vec<u8>,
        epoch: u64,
    },
    SharedV2 {
        job: PendingJob,
        output: Vec<u8>,
        weights: Vec<u8>,
        epoch: u64,
    },
    Leg {
        job: PendingJob,
        output: Vec<u8>,
        weights: Vec<u8>,
        epoch: u64,
        leg: LegId,
    },
    Quantum {
        job: PendingJob,
        output: Vec<u8>,
        weights: Vec<u8>,
        epoch: u64,
        leg: QuantumId,
    },
}

/// CPU-verified result ready to apply under the AI lock.
pub enum VerifiedComplete {
    Light {
        job: PendingJob,
        output_hash: Hash,
    },
    Shared {
        job: PendingJob,
        output_hash: Hash,
        advance: BrainAdvance,
    },
    SharedV2 {
        job: PendingJob,
        output_hash: Hash,
        advance: BrainAdvanceV2,
    },
    Leg {
        job: PendingJob,
        output_hash: Hash,
        leg: LegId,
        advance: LegAdvance,
    },
    Quantum {
        job: PendingJob,
        output_hash: Hash,
        leg: QuantumId,
        advance: QuantumAdvance,
    },
}

impl PendingVerify {
    /// Subject + detail if this verify path is quantum research.
    pub fn quantum_fail_hint(&self) -> Option<(String, String)> {
        match self {
            PendingVerify::Quantum { leg, .. } => Some((
                format!("quantum_{}", leg.as_str()),
                format!(
                    "Seed rejected the {} guardian train — output did not match re-exec",
                    leg.as_str()
                ),
            )),
            PendingVerify::Light { job, .. } => {
                let id = parse_research_scenario_id(&job.input)?;
                if id.starts_with("quantum_") {
                    Some((
                        id.clone(),
                        format!("Seed rejected {id} pressure-test — digests did not match"),
                    ))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// CPU re-exec / compare — call outside the AI mutex.
    /// `audit_every`: for light jobs only, full re-exec 1-in-K (`1` = always). Brain jobs always full.
    pub fn run_cpu(self) -> Result<VerifiedComplete, OrchError> {
        self.run_cpu_audited(1, 0)
    }

    pub fn run_cpu_audited(self, audit_every: u16, audit_nonce: u64) -> Result<VerifiedComplete, OrchError> {
        let every = audit_every.max(1) as u64;
        let do_full = every <= 1 || (audit_nonce % every) == 0;
        match self {
            PendingVerify::Light { job, output } => {
                if !do_full {
                    // Shape checks only — random audit catches cheats over time.
                    if output.is_empty() || output.len() > 1_048_576 {
                        return Err(OrchError::BenchmarkFailed);
                    }
                    return Ok(VerifiedComplete::Light {
                        job,
                        output_hash: Hash::digest(&output),
                    });
                }
                let q = JobQueue::default();
                let output_hash = q.verify_output(&job, &output)?;
                Ok(VerifiedComplete::Light { job, output_hash })
            }
            PendingVerify::Shared {
                job,
                output,
                weights,
                epoch,
            } => {
                let spec = crate::parse_ml_train_shared_input(&job.input)
                    .map_err(|_| OrchError::MlTrainFailed)?;
                if spec.epoch != epoch {
                    return Err(OrchError::StaleBrain);
                }
                let expected =
                    run_ml_train_shared(&weights, &job.input).map_err(|_| OrchError::MlTrainFailed)?;
                if output != expected.output.as_slice() {
                    return Err(OrchError::MlTrainFailed);
                }
                Ok(VerifiedComplete::Shared {
                    job,
                    output_hash: Hash::digest(&output),
                    advance: BrainAdvance {
                        job_epoch: spec.epoch,
                        steps: spec.steps,
                        new_weights: expected.new_weights,
                        digest: expected.weight_digest,
                        loss: expected.loss,
                        accuracy: expected.accuracy,
                    },
                })
            }
            PendingVerify::SharedV2 {
                job,
                output,
                weights,
                epoch,
            } => {
                let spec = mesh_ai_v2::parse_job(&job.input).map_err(|_| OrchError::MlTrainFailed)?;
                if spec.epoch != epoch {
                    return Err(OrchError::StaleBrain);
                }
                let expected =
                    run_job_v2(&weights, &job.input).map_err(|_| OrchError::MlTrainFailed)?;
                if output != expected.output.as_slice() {
                    return Err(OrchError::MlTrainFailed);
                }
                Ok(VerifiedComplete::SharedV2 {
                    job,
                    output_hash: Hash::digest(&output),
                    advance: BrainAdvanceV2 {
                        job_epoch: spec.epoch,
                        steps: spec.steps,
                        new_weights: expected.new_weights,
                        digest: expected.weight_digest,
                        loss_q16: expected.loss_q16,
                        accuracy_q16: expected.accuracy_q16,
                    },
                })
            }
            PendingVerify::Leg {
                job,
                output,
                weights,
                epoch,
                leg,
            } => {
                let spec = crate::parse_leg_job(&job.input).map_err(|_| OrchError::MlTrainFailed)?;
                if spec.leg != leg || spec.epoch != epoch {
                    return Err(OrchError::StaleBrain);
                }
                let expected =
                    crate::run_leg_train(&weights, &job.input).map_err(|_| OrchError::MlTrainFailed)?;
                if output != expected.output.as_slice() {
                    return Err(OrchError::MlTrainFailed);
                }
                Ok(VerifiedComplete::Leg {
                    job,
                    output_hash: Hash::digest(&output),
                    leg,
                    advance: LegAdvance {
                        job_epoch: spec.epoch,
                        steps: spec.steps,
                        new_weights: expected.new_weights,
                        digest: expected.weight_digest,
                        loss: expected.loss,
                        accuracy: expected.accuracy,
                    },
                })
            }
            PendingVerify::Quantum {
                job,
                output,
                weights,
                epoch,
                leg,
            } => {
                let spec = crate::parse_quantum_job(&job.input).map_err(|_| OrchError::MlTrainFailed)?;
                if spec.leg != leg || spec.epoch != epoch {
                    return Err(OrchError::StaleBrain);
                }
                let expected = crate::run_quantum_train(&weights, &job.input)
                    .map_err(|_| OrchError::MlTrainFailed)?;
                if output != expected.output.as_slice() {
                    return Err(OrchError::MlTrainFailed);
                }
                Ok(VerifiedComplete::Quantum {
                    job,
                    output_hash: Hash::digest(&output),
                    leg,
                    advance: QuantumAdvance {
                        job_epoch: spec.epoch,
                        steps: spec.steps,
                        new_weights: expected.new_weights,
                        digest: expected.weight_digest,
                        loss: expected.loss,
                        accuracy: expected.accuracy,
                    },
                })
            }
        }
    }
}

/// Soft cap for `POST /v1/results` batch size (`MESH_BRAIN_VERIFY_BATCH_MAX`, default 8).
pub fn brain_verify_batch_max() -> usize {
    env_u32("MESH_BRAIN_VERIFY_BATCH_MAX")
        .map(|n| n as usize)
        .unwrap_or(8)
        .clamp(1, 32)
}

/// CPU-verify several prepared jobs (parallel when >1). Call outside the AI mutex.
pub fn run_cpu_batch(pendings: Vec<PendingVerify>) -> Vec<Result<VerifiedComplete, OrchError>> {
    run_cpu_batch_audited(pendings, 1)
}

pub fn run_cpu_batch_audited(
    pendings: Vec<PendingVerify>,
    audit_every: u16,
) -> Vec<Result<VerifiedComplete, OrchError>> {
    if pendings.len() <= 1 {
        return pendings
            .into_iter()
            .enumerate()
            .map(|(i, p)| p.run_cpu_audited(audit_every, i as u64))
            .collect();
    }
    let mut handles = Vec::with_capacity(pendings.len());
    for (i, p) in pendings.into_iter().enumerate() {
        let every = audit_every;
        handles.push(std::thread::spawn(move || p.run_cpu_audited(every, i as u64)));
    }
    handles
        .into_iter()
        .map(|h| {
            h.join()
                .unwrap_or_else(|_| Err(OrchError::MlTrainFailed))
        })
        .collect()
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|&n| n > 0)
}

/// Soft caps so seed verify load stays bounded (`MESH_BRAIN_VERIFY_MAX_STEPS` / `_SAMPLES`).
fn clamp_brain_verify_size(steps: u32, samples: u32) -> (u32, u32) {
    let max_steps = env_u32("MESH_BRAIN_VERIFY_MAX_STEPS").unwrap_or(u32::MAX);
    let max_samples = env_u32("MESH_BRAIN_VERIFY_MAX_SAMPLES").unwrap_or(u32::MAX);
    (steps.min(max_steps).max(1), samples.min(max_samples).max(1))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capability {
    pub address: String,
    pub gpu_name: String,
    pub vram_mb: u32,
    /// Supported job kinds: "echo", "benchmark", "protocol_eval", "ml_train", …
    pub kinds: Vec<String>,
    /// Parallel job slots this worker can sustain (0 = derive from `vram_mb`).
    #[serde(default)]
    pub train_slots: u32,
    /// Brain backends this worker can run (e.g. `cpu_v1`, `cuda_v2`). Empty = v1-only.
    #[serde(default)]
    pub brain_backends: Vec<String>,
    /// Contract version when advertising v2 (e.g. `v2.0.0`). Empty = v1-only.
    #[serde(default)]
    pub brain_contract: String,
    /// Worker OS from `std::env::consts::OS` (`linux`, `windows`, …).
    /// f64 brains (v1 / leg / quantum) only assign when this matches the seed verifier OS.
    #[serde(default)]
    pub os_family: String,
}

impl Capability {
    pub fn supports_cuda_v2(&self) -> bool {
        self.brain_backends.iter().any(|b| b == "cuda_v2")
            && (self.brain_contract.is_empty()
                || self.brain_contract == mesh_ai_v2::BRAIN_CONTRACT)
    }

    /// True when this worker's OS matches the seed's f64 verify path (libm must match).
    pub fn matches_verifier_os_for_f64(&self) -> bool {
        if self.os_family.is_empty() {
            return false;
        }
        let seed = std::env::consts::OS;
        self.os_family == seed
            || (std::env::consts::FAMILY == "unix"
                && self.os_family != "windows"
                && seed != "windows")
    }
}

/// Jobs that re-train an f64 MLP — byte-identical only on the same OS/libm as the seed.
fn is_f64_brain_job(job: &PendingJob) -> bool {
    if !matches!(job.kind, AiJobKind::MlTrain) {
        return false;
    }
    if is_ml_train_shared_v2(&job.input) {
        return false;
    }
    is_ml_train_shared(&job.input) || is_leg_train(&job.input) || is_quantum_train(&job.input)
}

fn job_compatible_for_worker(job: &PendingJob, cap: &Capability) -> bool {
    let wire = JobQueue::wire_kind(job);
    if !cap.kinds.is_empty() {
        let advertised = cap.kinds.iter().any(|k| {
            k == &wire
                || (k == "ml_train"
                    && matches!(
                        wire.as_str(),
                        "ml_train"
                            | "ml_train_shared"
                            | "ml_train_shared_v2"
                            | "leg_train"
                            | "quantum_train"
                    ))
                || (k == "ml_train_shared"
                    && (wire == "ml_train_shared" || wire == "ml_train_shared_v2"))
        });
        if !advertised {
            return false;
        }
    }
    // GPU miners that do not advertise cpu_v1 must not be fed CPU research / f64 brains.
    let cuda_only =
        cap.supports_cuda_v2() && !cap.brain_backends.iter().any(|b| b == "cpu_v1");
    if cuda_only {
        return is_ml_train_shared_v2(&job.input);
    }
    if is_f64_brain_job(job) {
        return cap.matches_verifier_os_for_f64();
    }
    if matches!(job.kind, AiJobKind::MlTrain) && is_ml_train_shared_v2(&job.input) {
        return true;
    }
    true
}

/// Parallel capacity proxy from advertised VRAM (brain MLP is small; slots drive queue fill).
pub fn train_slots_for_vram(vram_mb: u32) -> u32 {
    match vram_mb {
        0..=2047 => 1,
        2048..=6143 => 2,
        6144..=12287 => 3,
        12288..=20479 => 4,
        20480..=32767 => 6,
        _ => 8,
    }
}

pub fn effective_train_slots(cap: &Capability) -> u32 {
    if cap.train_slots > 0 {
        cap.train_slots.min(16)
    } else {
        train_slots_for_vram(cap.vram_mb)
    }
}

pub type WorkerId = String;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingJob {
    pub job_id: String,
    pub kind: AiJobKind,
    pub input: Vec<u8>,
    pub input_commitment: Hash,
    pub assigned_to: Option<String>,
    pub created_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobAssignment {
    pub job_id: String,
    pub kind: String,
    pub input_hex: String,
    pub input_commitment: String,
    /// Build/31 protocol-assigned role for this worker.
    #[serde(default)]
    pub assigned_role: String,
}

#[derive(Default)]
pub struct JobQueue {
    workers: HashMap<WorkerId, Capability>,
    pending: VecDeque<PendingJob>,
    /// job_id → job
    inflight: HashMap<String, PendingJob>,
    next_id: u64,
    /// Soft routing: exponential moving average latency (ms) per worker.
    latency_ema: HashMap<WorkerId, f64>,
    completed: u64,
    verify_ok: u64,
    verify_fail: u64,
    /// Distinct research scenario ids completed (for MeshPulse coverage).
    research_scenarios: HashSet<String>,
    /// Seen remote job ids (gossip mirror dedupe).
    seen_remote_jobs: HashSet<String>,
    /// Job ids already announced on the P2P mesh.
    gossiped_jobs: HashSet<String>,
    protocol_eval_ok: u64,
    /// Shared network brain (one model for all workers).
    brain: Option<SharedBrain>,
    /// Shared brain v2 (Q16 mlp512) — parallel to v1.
    brain_v2: Option<SharedBrainV2>,
    /// Trilemma Guardians — four evolving specialist legs.
    legs: Option<LegBrainPack>,
    /// Quantum Research Guardians — three post-quantum readiness legs.
    quantum: Option<QuantumBrainPack>,
    /// Recent quantum train/sim outcomes for the public story feed.
    quantum_story: VecDeque<QuantumStoryBeat>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerRank {
    pub worker: String,
    pub gpu_name: String,
    pub vram_mb: u32,
    pub latency_ema_ms: f64,
    pub score: f64,
}

/// Plain-English quantum research beat for the explorer story feed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuantumStoryBeat {
    pub at: u64,
    /// worked | failed | trying
    pub outcome: String,
    pub subject: String,
    pub detail: String,
}

/// Pending / inflight quantum work item for the public story.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuantumActivityItem {
    pub phase: String,
    pub kind: String,
    pub subject: String,
    pub job_id: String,
    pub detail: String,
}

/// Durable pending/inflight snap for restart recovery (Build/27 N8).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DurableQueueSnap {
    pub pending: Vec<PendingJob>,
    pub inflight: Vec<PendingJob>,
    pub next_id: u64,
    #[serde(default)]
    pub seen_remote_jobs: Vec<String>,
}

impl JobQueue {
    pub fn export_durable(&self) -> DurableQueueSnap {
        let mut seen: Vec<_> = self.seen_remote_jobs.iter().cloned().collect();
        seen.sort();
        if seen.len() > 4_096 {
            seen = seen.split_off(seen.len() - 4_096);
        }
        DurableQueueSnap {
            pending: self.pending.iter().cloned().collect(),
            inflight: self.inflight.values().cloned().collect(),
            next_id: self.next_id,
            seen_remote_jobs: seen,
        }
    }

    /// Restore durable jobs (brains/workers stay as-is). Caps sizes.
    pub fn import_durable(&mut self, snap: DurableQueueSnap) {
        self.next_id = self.next_id.max(snap.next_id);
        for id in snap.seen_remote_jobs.into_iter().take(4_096) {
            self.seen_remote_jobs.insert(id);
        }
        for job in snap.pending.into_iter().take(256) {
            if !self.pending.iter().any(|p| p.job_id == job.job_id)
                && !self.inflight.contains_key(&job.job_id)
            {
                self.pending.push_back(job);
            }
        }
        for job in snap.inflight.into_iter().take(256) {
            // Re-queue abandoned inflight so workers can reclaim after restart.
            if !self.pending.iter().any(|p| p.job_id == job.job_id)
                && !self.inflight.contains_key(&job.job_id)
            {
                let mut j = job;
                j.assigned_to = None;
                self.pending.push_back(j);
            }
        }
    }

    pub fn with_brain(brain: SharedBrain) -> Self {
        let mut q = Self::default();
        q.brain = Some(brain);
        q
    }

    pub fn with_brains(brain: SharedBrain, brain_v2: SharedBrainV2) -> Self {
        let mut q = Self::default();
        q.brain = Some(brain);
        q.brain_v2 = Some(brain_v2);
        q
    }

    pub fn with_brains_and_legs(
        brain: SharedBrain,
        brain_v2: SharedBrainV2,
        legs: LegBrainPack,
    ) -> Self {
        Self::with_brains_legs_quantum(brain, brain_v2, legs, QuantumBrainPack::genesis(None))
    }

    pub fn with_brains_legs_quantum(
        brain: SharedBrain,
        brain_v2: SharedBrainV2,
        legs: LegBrainPack,
        quantum: QuantumBrainPack,
    ) -> Self {
        let mut q = Self::default();
        q.brain = Some(brain);
        q.brain_v2 = Some(brain_v2);
        q.legs = Some(legs);
        q.quantum = Some(quantum);
        q
    }

    pub fn brain(&self) -> Option<&SharedBrain> {
        self.brain.as_ref()
    }

    pub fn brain_mut(&mut self) -> Option<&mut SharedBrain> {
        self.brain.as_mut()
    }

    pub fn brain_v2(&self) -> Option<&SharedBrainV2> {
        self.brain_v2.as_ref()
    }

    pub fn brain_v2_mut(&mut self) -> Option<&mut SharedBrainV2> {
        self.brain_v2.as_mut()
    }

    pub fn legs(&self) -> Option<&LegBrainPack> {
        self.legs.as_ref()
    }

    pub fn legs_mut(&mut self) -> Option<&mut LegBrainPack> {
        self.legs.as_mut()
    }

    pub fn quantum(&self) -> Option<&QuantumBrainPack> {
        self.quantum.as_ref()
    }

    pub fn quantum_mut(&mut self) -> Option<&mut QuantumBrainPack> {
        self.quantum.as_mut()
    }

    pub fn advertise(&mut self, cap: Capability) -> Result<WorkerId, OrchError> {
        if Address::from_hex(&cap.address).is_none() {
            return Err(OrchError::BadAddress);
        }
        let id = cap.address.clone();
        self.workers.insert(id.clone(), cap);
        self.latency_ema.entry(id.clone()).or_insert(50.0);
        Ok(id)
    }

    pub fn workers(&self) -> impl Iterator<Item = &Capability> {
        self.workers.values()
    }

    /// Workers that advertise cuda_v2 at/above the VRAM floor.
    pub fn cuda_v2_worker_count(&self, vram_floor_mb: u32) -> u32 {
        self.workers
            .values()
            .filter(|w| w.supports_cuda_v2() && w.vram_mb >= vram_floor_mb)
            .count() as u32
    }

    /// Soft gate: enqueue v2 when prefer_v2 and enough capable workers.
    pub fn should_enqueue_brain_v2(
        &self,
        prefer_v2: u8,
        min_workers: u32,
        vram_floor_mb: u32,
    ) -> bool {
        prefer_v2 > 0
            && self.brain_v2.is_some()
            && self.cuda_v2_worker_count(vram_floor_mb) >= min_workers.max(1)
    }

    pub fn total_train_slots(&self) -> u32 {
        self.workers
            .values()
            .map(effective_train_slots)
            .sum::<u32>()
            .max(1)
    }

    pub fn total_vram_mb(&self) -> u64 {
        self.workers.values().map(|w| w.vram_mb as u64).sum()
    }

    /// Target pending+inflight depth from worker capacity and soft stipend envelope.
    /// Keep this shallow so miners see a stream, not a dumped board.
    pub fn target_queue_depth(&self, stipend_cap: u32) -> usize {
        let slots = self.total_train_slots() as u64;
        let workers = self.workers.len().max(1) as u64;
        let base = slots.saturating_add(workers);
        let scaled = base.saturating_mul(stipend_cap as u64).max(1) / 1_000.max(1);
        let depth = base.max(scaled).max(4);
        depth.clamp(4, 12) as usize
    }

    /// Shared-brain job intensity from growth + worker VRAM (fewer, heavier jobs).
    pub fn sized_shared_train(&self, worker: Option<&str>, growth: u64) -> (u32, u32) {
        let vram = worker
            .and_then(|id| self.workers.get(id))
            .map(|w| w.vram_mb)
            .unwrap_or_else(|| {
                let n = self.workers.len().max(1) as u64;
                (self.total_vram_mb() / n) as u32
            });
        let slots = train_slots_for_vram(vram) as u64;
        let g = growth.max(1);
        let steps = (96u64.saturating_mul(g).saturating_mul(slots) / 2).clamp(128, 1024) as u32;
        let samples = (320u64.saturating_mul(g).saturating_mul(slots) / 2).clamp(256, 2048) as u32;
        clamp_brain_verify_size(steps, samples)
    }

    /// v2 sizing — heavier steps/samples from VRAM (clamped to contract bounds).
    pub fn sized_shared_train_v2(&self, growth: u64, vram_floor_mb: u32) -> (u32, u32) {
        let vram = self
            .workers
            .values()
            .filter(|w| w.supports_cuda_v2() && w.vram_mb >= vram_floor_mb)
            .map(|w| w.vram_mb)
            .max()
            .unwrap_or(vram_floor_mb);
        let slots = train_slots_for_vram(vram) as u64;
        let g = growth.max(1);
        // Prefer mid-heavy parallelizable jobs (contract max steps/samples raised in v2.1).
        let steps = (64u64.saturating_mul(g).saturating_mul(slots) / 2).clamp(128, 1024) as u32;
        let samples = (256u64.saturating_mul(g).saturating_mul(slots) / 2).clamp(256, 2048) as u32;
        clamp_brain_verify_size(steps, samples)
    }

    /// How many concurrent shared-v2 jobs to keep queued (race same epoch; first advances).
    pub fn shared_v2_target_inflight(&self, vram_floor_mb: u32) -> usize {
        let cuda_slots: u32 = self
            .workers
            .values()
            .filter(|w| w.supports_cuda_v2() && w.vram_mb >= vram_floor_mb)
            .map(effective_train_slots)
            .sum();
        (cuda_slots as usize).clamp(1, 2)
    }

    pub fn shared_v2_jobs_queued_count(&self) -> usize {
        self.pending
            .iter()
            .chain(self.inflight.values())
            .filter(|j| matches!(j.kind, AiJobKind::MlTrain) && is_ml_train_shared_v2(&j.input))
            .count()
    }

    fn shared_inflight_or_pending(&self) -> bool {
        self.pending
            .iter()
            .chain(self.inflight.values())
            .any(|j| matches!(j.kind, AiJobKind::MlTrain) && is_ml_train_shared(&j.input))
    }

    fn shared_v2_inflight_or_pending(&self) -> bool {
        self.pending
            .iter()
            .chain(self.inflight.values())
            .any(|j| matches!(j.kind, AiJobKind::MlTrain) && is_ml_train_shared_v2(&j.input))
    }

    /// True when a shared-brain job is already pending or in flight (epochs are sequential).
    pub fn shared_job_queued(&self) -> bool {
        self.shared_inflight_or_pending()
    }

    pub fn shared_v2_job_queued(&self) -> bool {
        self.shared_v2_inflight_or_pending()
    }

    fn leg_inflight_or_pending(&self, leg: LegId) -> bool {
        self.pending
            .iter()
            .chain(self.inflight.values())
            .filter(|j| matches!(j.kind, AiJobKind::MlTrain) && is_leg_train(&j.input))
            .any(|j| {
                crate::parse_leg_job(&j.input)
                    .map(|s| s.leg == leg)
                    .unwrap_or(false)
            })
    }

    pub fn leg_jobs_queued(&self) -> usize {
        self.pending
            .iter()
            .chain(self.inflight.values())
            .filter(|j| matches!(j.kind, AiJobKind::MlTrain) && is_leg_train(&j.input))
            .count()
    }

    fn quantum_inflight_or_pending(&self, leg: QuantumId) -> bool {
        self.pending
            .iter()
            .chain(self.inflight.values())
            .filter(|j| matches!(j.kind, AiJobKind::MlTrain) && is_quantum_train(&j.input))
            .any(|j| {
                crate::parse_quantum_job(&j.input)
                    .map(|s| s.leg == leg)
                    .unwrap_or(false)
            })
    }

    pub fn quantum_jobs_queued(&self) -> usize {
        self.pending
            .iter()
            .chain(self.inflight.values())
            .filter(|j| matches!(j.kind, AiJobKind::MlTrain) && is_quantum_train(&j.input))
            .count()
    }

    /// Rank workers for soft routing. Higher `min_verifier_weight` penalizes latency harder
    /// (prefer reliable GPUs when research raised the verifier bar).
    pub fn rank_workers(&self) -> Vec<WorkerRank> {
        self.rank_workers_biased(1)
    }

    pub fn rank_workers_biased(&self, min_verifier_weight: u64) -> Vec<WorkerRank> {
        let penalty = 1.0 + (min_verifier_weight.saturating_sub(1) as f64) * 0.2;
        let mut ranks: Vec<WorkerRank> = self
            .workers
            .values()
            .map(|w| {
                let lat = self.latency_ema.get(&w.address).copied().unwrap_or(50.0);
                let score = 10_000.0 / (lat + 1.0).powf(penalty) + (w.vram_mb as f64) * 0.01;
                WorkerRank {
                    worker: w.address.clone(),
                    gpu_name: w.gpu_name.clone(),
                    vram_mb: w.vram_mb,
                    latency_ema_ms: lat,
                    score,
                }
            })
            .collect();
        ranks.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranks
    }

    pub fn preferred_worker(&self) -> Option<String> {
        self.rank_workers().into_iter().next().map(|r| r.worker)
    }

    pub fn preferred_worker_biased(&self, min_verifier_weight: u64) -> Option<String> {
        self.rank_workers_biased(min_verifier_weight)
            .into_iter()
            .next()
            .map(|r| r.worker)
    }

    pub fn record_latency(&mut self, worker: &str, latency_ms: u64) {
        let alpha = 0.3;
        let entry = self.latency_ema.entry(worker.to_string()).or_insert(50.0);
        *entry = alpha * (latency_ms as f64) + (1.0 - alpha) * *entry;
    }

    pub fn completed(&self) -> u64 {
        self.completed
    }

    pub fn verify_ok(&self) -> u64 {
        self.verify_ok
    }

    pub fn verify_fail(&self) -> u64 {
        self.verify_fail
    }

    pub fn verify_ok_rate(&self) -> f64 {
        let n = self.verify_ok + self.verify_fail;
        if n == 0 {
            1.0
        } else {
            self.verify_ok as f64 / n as f64
        }
    }

    pub fn protocol_eval_ok(&self) -> u64 {
        self.protocol_eval_ok
    }

    pub fn research_scenarios_touched(&self) -> u32 {
        self.research_scenarios.len() as u32
    }

    pub fn research_scenario_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.research_scenarios.iter().cloned().collect();
        ids.sort();
        ids
    }

    pub fn scenario_touched(&self, id: &str) -> bool {
        self.research_scenarios.contains(id)
    }

    pub fn push_quantum_story(&mut self, outcome: &str, subject: &str, detail: &str) {
        self.quantum_story.push_back(QuantumStoryBeat {
            at: now_secs(),
            outcome: outcome.into(),
            subject: subject.into(),
            detail: detail.into(),
        });
        while self.quantum_story.len() > 32 {
            self.quantum_story.pop_front();
        }
    }

    pub fn quantum_story(&self) -> Vec<QuantumStoryBeat> {
        self.quantum_story.iter().cloned().collect()
    }

    /// Pending + inflight quantum train / quantum protocol jobs (for "trying now").
    pub fn quantum_activity(&self) -> Vec<QuantumActivityItem> {
        let mut out = Vec::new();
        for (phase, job) in self
            .pending
            .iter()
            .map(|j| ("queued", j))
            .chain(self.inflight.values().map(|j| ("running", j)))
        {
            if matches!(job.kind, AiJobKind::MlTrain) && is_quantum_train(&job.input) {
                let leg = crate::parse_quantum_job(&job.input)
                    .map(|s| s.leg.as_str().to_string())
                    .unwrap_or_else(|_| "unknown".into());
                out.push(QuantumActivityItem {
                    phase: phase.into(),
                    kind: "quantum_train".into(),
                    subject: format!("quantum_{leg}"),
                    job_id: job.job_id.clone(),
                    detail: format!("Training the {leg} quantum guardian"),
                });
            } else if matches!(job.kind, AiJobKind::ProtocolEval) {
                if let Some(id) = parse_research_scenario_id(&job.input) {
                    if id.starts_with("quantum_") {
                        out.push(QuantumActivityItem {
                            phase: phase.into(),
                            kind: "protocol_eval".into(),
                            subject: id.clone(),
                            job_id: job.job_id.clone(),
                            detail: format!("Running {id} pressure-test sim"),
                        });
                    }
                }
            }
        }
        out
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn inflight_len(&self) -> usize {
        self.inflight.len()
    }

    /// True if any connected worker advertises this job kind (or a parent kind).
    pub fn any_worker_advertises_kind(&self, kind: &str) -> bool {
        self.workers.values().any(|c| {
            c.kinds.iter().any(|k| {
                k == kind
                    || (k == "ml_train"
                        && matches!(
                            kind,
                            "ml_train"
                                | "ml_train_shared"
                                | "ml_train_shared_v2"
                                | "leg_train"
                                | "quantum_train"
                        ))
                    || (k == "ml_train_shared"
                        && matches!(kind, "ml_train_shared" | "ml_train_shared_v2"))
            })
        })
    }

    pub fn queue_depth(&self) -> usize {
        self.pending.len() + self.inflight.len()
    }

    /// First research scenario not yet completed on this orchestrator, if any.
    pub fn next_uncovered_scenario(&self) -> Option<ResearchScenario> {
        ResearchScenario::all()
            .iter()
            .copied()
            .find(|s| !self.research_scenarios.contains(s.as_str()))
    }

    /// Prefer uncovered scenarios, else MeshPulse-driven suggestion.
    pub fn pick_research_scenario(
        &self,
        gpu_vs_height: f64,
        avg_latency_ms: f64,
    ) -> ResearchScenario {
        self.next_uncovered_scenario().unwrap_or_else(|| {
            crate::research::suggest_scenario(gpu_vs_height, avg_latency_ms, self.verify_ok_rate())
        })
    }

    /// Mirror a gossiped job into the local queue (idempotent).
    /// Shared-brain / leg jobs are tracked only (not enqueued) so non-seed nodes
    /// cannot fork the canonical brain by verifying them locally.
    pub fn ingest_remote_job(
        &mut self,
        job_id: String,
        kind: &str,
        input: Vec<u8>,
        input_commitment: Hash,
    ) -> bool {
        if self.pending.iter().any(|j| j.job_id == job_id)
            || self.inflight.contains_key(&job_id)
            || self.seen_remote_jobs.contains(&job_id)
        {
            return false;
        }
        self.seen_remote_jobs.insert(job_id.clone());
        if self.seen_remote_jobs.len() > 2_000 {
            self.seen_remote_jobs.clear();
        }
        let brainish = matches!(
            kind,
            "ml_train"
                | "ml_train_shared"
                | "ml_train_shared_v2"
                | "leg_train"
                | "quantum_train"
        ) || is_ml_train_shared(&input)
            || is_ml_train_shared_v2(&input)
            || is_leg_train(&input)
            || is_quantum_train(&input);
        if brainish {
            return true;
        }
        // Edge boards without a brain still receive every seed protocol_eval via gossip.
        // Cap so pending cannot grow unbounded (was 2500+ → RPC timeouts on :18081).
        let max_remote_pending = if self.brain.is_some() { 512 } else { 48 };
        if self.pending.len() >= max_remote_pending {
            return false;
        }
        let job_kind = match kind {
            "echo" => AiJobKind::Echo,
            "benchmark" => AiJobKind::Benchmark,
            "protocol_eval" => AiJobKind::ProtocolEval,
            "agent_assist" => AiJobKind::AgentAssist,
            _ => AiJobKind::ProtocolEval,
        };
        let commit = if input_commitment == Hash::default() {
            Hash::digest(&input)
        } else {
            input_commitment
        };
        self.pending.push_back(PendingJob {
            job_id,
            kind: job_kind,
            input,
            input_commitment: commit,
            assigned_to: None,
            created_at: now_secs(),
        });
        true
    }

    /// Kind string used on the wire / worker API for a pending job.
    pub fn wire_kind(job: &PendingJob) -> String {
        if matches!(job.kind, AiJobKind::MlTrain) && is_quantum_train(&job.input) {
            "quantum_train".into()
        } else if matches!(job.kind, AiJobKind::MlTrain) && is_leg_train(&job.input) {
            "leg_train".into()
        } else if matches!(job.kind, AiJobKind::MlTrain) && is_ml_train_shared_v2(&job.input) {
            "ml_train_shared_v2".into()
        } else if matches!(job.kind, AiJobKind::MlTrain) && is_ml_train_shared(&job.input) {
            "ml_train_shared".into()
        } else {
            match job.kind {
                AiJobKind::Echo => "echo".into(),
                AiJobKind::Benchmark => "benchmark".into(),
                AiJobKind::ProtocolEval => "protocol_eval".into(),
                AiJobKind::AgentAssist => "agent_assist".into(),
                AiJobKind::MlTrain => "ml_train".into(),
            }
        }
    }

    /// Jobs not yet gossiped to the mesh (marks them as gossiped).
    pub fn take_ungossiped(&mut self) -> Vec<PendingJob> {
        let mut out = Vec::new();
        for j in self.pending.iter().chain(self.inflight.values()) {
            if self.gossiped_jobs.insert(j.job_id.clone()) {
                out.push(j.clone());
            }
        }
        if self.gossiped_jobs.len() > 4_000 {
            self.gossiped_jobs.clear();
        }
        out
    }

    /// Enqueue an echo job (input bytes echoed via blake3 proof).
    pub fn enqueue_echo(&mut self, input: Vec<u8>) -> PendingJob {
        let input_commitment = Hash::digest(&input);
        let job_id = format!("echo-{}", self.next_id);
        self.next_id += 1;
        let job = PendingJob {
            job_id: job_id.clone(),
            kind: AiJobKind::Echo,
            input,
            input_commitment,
            assigned_to: None,
            created_at: now_secs(),
        };
        self.pending.push_back(job.clone());
        job
    }

    pub fn enqueue_benchmark(&mut self, rounds: u32) -> PendingJob {
        let input = rounds.to_le_bytes().to_vec();
        let input_commitment = Hash::digest(&input);
        let job_id = format!("bench-{}", self.next_id);
        self.next_id += 1;
        let job = PendingJob {
            job_id: job_id.clone(),
            kind: AiJobKind::Benchmark,
            input,
            input_commitment,
            assigned_to: None,
            created_at: now_secs(),
        };
        self.pending.push_back(job.clone());
        job
    }

    /// Protocol-eval: hash MeshPulse-like feature bytes (research, GPU-paid).
    pub fn enqueue_protocol_eval(&mut self, feature_bytes: Vec<u8>) -> PendingJob {
        let input_commitment = Hash::digest(&feature_bytes);
        let job_id = format!("eval-{}", self.next_id);
        self.next_id += 1;
        let job = PendingJob {
            job_id: job_id.clone(),
            kind: AiJobKind::ProtocolEval,
            input: feature_bytes,
            input_commitment,
            assigned_to: None,
            created_at: now_secs(),
        };
        self.pending.push_back(job.clone());
        job
    }

    /// Enqueue a named Phase-5 research scenario (deterministic payload).
    pub fn enqueue_research(
        &mut self,
        scenario: ResearchScenario,
        height: u64,
        pulse_signal: f64,
    ) -> PendingJob {
        self.enqueue_protocol_eval(scenario.encode(height, pulse_signal))
    }

    /// Shared-brain MNIST job (continues from current epoch).
    pub fn enqueue_ml_train_shared(
        &mut self,
        steps: u32,
        lr_milli: u32,
        samples: u32,
        offset: u32,
    ) -> Option<PendingJob> {
        let brain = self.brain.as_ref()?;
        let input = brain.encode_job(steps, lr_milli, samples, offset);
        let input_commitment = Hash::digest(&input);
        let job_id = format!("mlshare-{}", self.next_id);
        self.next_id += 1;
        let job = PendingJob {
            job_id: job_id.clone(),
            kind: AiJobKind::MlTrain,
            input,
            input_commitment,
            assigned_to: None,
            created_at: now_secs(),
        };
        self.pending.push_back(job.clone());
        Some(job)
    }

    /// Shared-brain v2 job (Q16 mlp512).
    pub fn enqueue_ml_train_shared_v2(
        &mut self,
        steps: u32,
        lr_milli: u32,
        samples: u32,
        offset: u32,
    ) -> Option<PendingJob> {
        let brain = self.brain_v2.as_ref()?;
        let input = brain.encode_job(steps, lr_milli, samples, offset);
        let input_commitment = Hash::digest(&input);
        let job_id = format!("mlshare2-{}", self.next_id);
        self.next_id += 1;
        let job = PendingJob {
            job_id: job_id.clone(),
            kind: AiJobKind::MlTrain,
            input,
            input_commitment,
            assigned_to: None,
            created_at: now_secs(),
        };
        self.pending.push_back(job.clone());
        Some(job)
    }

    /// Trilemma Guardian leg-train job.
    pub fn enqueue_leg_train(
        &mut self,
        leg: LegId,
        steps: u32,
        lr_milli: u32,
        samples: u32,
        offset: u32,
    ) -> Option<PendingJob> {
        if self.leg_inflight_or_pending(leg) {
            return None;
        }
        let pack = self.legs.as_ref()?;
        let input = pack.encode_job(leg, steps, lr_milli, samples, offset);
        let input_commitment = Hash::digest(&input);
        let job_id = format!("leg-{}-{}", leg.as_str(), self.next_id);
        self.next_id += 1;
        let job = PendingJob {
            job_id: job_id.clone(),
            kind: AiJobKind::MlTrain,
            input,
            input_commitment,
            assigned_to: None,
            created_at: now_secs(),
        };
        self.pending.push_back(job.clone());
        Some(job)
    }

    /// Quantum Research Guardian leg-train job.
    pub fn enqueue_quantum_train(
        &mut self,
        leg: QuantumId,
        steps: u32,
        lr_milli: u32,
        samples: u32,
        offset: u32,
    ) -> Option<PendingJob> {
        if self.quantum_inflight_or_pending(leg) {
            return None;
        }
        let pack = self.quantum.as_ref()?;
        let input = pack.encode_job(leg, steps, lr_milli, samples, offset);
        let input_commitment = Hash::digest(&input);
        let job_id = format!("qleg-{}-{}", leg.as_str(), self.next_id);
        self.next_id += 1;
        let job = PendingJob {
            job_id: job_id.clone(),
            kind: AiJobKind::MlTrain,
            input,
            input_commitment,
            assigned_to: None,
            created_at: now_secs(),
        };
        self.pending.push_back(job.clone());
        Some(job)
    }

    /// Real MNIST training job (GPU market) — legacy scratch init.
    pub fn enqueue_ml_train(
        &mut self,
        steps: u32,
        lr_milli: u32,
        seed: u64,
        samples: u32,
        offset: u32,
    ) -> PendingJob {
        let input = crate::encode_ml_train_input(steps, lr_milli, seed, samples, offset);
        let input_commitment = Hash::digest(&input);
        let job_id = format!("mltrain-{}", self.next_id);
        self.next_id += 1;
        let job = PendingJob {
            job_id: job_id.clone(),
            kind: AiJobKind::MlTrain,
            input,
            input_commitment,
            assigned_to: None,
            created_at: now_secs(),
        };
        self.pending.push_back(job.clone());
        job
    }

    /// Assign next matching job to worker, or auto-enqueue work sized to capacity.
    pub fn take_job(&mut self, worker: &str) -> Result<JobAssignment, OrchError> {
        let cap = self
            .workers
            .get(worker)
            .cloned()
            .ok_or(OrchError::UnknownWorker)?;
        let worker_cuda = cap.supports_cuda_v2();
        let f64_ok = cap.matches_verifier_os_for_f64();

        let compatible_idx = self
            .pending
            .iter()
            .position(|j| job_compatible_for_worker(j, &cap));

        if compatible_idx.is_none() {
            let growth = 2u64;
            let v2_n = self.shared_v2_jobs_queued_count();
            let v2_want = self.shared_v2_target_inflight(4_096);
            if worker_cuda && self.brain_v2.is_some() && v2_n < v2_want {
                let (steps, samples) = self.sized_shared_train_v2(growth, 4_096);
                let _ = self.enqueue_ml_train_shared_v2(
                    steps,
                    50,
                    samples,
                    (now_secs() % 3500) as u32,
                );
            } else if f64_ok
                && self.brain.is_some()
                && !self.shared_inflight_or_pending()
                && !self.shared_v2_inflight_or_pending()
            {
                let (steps, samples) = self.sized_shared_train(Some(worker), growth);
                let _ = self.enqueue_ml_train_shared(
                    steps,
                    50,
                    samples,
                    (now_secs() % 3500) as u32,
                );
            } else if worker_cuda
                && self.brain_v2.is_some()
                && !self.shared_v2_inflight_or_pending()
            {
                // Windows / cross-OS CUDA miners: keep feeding Q16 v2, never f64 v1.
                let (steps, samples) = self.sized_shared_train_v2(growth, 4_096);
                let _ = self.enqueue_ml_train_shared_v2(
                    steps,
                    50,
                    samples,
                    (now_secs() % 3500) as u32,
                );
            } else if self.brain.is_none() && self.brain_v2.is_none() {
                let _ = self.enqueue_ml_train(48, 50, now_secs(), 256, (now_secs() % 3500) as u32);
            } else {
                return Err(OrchError::NoJob);
            }
        }

        let idx = self
            .pending
            .iter()
            .position(|j| job_compatible_for_worker(j, &cap))
            .ok_or(OrchError::NoJob)?;
        let mut job = self.pending.remove(idx).ok_or(OrchError::NoJob)?;
        job.assigned_to = Some(worker.to_string());
        let kind = Self::wire_kind(&job);
        let assigned_role = if cap.supports_cuda_v2() {
            if kind.contains("protocol") {
                "protocol"
            } else {
                "ai_gpu"
            }
        } else if kind.contains("protocol") {
            "verify_assist"
        } else {
            "pow_cpu"
        };
        let assign = JobAssignment {
            job_id: job.job_id.clone(),
            kind,
            input_hex: hex::encode(&job.input),
            input_commitment: job.input_commitment.to_string(),
            assigned_role: assigned_role.into(),
        };
        self.inflight.insert(job.job_id.clone(), job);
        Ok(assign)
    }

    /// Verify worker result and build a receipt (caller credits chain).
    /// `height` is used when advancing the shared brain.
    pub fn complete(
        &mut self,
        worker: &str,
        job_id: &str,
        output_hex: &str,
        latency_ms: u64,
    ) -> Result<AiJobReceipt, OrchError> {
        self.complete_at(worker, job_id, output_hex, latency_ms, 0)
    }

    pub fn complete_at(
        &mut self,
        worker: &str,
        job_id: &str,
        output_hex: &str,
        latency_ms: u64,
        height: u64,
    ) -> Result<AiJobReceipt, OrchError> {
        let pending = self.prepare_complete(worker, job_id, output_hex)?;
        let fail_hint = pending.quantum_fail_hint();
        match pending.run_cpu() {
            Ok(verified) => self.finish_complete(verified, latency_ms, height),
            Err(e) => {
                if let Some((subject, detail)) = fail_hint {
                    self.push_quantum_story("failed", &subject, &detail);
                }
                self.note_verify_fail();
                Err(e)
            }
        }
    }

    pub fn note_verify_fail(&mut self) {
        self.verify_fail = self.verify_fail.saturating_add(1);
    }

    /// Take job + clone brain weights under the AI lock (Build/27 N9).
    pub fn prepare_complete(
        &mut self,
        worker: &str,
        job_id: &str,
        output_hex: &str,
    ) -> Result<PendingVerify, OrchError> {
        let job = self.inflight.get(job_id).ok_or(OrchError::NoJob)?.clone();
        if job.assigned_to.as_deref() != Some(worker) {
            return Err(OrchError::JobMismatch);
        }
        let output = match hex::decode(output_hex) {
            Ok(o) => o,
            Err(_) => {
                self.verify_fail = self.verify_fail.saturating_add(1);
                self.inflight.remove(job_id);
                return Err(OrchError::EchoFailed);
            }
        };
        self.inflight.remove(job_id);

        let quantum_job = matches!(job.kind, AiJobKind::MlTrain) && is_quantum_train(&job.input);
        let leg_job = matches!(job.kind, AiJobKind::MlTrain) && is_leg_train(&job.input);
        let shared_v2 =
            matches!(job.kind, AiJobKind::MlTrain) && is_ml_train_shared_v2(&job.input);
        let shared = matches!(job.kind, AiJobKind::MlTrain) && is_ml_train_shared(&job.input);

        if quantum_job {
            let pack = self.quantum.as_ref().ok_or(OrchError::MlTrainFailed)?;
            let spec =
                crate::parse_quantum_job(&job.input).map_err(|_| OrchError::MlTrainFailed)?;
            Ok(PendingVerify::Quantum {
                job,
                output,
                weights: pack.weights(spec.leg).to_vec(),
                epoch: pack.epoch(spec.leg),
                leg: spec.leg,
            })
        } else if leg_job {
            let pack = self.legs.as_ref().ok_or(OrchError::MlTrainFailed)?;
            let spec = crate::parse_leg_job(&job.input).map_err(|_| OrchError::MlTrainFailed)?;
            Ok(PendingVerify::Leg {
                job,
                output,
                weights: pack.weights(spec.leg).to_vec(),
                epoch: pack.epoch(spec.leg),
                leg: spec.leg,
            })
        } else if shared_v2 {
            let brain = self.brain_v2.as_ref().ok_or(OrchError::MlTrainFailed)?;
            Ok(PendingVerify::SharedV2 {
                job,
                output,
                weights: brain.weights.clone(),
                epoch: brain.epoch,
            })
        } else if shared {
            let brain = self.brain.as_ref().ok_or(OrchError::MlTrainFailed)?;
            Ok(PendingVerify::Shared {
                job,
                output,
                weights: brain.weights.clone(),
                epoch: brain.epoch,
            })
        } else {
            Ok(PendingVerify::Light { job, output })
        }
    }

    /// Apply verified result under the AI lock and build the receipt.
    pub fn finish_complete(
        &mut self,
        verified: VerifiedComplete,
        latency_ms: u64,
        height: u64,
    ) -> Result<AiJobReceipt, OrchError> {
        let (job, output_hash) = match &verified {
            VerifiedComplete::Light { job, output_hash, .. }
            | VerifiedComplete::Shared { job, output_hash, .. }
            | VerifiedComplete::SharedV2 { job, output_hash, .. }
            | VerifiedComplete::Leg { job, output_hash, .. }
            | VerifiedComplete::Quantum { job, output_hash, .. } => (job.clone(), *output_hash),
        };

        let mut raced_stale = false;
        match verified {
            VerifiedComplete::Light { .. } => {}
            VerifiedComplete::Shared { advance, .. } => {
                let brain = self.brain.as_mut().ok_or(OrchError::MlTrainFailed)?;
                match brain.apply_advance(advance, height) {
                    Ok(()) => {}
                    Err(crate::brain::BrainError::StaleEpoch { .. }) => raced_stale = true,
                    Err(_) => return Err(OrchError::MlTrainFailed),
                }
            }
            VerifiedComplete::SharedV2 { advance, .. } => {
                let brain = self.brain_v2.as_mut().ok_or(OrchError::MlTrainFailed)?;
                match brain.apply_advance(advance, height) {
                    Ok(()) => {}
                    Err(mesh_ai_v2::BrainError::StaleEpoch { .. }) => raced_stale = true,
                    Err(_) => return Err(OrchError::MlTrainFailed),
                }
            }
            VerifiedComplete::Leg { leg, advance, .. } => {
                let pack = self.legs.as_mut().ok_or(OrchError::MlTrainFailed)?;
                match pack.apply_advance(leg, advance) {
                    Ok(()) => {}
                    Err(crate::LegBrainError::StaleEpoch { .. }) => raced_stale = true,
                    Err(_) => return Err(OrchError::MlTrainFailed),
                }
            }
            VerifiedComplete::Quantum { leg, advance, .. } => {
                let pack = self.quantum.as_mut().ok_or(OrchError::MlTrainFailed)?;
                match pack.apply_advance(leg, advance) {
                    Ok(()) => {}
                    Err(crate::QuantumBrainError::StaleEpoch { .. }) => raced_stale = true,
                    Err(_) => return Err(OrchError::MlTrainFailed),
                }
            }
        }

        let quantum_job = matches!(job.kind, AiJobKind::MlTrain) && is_quantum_train(&job.input);
        let leg_job = matches!(job.kind, AiJobKind::MlTrain) && is_leg_train(&job.input);
        let shared_v2 =
            matches!(job.kind, AiJobKind::MlTrain) && is_ml_train_shared_v2(&job.input);
        let shared = matches!(job.kind, AiJobKind::MlTrain) && is_ml_train_shared(&job.input);

        let mut weight = match job.kind {
            AiJobKind::Echo => 1,
            AiJobKind::Benchmark => {
                let rounds = u32::from_le_bytes(job.input[..4].try_into().unwrap_or([0; 4]));
                rounds.max(1) as u64
            }
            // Heavy research pays more than tiny sims (was 25).
            AiJobKind::ProtocolEval => 100,
            AiJobKind::AgentAssist => 20,
            AiJobKind::MlTrain => {
                if quantum_job {
                    let steps = crate::parse_quantum_job(&job.input)
                        .map(|s| s.steps)
                        .unwrap_or(32);
                    (steps as u64).saturating_mul(6).max(48)
                } else if leg_job {
                    let steps = crate::parse_leg_job(&job.input)
                        .map(|s| s.steps)
                        .unwrap_or(32);
                    (steps as u64).saturating_mul(6).max(48)
                } else if shared_v2 {
                    let steps = mesh_ai_v2::parse_job(&job.input)
                        .map(|s| s.steps)
                        .unwrap_or(32);
                    (steps as u64).saturating_mul(12).max(96)
                } else if shared {
                    let steps = crate::parse_ml_train_shared_input(&job.input)
                        .map(|s| s.steps)
                        .unwrap_or(32);
                    (steps as u64).saturating_mul(4).max(32)
                } else {
                    let steps = crate::parse_ml_train_input(&job.input)
                        .map(|s| s.steps)
                        .unwrap_or(32);
                    (steps as u64).saturating_mul(2).max(16)
                }
            }
        };
        // Correct train that lost the epoch race still earns partial GPU credit.
        if raced_stale {
            weight = (weight / 4).max(8);
        }
        let worker = job.assigned_to.clone().unwrap_or_default();
        let worker_addr = Address::from_hex(&worker).ok_or(OrchError::BadAddress)?;
        self.record_latency(&worker, latency_ms);
        self.completed = self.completed.saturating_add(1);
        self.verify_ok = self.verify_ok.saturating_add(1);
        let mut research_scenario = String::new();
        let mut score_primary = 0.0;
        let mut score_orphan_risk = 0.0;
        let mut score_detect_rate = 0.0;
        let mut score_linkability = 0.0;
        let mut score_backlog_ratio = 0.0;
        let mut score_latency_p95_ms = 0.0;
        if matches!(job.kind, AiJobKind::ProtocolEval) {
            self.protocol_eval_ok = self.protocol_eval_ok.saturating_add(1);
            if let Some(id) = parse_research_scenario_id(&job.input) {
                self.research_scenarios.insert(id);
            }
            if let Some(result) = crate::protocol_sim::eval_research_input(&job.input) {
                research_scenario = result.scenario.clone();
                score_primary = result.scores.primary;
                score_orphan_risk = result.scores.orphan_risk;
                score_detect_rate = result.scores.detect_rate;
                score_linkability = result.scores.linkability;
                score_backlog_ratio = result.scores.backlog_ratio;
                score_latency_p95_ms = result.scores.latency_p95_ms;
            }
        }
        if quantum_job {
            research_scenario = crate::parse_quantum_job(&job.input)
                .map(|s| format!("quantum_{}", s.leg.as_str()))
                .unwrap_or_else(|_| "quantum_train".into());
            if let Some(pack) = self.quantum.as_ref() {
                if let Ok(spec) = crate::parse_quantum_job(&job.input) {
                    score_primary = pack.meta(spec.leg).last_acc;
                }
            }
        } else if leg_job {
            research_scenario = crate::parse_leg_job(&job.input)
                .map(|s| format!("leg_{}", s.leg.as_str()))
                .unwrap_or_else(|_| "leg_train".into());
            if let Some(pack) = self.legs.as_ref() {
                if let Ok(spec) = crate::parse_leg_job(&job.input) {
                    score_primary = pack.meta(spec.leg).last_acc;
                }
            }
        } else if shared_v2 {
            research_scenario = "shared_brain_v2".into();
            if let Some(b) = self.brain_v2.as_ref() {
                score_primary = b.last_acc_q16 as f64 / (1 << 16) as f64;
            }
        } else if shared {
            research_scenario = "shared_brain".into();
            if let Some(b) = self.brain.as_ref() {
                score_primary = b.last_acc;
            }
        }

        if quantum_job {
            let pct = (score_primary * 100.0).round() as i32;
            self.push_quantum_story(
                "worked",
                &research_scenario,
                &format!("Guardian train verified — practice accuracy ~{pct}%"),
            );
        } else if matches!(job.kind, AiJobKind::ProtocolEval)
            && research_scenario.starts_with("quantum_")
        {
            let pct = (score_primary * 100.0).round() as i32;
            if score_primary < 0.4 {
                self.push_quantum_story(
                    "failed",
                    &research_scenario,
                    &format!("Pressure-test scored weak (~{pct}%) — more research needed"),
                );
            } else {
                self.push_quantum_story(
                    "worked",
                    &research_scenario,
                    &format!("Pressure-test scored ~{pct}% — looking healthier"),
                );
            }
        }

        Ok(AiJobReceipt {
            job_id: job.job_id,
            worker: worker_addr,
            input_commitment: job.input_commitment,
            output_hash,
            latency_ms,
            weight,
            verified_at: now_secs(),
            job_kind: job.kind,
            research_scenario,
            score_primary,
            score_orphan_risk,
            score_detect_rate,
            score_linkability,
            score_backlog_ratio,
            score_latency_p95_ms,
        })
    }

    /// Apply several verified completes under one AI lock (Build/27 N9 batch).
    /// Per-item errors: Shared/SharedV2 may return `StaleBrain` after an earlier apply.
    pub fn finish_completes(
        &mut self,
        items: Vec<(VerifiedComplete, u64, u64)>,
    ) -> Vec<Result<AiJobReceipt, OrchError>> {
        let mut out = Vec::with_capacity(items.len());
        for (verified, latency_ms, height) in items {
            match self.finish_complete(verified, latency_ms, height) {
                Ok(r) => out.push(Ok(r)),
                Err(e) => {
                    self.note_verify_fail();
                    out.push(Err(e));
                }
            }
        }
        out
    }

    fn verify_output(&self, job: &PendingJob, output: &[u8]) -> Result<Hash, OrchError> {
        match job.kind {
            AiJobKind::Echo => {
                verify_echo_result(&job.input, output)?;
                Ok(Hash::digest(output))
            }
            AiJobKind::Benchmark => {
                let expected = run_benchmark(&job.input);
                if output.as_ref() != expected.as_slice() {
                    return Err(OrchError::BenchmarkFailed);
                }
                Ok(Hash::from_bytes(expected))
            }
            AiJobKind::ProtocolEval => {
                let expected = run_protocol_eval(&job.input);
                if output.as_ref() != expected.as_slice() {
                    return Err(OrchError::ProtocolEvalFailed);
                }
                Ok(Hash::from_bytes(expected))
            }
            AiJobKind::AgentAssist => {
                let expected = run_agent_assist(&job.input);
                if output != expected {
                    return Err(OrchError::AgentAssistFailed);
                }
                Ok(Hash::digest(&output))
            }
            AiJobKind::MlTrain => {
                let expected = run_ml_train_job(&job.input);
                if output != expected {
                    return Err(OrchError::MlTrainFailed);
                }
                Ok(Hash::digest(&output))
            }
        }
    }
}

fn parse_research_scenario_id(input: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(input).ok()?;
    let rest = s
        .strip_prefix("mesh-research:v2:")
        .or_else(|| s.strip_prefix("mesh-research:v1:"))?;
    let id = rest.split(':').next()?.to_string();
    if ResearchScenario::parse(&id).is_some() {
        Some(id)
    } else {
        None
    }
}

/// Echo job: output must equal input (deterministic verify).
pub fn verify_echo_result(input: &[u8], output: &[u8]) -> Result<(), OrchError> {
    if input == output {
        Ok(())
    } else {
        Err(OrchError::EchoFailed)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_types::Address;

    #[test]
    fn protocol_eval_pays_fixed_gpu_weight() {
        let mut q = JobQueue::default();
        let addr = Address::from_pubkey_bytes(b"eval-worker").to_string();
        q.advertise(Capability {
            address: addr.clone(),
            gpu_name: "test".into(),
            vram_mb: 8,
            kinds: vec!["protocol_eval".into()],
            train_slots: 0,
            brain_backends: vec![],
            brain_contract: String::new(),
            os_family: std::env::consts::OS.into(),
        })
        .unwrap();
        let job = q.enqueue_research(ResearchScenario::SpamRecovery, 10, 0.2);
        let assign = q.take_job(&addr).unwrap();
        assert_eq!(assign.job_id, job.job_id);
        assert_eq!(assign.kind, "protocol_eval");
        let out = hex::encode(run_protocol_eval(&job.input));
        let receipt = q.complete(&addr, &assign.job_id, &out, 5).unwrap();
        assert_eq!(receipt.weight, 100);
        assert_eq!(q.research_scenarios_touched(), 1);
    }

    #[test]
    fn rejects_forged_protocol_eval_digest() {
        let mut q = JobQueue::default();
        let addr = Address::from_pubkey_bytes(b"evil-worker").to_string();
        q.advertise(Capability {
            address: addr.clone(),
            gpu_name: "test".into(),
            vram_mb: 8,
            kinds: vec!["protocol_eval".into()],
            train_slots: 0,
            brain_backends: vec![],
            brain_contract: String::new(),
            os_family: std::env::consts::OS.into(),
        })
        .unwrap();
        q.enqueue_protocol_eval(b"features".to_vec());
        let assign = q.take_job(&addr).unwrap();
        let forged = hex::encode([7u8; 32]);
        assert!(matches!(
            q.complete(&addr, &assign.job_id, &forged, 1),
            Err(OrchError::ProtocolEvalFailed)
        ));
        assert_eq!(q.verify_fail(), 1);
        // Failed jobs are dropped so the queue cannot fill with zombies.
        assert_eq!(q.inflight_len(), 0);
    }

    #[test]
    fn echo_roundtrip() {
        let mut q = JobQueue::default();
        let addr = Address::from_pubkey_bytes(b"echo-worker").to_string();
        q.advertise(Capability {
            address: addr.clone(),
            gpu_name: "test".into(),
            vram_mb: 1,
            kinds: vec!["echo".into()],
            train_slots: 0,
            brain_backends: vec![],
            brain_contract: String::new(),
            os_family: std::env::consts::OS.into(),
        })
        .unwrap();
        q.enqueue_echo(b"hello".to_vec());
        let assign = q.take_job(&addr).unwrap();
        let receipt = q
            .complete(&addr, &assign.job_id, &assign.input_hex, 1)
            .unwrap();
        assert_eq!(receipt.weight, 1);
    }

    #[test]
    fn batch_echo_finish_completes() {
        let mut q = JobQueue::default();
        let addr = Address::from_pubkey_bytes(b"batch-echo").to_string();
        q.advertise(Capability {
            address: addr.clone(),
            gpu_name: "test".into(),
            vram_mb: 1,
            kinds: vec!["echo".into()],
            train_slots: 0,
            brain_backends: vec![],
            brain_contract: String::new(),
            os_family: std::env::consts::OS.into(),
        })
        .unwrap();
        q.enqueue_echo(b"one".to_vec());
        q.enqueue_echo(b"two".to_vec());
        let a1 = q.take_job(&addr).unwrap();
        let a2 = q.take_job(&addr).unwrap();
        let p1 = q
            .prepare_complete(&addr, &a1.job_id, &a1.input_hex)
            .unwrap();
        let p2 = q
            .prepare_complete(&addr, &a2.job_id, &a2.input_hex)
            .unwrap();
        let verified = run_cpu_batch(vec![p1, p2]);
        assert!(verified.iter().all(|r| r.is_ok()));
        let items: Vec<_> = verified
            .into_iter()
            .map(|r| (r.unwrap(), 1u64, 0u64))
            .collect();
        let outs = q.finish_completes(items);
        assert_eq!(outs.len(), 2);
        assert!(outs.iter().all(|r| r.is_ok()));
        assert_eq!(q.verify_ok(), 2);
    }

    #[test]
    fn batch_leg_different_legs() {
        let mut q = JobQueue::with_brains_and_legs(
            SharedBrain::genesis(None),
            SharedBrainV2::genesis(None),
            LegBrainPack::genesis(None),
        );
        let addr = Address::from_pubkey_bytes(b"batch-leg").to_string();
        q.advertise(Capability {
            address: addr.clone(),
            gpu_name: "test".into(),
            vram_mb: 8,
            kinds: vec!["ml_train".into(), "leg_train".into()],
            train_slots: 2,
            brain_backends: vec!["cpu_v1".into()],
            brain_contract: String::new(),
            os_family: std::env::consts::OS.into(),
        })
        .unwrap();
        let j1 = q
            .enqueue_leg_train(LegId::Security, 2, 50, 4, 0)
            .expect("sec leg");
        let j2 = q
            .enqueue_leg_train(LegId::Network, 2, 50, 4, 0)
            .expect("net leg");
        let a1 = q.take_job(&addr).unwrap();
        let a2 = q.take_job(&addr).unwrap();
        assert_eq!(a1.job_id, j1.job_id);
        assert_eq!(a2.job_id, j2.job_id);
        let w_sec = q.legs().unwrap().weights(LegId::Security).to_vec();
        let w_net = q.legs().unwrap().weights(LegId::Network).to_vec();
        let in1 = hex::decode(&a1.input_hex).unwrap();
        let in2 = hex::decode(&a2.input_hex).unwrap();
        let out1 = hex::encode(crate::run_leg_train(&w_sec, &in1).unwrap().output);
        let out2 = hex::encode(crate::run_leg_train(&w_net, &in2).unwrap().output);
        let p1 = q.prepare_complete(&addr, &a1.job_id, &out1).unwrap();
        let p2 = q.prepare_complete(&addr, &a2.job_id, &out2).unwrap();
        let verified = run_cpu_batch(vec![p1, p2]);
        let items: Vec<_> = verified
            .into_iter()
            .map(|r| (r.expect("verify"), 1u64, 0u64))
            .collect();
        let outs = q.finish_completes(items);
        assert!(outs.iter().all(|r| r.is_ok()), "{outs:?}");
        assert_eq!(q.legs().unwrap().epoch(LegId::Security), 1);
        assert_eq!(q.legs().unwrap().epoch(LegId::Network), 1);
    }

    #[test]
    fn batch_shared_second_stale() {
        let mut q = JobQueue::with_brain(SharedBrain::genesis(None));
        let addr = Address::from_pubkey_bytes(b"batch-share").to_string();
        q.advertise(Capability {
            address: addr.clone(),
            gpu_name: "test".into(),
            vram_mb: 8,
            kinds: vec!["ml_train".into()],
            train_slots: 2,
            brain_backends: vec!["cpu_v1".into()],
            brain_contract: String::new(),
            os_family: std::env::consts::OS.into(),
        })
        .unwrap();
        q.enqueue_ml_train_shared(2, 50, 4, 0).unwrap();
        q.enqueue_ml_train_shared(2, 50, 4, 1).unwrap();
        let a1 = q.take_job(&addr).unwrap();
        let a2 = q.take_job(&addr).unwrap();
        let w = q.brain().unwrap().weights.clone();
        let in1 = hex::decode(&a1.input_hex).unwrap();
        let in2 = hex::decode(&a2.input_hex).unwrap();
        let out1 = hex::encode(run_ml_train_shared(&w, &in1).unwrap().output);
        let out2 = hex::encode(run_ml_train_shared(&w, &in2).unwrap().output);
        let p1 = q.prepare_complete(&addr, &a1.job_id, &out1).unwrap();
        let p2 = q.prepare_complete(&addr, &a2.job_id, &out2).unwrap();
        let v1 = p1.run_cpu().unwrap();
        let v2 = p2.run_cpu().unwrap();
        let outs = q.finish_completes(vec![(v1, 1, 0), (v2, 1, 0)]);
        assert!(outs[0].is_ok());
        // Second apply loses the epoch race but still earns partial GPU credit.
        let stale = outs[1].as_ref().expect("raced stale still receipts");
        assert_eq!(stale.weight, 8);
        assert_eq!(q.brain().unwrap().epoch, 1);
    }

    #[test]
    fn windows_worker_skips_f64_shared_gets_protocol() {
        let mut q = JobQueue::with_brains(
            SharedBrain::genesis(None),
            SharedBrainV2::genesis(None),
        );
        let addr = Address::from_pubkey_bytes(b"win-miner").to_string();
        q.advertise(Capability {
            address: addr.clone(),
            gpu_name: "rtx".into(),
            vram_mb: 12_000,
            kinds: vec!["ml_train".into(), "protocol_eval".into()],
            train_slots: 2,
            brain_backends: vec!["cpu_v1".into(), "cuda_v2".into()],
            brain_contract: mesh_ai_v2::BRAIN_CONTRACT.into(),
            // Intentionally mismatch verifier OS so f64 shared is skipped.
            os_family: if std::env::consts::OS == "windows" {
                "linux".into()
            } else {
                "windows".into()
            },
        })
        .unwrap();
        q.enqueue_ml_train_shared(2, 50, 4, 0).unwrap();
        let _ = q.enqueue_research(ResearchScenario::SpamRecovery, 1, 0.1);
        let assign = q.take_job(&addr).unwrap();
        // Must not pull the f64 v1 shared job when OS mismatches the verifier.
        assert_eq!(assign.kind, "protocol_eval");
    }

    #[test]
    fn edge_board_caps_remote_protocol_ingest() {
        let mut q = JobQueue::default(); // no brain → edge-style board
        for i in 0..60 {
            let ok = q.ingest_remote_job(
                format!("eval-{i}"),
                "protocol_eval",
                format!("feat-{i}").into_bytes(),
                Hash::digest(format!("feat-{i}").as_bytes()),
            );
            if i < 48 {
                assert!(ok, "expected ingest at {i}");
            } else {
                assert!(!ok, "expected cap reject at {i}");
            }
        }
        assert_eq!(q.pending_len(), 48);
        // Brainish jobs still "seen" without enqueueing.
        assert!(q.ingest_remote_job(
            "shared-1".into(),
            "ml_train_shared",
            b"x".to_vec(),
            Hash::digest(b"x"),
        ));
        assert_eq!(q.pending_len(), 48);
    }

    #[test]
    fn cuda_only_worker_skips_protocol_gets_v2() {
        let mut q = JobQueue::with_brains(
            SharedBrain::genesis(None),
            SharedBrainV2::genesis(None),
        );
        let addr = Address::from_pubkey_bytes(b"cuda-only").to_string();
        q.advertise(Capability {
            address: addr.clone(),
            gpu_name: "rtx".into(),
            vram_mb: 12_000,
            kinds: vec!["ml_train".into(), "ml_train_shared".into()],
            train_slots: 1,
            brain_backends: vec!["cuda_v2".into()],
            brain_contract: mesh_ai_v2::BRAIN_CONTRACT.into(),
            os_family: std::env::consts::OS.into(),
        })
        .unwrap();
        let _ = q.enqueue_research(ResearchScenario::SpamRecovery, 1, 0.1);
        q.enqueue_ml_train_shared_v2(2, 50, 4, 0).unwrap();
        let assign = q.take_job(&addr).unwrap();
        assert_eq!(assign.kind, "ml_train_shared_v2");
        let input = hex::decode(&assign.input_hex).expect("input hex");
        assert!(is_ml_train_shared_v2(&input));
        assert!(q
            .pending
            .iter()
            .any(|j| matches!(j.kind, AiJobKind::ProtocolEval)));
    }
}
