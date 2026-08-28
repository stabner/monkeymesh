//! MonkeyMind marketplace stub — user jobs → GPU queue → verified result (Build/12).
//!
//! Real LLM/image models come later. v1 maps services to deterministic GPU work
//! (echo / protocol_eval) so payment + routing pipes can be proven end-to-end.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::orchestrator::JobQueue;

/// Max stored marketplace jobs (oldest completed/failed pruned first).
pub const MAX_MARKET_JOBS: usize = 200;
/// Max prompt bytes.
pub const MAX_PROMPT_BYTES: usize = 8_192;
/// Max submits in the rolling window.
pub const RATE_LIMIT_MAX: usize = 30;
/// Rolling window seconds.
pub const RATE_LIMIT_WINDOW_SECS: u64 = 60;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MarketError {
    #[error("prompt empty")]
    EmptyPrompt,
    #[error("prompt too large (max {MAX_PROMPT_BYTES} bytes)")]
    PromptTooLarge,
    #[error("rate limited — try again shortly")]
    RateLimited,
    #[error("marketplace job capacity reached")]
    Capacity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketService {
    Echo,
    Llm,
    Embeddings,
    Image,
    Agent,
}

impl MarketService {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "echo" => Some(Self::Echo),
            "llm" | "inference" => Some(Self::Llm),
            "embeddings" | "embed" => Some(Self::Embeddings),
            "image" | "img" => Some(Self::Image),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Echo => "echo",
            Self::Llm => "llm",
            Self::Embeddings => "embeddings",
            Self::Image => "image",
            Self::Agent => "agent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketJobStatus {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementStatus {
    #[default]
    None,
    Pending,
    Paid,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketJob {
    pub id: String,
    pub service: String,
    pub prompt: String,
    pub status: MarketJobStatus,
    /// Internal worker queue job id.
    pub worker_job_id: String,
    pub worker: Option<String>,
    pub output_hash: Option<String>,
    pub output_hex: Option<String>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
    pub error: Option<String>,
    pub note: String,
    #[serde(default)]
    pub settlement_status: SettlementStatus,
    pub settlement_amount: Option<String>,
    pub settlement_txid: Option<String>,
    pub settlement_error: Option<String>,
}

pub struct Marketplace {
    jobs: HashMap<String, MarketJob>,
    next_id: u64,
    /// Submit timestamps for rate limiting.
    submit_times: Vec<u64>,
    pub rate_limit_max: usize,
    pub rate_limit_window_secs: u64,
    pub max_jobs: usize,
}

impl Default for Marketplace {
    fn default() -> Self {
        Self {
            jobs: HashMap::new(),
            next_id: 0,
            submit_times: Vec::new(),
            rate_limit_max: RATE_LIMIT_MAX,
            rate_limit_window_secs: RATE_LIMIT_WINDOW_SECS,
            max_jobs: MAX_MARKET_JOBS,
        }
    }
}

impl Marketplace {
    pub fn submit(
        &mut self,
        queue: &mut JobQueue,
        service: MarketService,
        prompt: String,
    ) -> Result<MarketJob, MarketError> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(MarketError::EmptyPrompt);
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(MarketError::PromptTooLarge);
        }
        let now = now_secs();
        self.prune_rate_window(now);
        if self.submit_times.len() >= self.rate_limit_max {
            return Err(MarketError::RateLimited);
        }
        self.prune_capacity();
        if self.jobs.len() >= self.max_jobs {
            return Err(MarketError::Capacity);
        }

        self.next_id = self.next_id.saturating_add(1);
        let id = format!("mkt-{}", self.next_id);
        let input = format!("{}:{}", service.as_str(), prompt).into_bytes();

        let pending = match service {
            MarketService::Echo | MarketService::Embeddings => queue.enqueue_echo(input),
            MarketService::Llm | MarketService::Image | MarketService::Agent => {
                queue.enqueue_protocol_eval(input)
            }
        };

        let job = MarketJob {
            id: id.clone(),
            service: service.as_str().into(),
            prompt,
            status: MarketJobStatus::Queued,
            worker_job_id: pending.job_id,
            worker: None,
            output_hash: None,
            output_hex: None,
            created_at: now,
            completed_at: None,
            error: None,
            note: "stub GPU work; GPU score receipts + optional hot-wallet MESH settle".into(),
            settlement_status: SettlementStatus::None,
            settlement_amount: None,
            settlement_txid: None,
            settlement_error: None,
        };
        self.submit_times.push(now);
        self.jobs.insert(id, job.clone());
        Ok(job)
    }

    fn prune_rate_window(&mut self, now: u64) {
        let cut = now.saturating_sub(self.rate_limit_window_secs);
        self.submit_times.retain(|t| *t >= cut);
    }

    /// Drop oldest terminal jobs when near capacity.
    fn prune_capacity(&mut self) {
        if self.jobs.len() < self.max_jobs {
            return;
        }
        let mut terminal: Vec<(String, u64)> = self
            .jobs
            .iter()
            .filter(|(_, j)| {
                matches!(
                    j.status,
                    MarketJobStatus::Done | MarketJobStatus::Failed
                )
            })
            .map(|(id, j)| (id.clone(), j.completed_at.unwrap_or(j.created_at)))
            .collect();
        terminal.sort_by_key(|(_, t)| *t);
        for (id, _) in terminal.into_iter().take(self.jobs.len().saturating_sub(self.max_jobs / 2))
        {
            self.jobs.remove(&id);
            if self.jobs.len() < self.max_jobs {
                break;
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&MarketJob> {
        self.jobs.get(id)
    }

    pub fn get_by_worker_job(&self, worker_job_id: &str) -> Option<&MarketJob> {
        self.jobs.values().find(|j| j.worker_job_id == worker_job_id)
    }

    pub fn get_by_worker_job_mut(&mut self, worker_job_id: &str) -> Option<&mut MarketJob> {
        self.jobs
            .values_mut()
            .find(|j| j.worker_job_id == worker_job_id)
    }

    pub fn list(&self) -> Vec<&MarketJob> {
        let mut v: Vec<_> = self.jobs.values().collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// When a worker takes a job, mark marketplace entry running if linked.
    pub fn on_worker_assigned(&mut self, worker_job_id: &str, worker: &str) {
        if let Some(j) = self.get_by_worker_job_mut(worker_job_id) {
            j.status = MarketJobStatus::Running;
            j.worker = Some(worker.to_string());
        }
    }

    pub fn on_worker_result(
        &mut self,
        worker_job_id: &str,
        worker: &str,
        output_hash: &str,
        output_hex: &str,
        ok: bool,
        err: Option<String>,
    ) -> Option<String> {
        let j = self.get_by_worker_job_mut(worker_job_id)?;
        j.worker = Some(worker.to_string());
        j.completed_at = Some(now_secs());
        if ok {
            j.status = MarketJobStatus::Done;
            j.output_hash = Some(output_hash.to_string());
            j.output_hex = Some(output_hex.to_string());
            j.error = None;
            if matches!(
                j.settlement_status,
                SettlementStatus::None | SettlementStatus::Failed
            ) {
                j.settlement_status = SettlementStatus::Pending;
            }
            Some(j.id.clone())
        } else {
            j.status = MarketJobStatus::Failed;
            j.error = err.or_else(|| Some("worker failed".into()));
            j.settlement_status = SettlementStatus::Skipped;
            Some(j.id.clone())
        }
    }

    pub fn mark_settled(
        &mut self,
        market_job_id: &str,
        amount: &str,
        txid: &str,
    ) -> bool {
        let Some(j) = self.jobs.get_mut(market_job_id) else {
            return false;
        };
        if j.settlement_txid.is_some() {
            return true; // idempotent
        }
        j.settlement_status = SettlementStatus::Paid;
        j.settlement_amount = Some(amount.to_string());
        j.settlement_txid = Some(txid.to_string());
        j.settlement_error = None;
        true
    }

    pub fn mark_settle_failed(&mut self, market_job_id: &str, err: &str) -> bool {
        let Some(j) = self.jobs.get_mut(market_job_id) else {
            return false;
        };
        if j.settlement_txid.is_some() {
            return true;
        }
        j.settlement_status = SettlementStatus::Failed;
        j.settlement_error = Some(err.to_string());
        true
    }

    pub fn mark_settle_skipped(&mut self, market_job_id: &str, reason: &str) -> bool {
        let Some(j) = self.jobs.get_mut(market_job_id) else {
            return false;
        };
        if j.settlement_txid.is_some() {
            return true;
        }
        j.settlement_status = SettlementStatus::Skipped;
        j.settlement_error = Some(reason.to_string());
        true
    }
}

/// Format atomic MESH units as `"0.01000000"` (no suffix) for `/v1/sendtoaddress`.
pub fn format_settle_amount(atomic: u64) -> String {
    let whole = atomic / 100_000_000;
    let frac = atomic % 100_000_000;
    format!("{whole}.{frac:08}")
}

/// Weight-scaled settlement: `base_atomic * weight`.
pub fn settle_amount_for_weight(base_atomic: u64, weight: u64) -> String {
    format_settle_amount(base_atomic.saturating_mul(weight.max(1)))
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
    use crate::orchestrator::{Capability, JobQueue};
    use mesh_types::Address;

    fn worker_q() -> (JobQueue, String) {
        let mut q = JobQueue::default();
        let addr = Address::from_pubkey_bytes(b"mkt-worker").to_string();
        q.advertise(Capability {
            address: addr.clone(),
            gpu_name: "t".into(),
            vram_mb: 1,
            kinds: vec!["echo".into(), "protocol_eval".into()],
            train_slots: 0,
            brain_backends: vec![],
            brain_contract: String::new(),
            os_family: std::env::consts::OS.into(),
        })
        .unwrap();
        (q, addr)
    }

    #[test]
    fn submit_and_complete_echo_market_job() {
        let mut m = Marketplace::default();
        let (mut q, addr) = worker_q();

        let job = m
            .submit(&mut q, MarketService::Echo, "hello".into())
            .unwrap();
        assert_eq!(job.status, MarketJobStatus::Queued);
        let assign = q.take_job(&addr).unwrap();
        m.on_worker_assigned(&assign.job_id, &addr);
        let receipt = q
            .complete(&addr, &assign.job_id, &assign.input_hex, 2)
            .unwrap();
        let mid = m
            .on_worker_result(
                &assign.job_id,
                &addr,
                &receipt.output_hash.to_string(),
                &assign.input_hex,
                true,
                None,
            )
            .unwrap();
        assert_eq!(mid, job.id);
        let done = m.get(&job.id).unwrap();
        assert_eq!(done.status, MarketJobStatus::Done);
        assert_eq!(done.settlement_status, SettlementStatus::Pending);
        m.mark_settled(&job.id, "0.00100000", "deadbeef");
        let paid = m.get(&job.id).unwrap();
        assert_eq!(paid.settlement_status, SettlementStatus::Paid);
        assert_eq!(paid.settlement_txid.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn settle_amount_scales_with_weight() {
        assert_eq!(settle_amount_for_weight(100_000, 1), "0.00100000");
        assert_eq!(settle_amount_for_weight(100_000, 25), "0.02500000");
    }

    #[test]
    fn rejects_empty_and_huge_prompt() {
        let mut m = Marketplace::default();
        let mut q = JobQueue::default();
        assert_eq!(
            m.submit(&mut q, MarketService::Echo, "  ".into())
                .unwrap_err(),
            MarketError::EmptyPrompt
        );
        let big = "x".repeat(MAX_PROMPT_BYTES + 1);
        assert_eq!(
            m.submit(&mut q, MarketService::Llm, big).unwrap_err(),
            MarketError::PromptTooLarge
        );
    }

    #[test]
    fn rate_limit_blocks_burst() {
        let mut m = Marketplace::default();
        m.rate_limit_max = 3;
        m.rate_limit_window_secs = 600;
        let mut q = JobQueue::default();
        for i in 0..3 {
            m.submit(&mut q, MarketService::Echo, format!("p{i}"))
                .unwrap();
        }
        assert_eq!(
            m.submit(&mut q, MarketService::Echo, "nope".into())
                .unwrap_err(),
            MarketError::RateLimited
        );
    }

    #[test]
    fn llm_maps_to_protocol_eval_weight() {
        let mut m = Marketplace::default();
        let (mut q, addr) = worker_q();
        let job = m
            .submit(&mut q, MarketService::Llm, "hi".into())
            .unwrap();
        let assign = q.take_job(&addr).unwrap();
        assert_eq!(assign.kind, "protocol_eval");
        assert_eq!(assign.job_id, job.worker_job_id);
        let input = hex::decode(&assign.input_hex).unwrap();
        let out = hex::encode(crate::work::run_protocol_eval(&input));
        let receipt = q.complete(&addr, &assign.job_id, &out, 3).unwrap();
        assert_eq!(receipt.weight, 100);
    }
}
