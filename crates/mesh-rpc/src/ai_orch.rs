//! Built-in AI job board on the node (workers connect here — no separate orch required).
//!
//! Compatible worker API: `/v1/advertise`, `/v1/job`, `/v1/result`, `/v1/results`.
//! Enqueues protocol sims + **shared-brain** MNIST training (v1 + soft-gated v2).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Last time take_job topped up an empty board (ms since epoch).
static LAST_EMPTY_TOPUP_MS: AtomicU64 = AtomicU64::new(0);

fn should_empty_topup() -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let prev = LAST_EMPTY_TOPUP_MS.load(Ordering::Relaxed);
    if now.saturating_sub(prev) < 800 {
        return false;
    }
    LAST_EMPTY_TOPUP_MS.store(now, Ordering::Relaxed);
    true
}

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use mesh_ai::{brain_verify_batch_max, Capability, ResearchScenario};
use serde::Deserialize;

use crate::RpcState;

/// Board mutate limits (per 60s window) — Build/27 B3 (env-overridable).
fn ai_limit_env(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

fn ai_adv_limit() -> u32 {
    ai_limit_env("MESH_AI_ADV_LIMIT", 12)
}
fn ai_job_limit() -> u32 {
    ai_limit_env("MESH_AI_JOB_LIMIT", 60)
}
fn ai_res_limit() -> u32 {
    ai_limit_env("MESH_AI_RES_LIMIT", 90)
}
fn ai_global_limit() -> u32 {
    ai_limit_env("MESH_AI_GLOBAL_LIMIT", 2_000)
}
fn ai_ip_limit() -> u32 {
    ai_limit_env("MESH_AI_IP_LIMIT", 300)
}

const ABS_MAX_QUEUE_DEPTH: usize = 128;

/// Non-brain job board routes (safe to host on edge shards).
pub fn ai_board_router() -> Router<RpcState> {
    Router::new()
        .route("/v1/advertise", post(advertise))
        .route("/v1/job", post(take_job))
        .route("/v1/result", post(submit_result))
        .route("/v1/results", post(submit_results))
        .route("/v1/ai/health", get(ai_health))
        .route("/v1/workers", get(list_workers))
        .route("/v1/research/status", get(research_status))
        .route("/v1/research/scenarios", get(research_scenarios))
}

/// Shared-brain / trilemma guardian routes (seed-canonical).
pub fn ai_brain_router() -> Router<RpcState> {
    Router::new()
        .route("/v1/model", get(get_model))
        .route("/v1/model/meta", get(get_model_meta))
        .route("/v1/model/bin", get(get_model_bin))
        .route("/v1/trilemma", get(get_trilemma))
        .route("/v1/quantum", get(get_quantum))
        .route("/v1/leg/{leg}", get(get_leg_model))
        .route("/v1/leg/{leg}/meta", get(get_leg_meta))
        .route("/v1/leg/{leg}/bin", get(get_leg_bin))
        .route("/v1/qleg/{leg}", get(get_qleg_model))
        .route("/v1/qleg/{leg}/meta", get(get_qleg_meta))
        .route("/v1/qleg/{leg}/bin", get(get_qleg_bin))
}

pub fn ai_router() -> Router<RpcState> {
    Router::new()
        .merge(ai_board_router())
        .merge(ai_brain_router())
}

pub async fn spawn_research_tick(state: RpcState) {
    tokio::spawn(async move {
        loop {
            let (workers, depth) = {
                let q = state.ai.lock().await;
                (q.workers().count(), q.queue_depth())
            };
            // Even drip: never refill faster than ~0.5s while workers are present.
            let wait = if workers > 0 {
                if depth == 0 {
                    std::time::Duration::from_millis(450)
                } else {
                    std::time::Duration::from_millis(700)
                }
            } else {
                std::time::Duration::from_secs(8)
            };
            tokio::time::sleep(wait).await;
            research_tick(&state).await;
        }
    });
}

/// Mirror gossiped AI jobs into the local queue (durable buffer + cursor + queue snap).
pub fn spawn_ai_inbound(state: RpcState) {
    let data_dir = state
        .wallet_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    let wal_path = data_dir.join("ai_inbound.wal");
    let cursor_path = data_dir.join("ai_inbound.cursor");
    let snap_path = data_dir.join("ai_queue.snap");

    let Some(net) = state.network.clone() else {
        spawn_ai_queue_persist(state, snap_path);
        return;
    };

    let mut rx = net.subscribe_ai_jobs();
    let buf: Arc<std::sync::Mutex<std::collections::VecDeque<(u64, mesh_p2p::InboundAiJob)>>> =
        Arc::new(std::sync::Mutex::new(std::collections::VecDeque::with_capacity(
            1024,
        )));
    let next_seq = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let cursor = Arc::new(std::sync::atomic::AtomicU64::new(read_ai_cursor(&cursor_path)));

    // Reload undrained WAL lines (seq > cursor); support legacy unsequenced lines.
    let mut max_seq = cursor.load(std::sync::atomic::Ordering::Relaxed);
    if let Ok(text) = std::fs::read_to_string(&wal_path) {
        let mut g = buf.lock().unwrap_or_else(|e| e.into_inner());
        for line in text.lines().take(8_192) {
            if let Ok(rec) = serde_json::from_str::<WalAiRec>(line) {
                if rec.seq > cursor.load(std::sync::atomic::Ordering::Relaxed) {
                    max_seq = max_seq.max(rec.seq);
                    g.push_back((rec.seq, rec.job.into_inbound()));
                }
            } else if let Ok(job) = serde_json::from_str::<WalAiJob>(line) {
                // Legacy line — assign synthetic seq after cursor.
                max_seq = max_seq.saturating_add(1);
                g.push_back((max_seq, job.into_inbound()));
            }
        }
        if !g.is_empty() {
            tracing::info!(n = g.len(), path = %wal_path.display(), "reloaded durable AI inbound");
        }
    }
    next_seq.store(max_seq.saturating_add(1), std::sync::atomic::Ordering::Relaxed);

    let buf_rx = buf.clone();
    let wal_rx = wal_path.clone();
    let next_rx = next_seq.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(job) => {
                    let seq = next_rx.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    {
                        let mut g = buf_rx.lock().unwrap_or_else(|e| e.into_inner());
                        while g.len() >= 8_192 {
                            g.pop_front();
                        }
                        g.push_back((seq, job.clone()));
                    }
                    append_ai_wal(&wal_rx, seq, &job);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "AI inbound broadcast lagged — durable buffer still drains");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let buf_c = buf;
    let wal_c = wal_path;
    let cursor_c = cursor_path;
    let cursor_v = cursor;
    let state_drain = state.clone();
    let snap_drain = snap_path.clone();
    tokio::spawn(async move {
        restore_ai_queue_snap(&state_drain, &snap_drain).await;
        let mut processed_since_compact = 0u64;
        loop {
            let job = {
                let mut g = buf_c.lock().unwrap_or_else(|e| e.into_inner());
                g.pop_front()
            };
            if let Some((seq, job)) = job {
                {
                    let mut q = state_drain.ai.lock().await;
                    let _ = q.ingest_remote_job(
                        job.job_id,
                        &job.kind,
                        job.payload,
                        job.input_commitment,
                    );
                }
                cursor_v.store(seq, std::sync::atomic::Ordering::Relaxed);
                write_ai_cursor(&cursor_c, seq);
                processed_since_compact = processed_since_compact.saturating_add(1);
                if processed_since_compact >= 64 {
                    compact_ai_wal(
                        &wal_c,
                        &buf_c,
                        cursor_v.load(std::sync::atomic::Ordering::Relaxed),
                    );
                    processed_since_compact = 0;
                }
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
    });

    spawn_ai_queue_persist(state, snap_path);
}

async fn restore_ai_queue_snap(state: &RpcState, snap_path: &std::path::Path) {
    let Ok(bytes) = std::fs::read(snap_path) else {
        return;
    };
    let Ok(snap) = serde_json::from_slice::<mesh_ai::DurableQueueSnap>(&bytes) else {
        return;
    };
    let n = snap.pending.len() + snap.inflight.len();
    let mut q = state.ai.lock().await;
    q.import_durable(snap);
    if n > 0 {
        tracing::info!(n, path = %snap_path.display(), "restored AI queue snap");
    }
}

fn spawn_ai_queue_persist(state: RpcState, snap_path: std::path::PathBuf) {
    tokio::spawn(async move {
        restore_ai_queue_snap(&state, &snap_path).await;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let snap = {
                let q = state.ai.lock().await;
                if q.pending_len() == 0 && q.inflight_len() == 0 {
                    continue;
                }
                q.export_durable()
            };
            if let Ok(bytes) = serde_json::to_vec(&snap) {
                let tmp = snap_path.with_extension("snap.tmp");
                if std::fs::write(&tmp, &bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, &snap_path);
                }
            }
        }
    });
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct WalAiJob {
    job_id: String,
    kind: String,
    input_commitment: String,
    worker_hint: String,
    payload_hex: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct WalAiRec {
    seq: u64,
    #[serde(flatten)]
    job: WalAiJob,
}

impl WalAiJob {
    fn from_inbound(j: &mesh_p2p::InboundAiJob) -> Self {
        Self {
            job_id: j.job_id.clone(),
            kind: j.kind.clone(),
            input_commitment: j.input_commitment.to_string(),
            worker_hint: j.worker_hint.clone(),
            payload_hex: hex::encode(&j.payload),
        }
    }

    fn into_inbound(self) -> mesh_p2p::InboundAiJob {
        mesh_p2p::InboundAiJob {
            job_id: self.job_id,
            kind: self.kind,
            input_commitment: mesh_types::Hash::from_hex(&self.input_commitment)
                .unwrap_or_else(|_| mesh_types::Hash::zero()),
            worker_hint: self.worker_hint,
            payload: hex::decode(&self.payload_hex).unwrap_or_default(),
        }
    }
}

fn append_ai_wal(path: &std::path::Path, seq: u64, job: &mesh_p2p::InboundAiJob) {
    use std::io::Write;
    let rec = WalAiRec {
        seq,
        job: WalAiJob::from_inbound(job),
    };
    if let Ok(line) = serde_json::to_string(&rec) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{line}");
            if seq % 32 == 0 {
                let _ = f.sync_data();
            }
        }
    }
}

fn read_ai_cursor(path: &std::path::Path) -> u64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn write_ai_cursor(path: &std::path::Path, seq: u64) {
    let tmp = path.with_extension("cursor.tmp");
    if std::fs::write(&tmp, seq.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn compact_ai_wal(
    path: &std::path::Path,
    buf: &std::sync::Mutex<std::collections::VecDeque<(u64, mesh_p2p::InboundAiJob)>>,
    cursor: u64,
) {
    let g = buf.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = String::new();
    for (seq, job) in g.iter() {
        if *seq <= cursor {
            continue;
        }
        let rec = WalAiRec {
            seq: *seq,
            job: WalAiJob::from_inbound(job),
        };
        if let Ok(line) = serde_json::to_string(&rec) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    drop(g);
    let tmp = path.with_extension("wal.tmp");
    if std::fs::write(&tmp, out).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn announce_job(st: &RpcState, job: &mesh_ai::PendingJob) {
    let Some(net) = &st.network else {
        return;
    };
    net.announce_ai_job(
        job.job_id.clone(),
        mesh_ai::JobQueue::wire_kind(job),
        job.input_commitment,
        String::new(),
        job.input.clone(),
    );
}

async fn ai_health(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let q = st.ai.lock().await;
    let brain = q.brain().map(|b| b.meta());
    let brain_v2 = q.brain_v2().map(|b| b.meta());
    let (shard_id, shard_count) = mesh_types::local_ai_shard_config();
    let shard_urls = mesh_types::ai_shard_urls(shard_count);
    let has_brain = brain.is_some();
    let edge_local = !has_brain;
    let board_warm = q.queue_depth() > 0 || q.workers().next().is_some();
    let shard_strict = shard_strict_enabled(board_warm, edge_local);
    Json(serde_json::json!({
        "ok": true,
        "service": "mesh-node-ai",
        "embedded_orchestrator": true,
        "edge_local_board": edge_local,
        "shard_strict": shard_strict,
        "board_warm": board_warm,
        "shared_brain": has_brain,
        "shared_brain_v2": brain_v2.is_some(),
        "ml_train": true,
        "ml_dataset": "MNIST-4096 (official training subset)",
        "protocol_research": true,
        "self_tune": has_brain,
        "completed": q.completed(),
        "pending": q.pending_len(),
        "inflight": q.inflight_len(),
        "verify_ok": q.verify_ok(),
        "verify_fail": q.verify_fail(),
        "brain": brain,
        "brain_v2": brain_v2,
        "quantum_guardians": q.quantum().map(|p| p.all_meta()),
        "worker_slots": q.total_train_slots(),
        "worker_vram_mb": q.total_vram_mb(),
        "target_queue_depth": q.target_queue_depth(1000),
        "ai_shard_id": shard_id,
        "ai_shard_count": shard_count,
        "ai_shard_urls": shard_urls,
        "ai_upstream": st.ai_upstream,
        "auth_ai": st.ai_token.is_some(),
        "durable_inbound": true,
        "durable_queue_snap": true,
        "rate_limits": {
            "advertise_per_worker_60s": ai_adv_limit(),
            "job_per_worker_60s": ai_job_limit(),
            "result_per_worker_60s": ai_res_limit(),
            "global_board_60s": ai_global_limit(),
            "ip_board_60s": ai_ip_limit(),
        },
        "note": if has_brain {
            "seed-canonical board + shared brain"
        } else {
            "edge local non-brain board; brain routes via MESH_AI_UPSTREAM"
        },
    }))
}

#[derive(Deserialize, Default)]
struct ModelQuery {
    #[serde(default)]
    ver: Option<u32>,
}

async fn get_model_meta(
    State(st): State<RpcState>,
    Query(qparams): Query<ModelQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let q = st.ai.lock().await;
    let ver = qparams.ver.unwrap_or(1);
    if ver == 2 {
        let b = q
            .brain_v2()
            .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no shared brain v2".into()))?;
        let m = b.meta();
        return Ok(Json(serde_json::json!({
            "epoch": m.epoch,
            "digest_hex": m.digest_hex,
            "updated_height": m.updated_height,
            "train_steps_total": m.train_steps_total,
            "last_loss_q16": m.last_loss_q16,
            "last_acc_q16": m.last_acc_q16,
            "advances": m.advances,
            "contract": m.contract,
            "ver": m.ver,
            "note": "Shared brain v2 — Q16 mlp512 (Build/24)",
        })));
    }
    let b = q
        .brain()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no shared brain".into()))?;
    let m = b.meta();
    Ok(Json(serde_json::json!({
        "epoch": m.epoch,
        "digest_hex": m.digest_hex,
        "updated_height": m.updated_height,
        "train_steps_total": m.train_steps_total,
        "last_loss": m.last_loss,
        "last_acc": m.last_acc,
        "advances": m.advances,
        "contract": "v1",
        "ver": 1,
        "note": "One shared network brain — all workers train this model",
    })))
}

async fn get_model(
    State(st): State<RpcState>,
    Query(qparams): Query<ModelQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let q = st.ai.lock().await;
    let ver = qparams.ver.unwrap_or(1);
    if ver == 2 {
        let b = q
            .brain_v2()
            .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no shared brain v2".into()))?;
        let m = b.meta();
        return Ok(Json(serde_json::json!({
            "epoch": m.epoch,
            "digest_hex": m.digest_hex,
            "updated_height": m.updated_height,
            "train_steps_total": m.train_steps_total,
            "last_loss_q16": m.last_loss_q16,
            "last_acc_q16": m.last_acc_q16,
            "advances": m.advances,
            "contract": m.contract,
            "ver": m.ver,
            "weights_hex": hex::encode(&b.weights),
            "weights_bin": "/v1/model/bin?ver=2",
        })));
    }
    let b = q
        .brain()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no shared brain".into()))?;
    let m = b.meta();
    Ok(Json(serde_json::json!({
        "epoch": m.epoch,
        "digest_hex": m.digest_hex,
        "updated_height": m.updated_height,
        "train_steps_total": m.train_steps_total,
        "last_loss": m.last_loss,
        "last_acc": m.last_acc,
        "advances": m.advances,
        "contract": "v1",
        "ver": 1,
        "weights_hex": hex::encode(&b.weights),
        "weights_bin": "/v1/model/bin?ver=1",
    })))
}

/// Raw weight bytes (half the wire size of hex JSON) — preferred by GPU workers.
async fn get_model_bin(
    State(st): State<RpcState>,
    Query(qparams): Query<ModelQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let q = st.ai.lock().await;
    let ver = qparams.ver.unwrap_or(1);
    let (epoch, digest, weights) = if ver == 2 {
        let b = q
            .brain_v2()
            .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no shared brain v2".into()))?;
        let m = b.meta();
        (m.epoch, m.digest_hex, b.weights.clone())
    } else {
        let b = q
            .brain()
            .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no shared brain".into()))?;
        let m = b.meta();
        (m.epoch, m.digest_hex, b.weights.clone())
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        "x-mesh-epoch",
        HeaderValue::from_str(&epoch.to_string()).unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    headers.insert(
        "x-mesh-digest",
        HeaderValue::from_str(&digest).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert(
        "x-mesh-ver",
        HeaderValue::from_str(&ver.to_string()).unwrap_or_else(|_| HeaderValue::from_static("1")),
    );
    Ok((StatusCode::OK, headers, weights))
}

fn live_trilemma(st_ai: &mesh_ai::JobQueue, pulse: &mesh_ai::MeshPulse) -> mesh_ai::TrilemmaBoard {
    let scores = &pulse.markets.research_scores;
    let (epochs, smart) = match st_ai.legs() {
        Some(p) => (p.epochs(), p.smart()),
        None => (mesh_ai::LegEpochs::default(), mesh_ai::LegSmart::default()),
    };
    let nodes = 1u32; // seed-local; enriched by worker count below
    let workers = st_ai.workers().count() as u32;
    mesh_ai::build_trilemma_board(
        scores.mean_detect_rate,
        scores.mean_orphan_risk,
        scores.mean_backlog_ratio,
        scores.mean_latency_p95_ms,
        scores.mean_linkability,
        scores.mean_primary,
        nodes.max(1),
        workers,
        epochs,
        smart,
    )
}

async fn get_trilemma(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let pulse = {
        let c = st.chain.lock().await;
        let gpu_w: u64 = c.store().gpu_scores().values().sum();
        let node_w: u64 = c.store().node_scores().values().sum();
        mesh_ai::build_mesh_pulse(
            c.height(),
            c.tip_hash().to_string(),
            gpu_w,
            node_w,
            c.store().ai_receipts(),
        )
    };
    let q = st.ai.lock().await;
    let board = live_trilemma(&q, &pulse);
    let legs = q.legs().map(|p| p.all_meta());
    Json(serde_json::json!({
        "board": board,
        "legs": legs,
        "note": board.note,
    }))
}

fn live_quantum(
    st_ai: &mesh_ai::JobQueue,
    pulse: &mesh_ai::MeshPulse,
    receipts: &[mesh_types::AiJobReceipt],
) -> mesh_ai::QuantumBoard {
    let inputs = mesh_ai::quantum_score_inputs(receipts, pulse.height, pulse.gpu_vs_height_signal);
    let (epochs, smart) = match st_ai.quantum() {
        Some(p) => (p.epochs(), p.smart()),
        None => (mesh_ai::QuantumEpochs::default(), mesh_ai::QuantumSmart::default()),
    };
    let mut board = mesh_ai::build_quantum_board(
        inputs.pqc_primary,
        inputs.pqc_detect,
        inputs.grover_primary,
        inputs.grover_orphan,
        inputs.grover_backlog,
        inputs.harvest_primary,
        inputs.harvest_linkability,
        epochs,
        smart,
    );
    board.note = format!(
        "{} — feed weakest={}; readiness is the min needle",
        inputs.honesty, board.weakest
    );
    board
}

async fn get_quantum(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let (pulse, receipts, gate) = {
        let c = st.chain.lock().await;
        let gpu_w: u64 = c.store().gpu_scores().values().sum();
        let node_w: u64 = c.store().node_scores().values().sum();
        let receipts = c.store().ai_receipts().to_vec();
        let pulse = mesh_ai::build_mesh_pulse(
            c.height(),
            c.tip_hash().to_string(),
            gpu_w,
            node_w,
            &receipts,
        );
        let gate = serde_json::json!({
            "grover_eval_count": c.grover_eval_count(),
            "last_retarget_adapt_grover_count": c.last_retarget_adapt_grover_count(),
            "grover_certs_since_retarget_adapt": c.grover_certs_since_retarget_adapt(),
            "min_grover_certs_for_retarget": mesh_types::MIN_GROVER_CERTS_FOR_RETARGET,
            "retarget": {
                "interval": c.retarget_params().interval,
                "step": c.retarget_params().step,
                "min_floor": c.retarget_params().min_floor,
            },
            "note": "Build/30: ≥5 new quantum_grover certificates can move bounded retarget knobs",
        });
        (pulse, receipts, gate)
    };
    let q = st.ai.lock().await;
    let board = live_quantum(&q, &pulse, &receipts);
    let legs = q.quantum().map(|p| p.all_meta());
    let inputs = mesh_ai::quantum_score_inputs(&receipts, pulse.height, pulse.gpu_vs_height_signal);
    let activity = q.quantum_activity();
    let beats = q.quantum_story();
    let mut trying: Vec<String> = activity
        .iter()
        .map(|a| {
            format!(
                "{} · {}",
                if a.phase == "running" {
                    "Running now"
                } else {
                    "Queued"
                },
                a.detail
            )
        })
        .collect();
    if trying.is_empty() {
        if board.weakest == "pqc" {
            trying.push("Next focus: practice post-quantum (PQC) readiness".into());
        } else if board.weakest == "grover" {
            trying.push("Next focus: practice PoW vs Grover-style search pressure".into());
        } else {
            trying.push("Next focus: practice long-term secrecy / harvest-now risk".into());
        }
    }
    let mut worked: Vec<String> = beats
        .iter()
        .rev()
        .filter(|b| b.outcome == "worked")
        .take(6)
        .map(|b| b.detail.clone())
        .collect();
    if worked.is_empty() {
        if let Some(legs_meta) = legs.as_ref() {
            for m in legs_meta {
                if m.advances > 0 {
                    worked.push(format!(
                        "{} guardian at epoch {} ({} verified advances)",
                        m.leg, m.epoch, m.advances
                    ));
                }
            }
        }
    }
    let failed: Vec<String> = beats
        .iter()
        .rev()
        .filter(|b| b.outcome == "failed")
        .take(6)
        .map(|b| b.detail.clone())
        .collect();
    let headline = format!(
        "Quantum readiness {}/100 — weakest leg is {}{}",
        board.readiness,
        board.weakest,
        if inputs.from_receipts {
            ""
        } else {
            " (provisional scores)"
        }
    );
    Json(serde_json::json!({
        "board": board,
        "legs": legs,
        "note": board.note,
        "honesty": inputs.honesty,
        "protocol": inputs.protocol,
        "activity": activity,
        "self_evolution": gate,
        "story": {
            "headline": headline,
            "trying": trying,
            "worked": worked,
            "failed": failed,
            "focus": board.weakest,
            "from_receipts": inputs.from_receipts,
            "beats": beats,
        },
    }))
}

async fn get_leg_meta(
    State(st): State<RpcState>,
    Path(leg): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let id = mesh_ai::LegId::parse(&leg)
        .ok_or((StatusCode::BAD_REQUEST, "unknown leg".into()))?;
    let q = st.ai.lock().await;
    let pack = q
        .legs()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no leg brains".into()))?;
    let m = pack.meta(id);
    Ok(Json(serde_json::json!(m)))
}

async fn get_leg_model(
    State(st): State<RpcState>,
    Path(leg): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let id = mesh_ai::LegId::parse(&leg)
        .ok_or((StatusCode::BAD_REQUEST, "unknown leg".into()))?;
    let q = st.ai.lock().await;
    let pack = q
        .legs()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no leg brains".into()))?;
    let m = pack.meta(id);
    Ok(Json(serde_json::json!({
        "leg": m.leg,
        "epoch": m.epoch,
        "digest_hex": m.digest_hex,
        "last_loss": m.last_loss,
        "last_acc": m.last_acc,
        "advances": m.advances,
        "train_steps_total": m.train_steps_total,
        "weights_hex": hex::encode(pack.weights(id)),
        "weights_bin": format!("/v1/leg/{leg}/bin"),
    })))
}

async fn get_leg_bin(
    State(st): State<RpcState>,
    Path(leg): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let id = mesh_ai::LegId::parse(&leg)
        .ok_or((StatusCode::BAD_REQUEST, "unknown leg".into()))?;
    let q = st.ai.lock().await;
    let pack = q
        .legs()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no leg brains".into()))?;
    let m = pack.meta(id);
    let weights = pack.weights(id).to_vec();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        "x-mesh-epoch",
        HeaderValue::from_str(&m.epoch.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    headers.insert(
        "x-mesh-digest",
        HeaderValue::from_str(&m.digest_hex).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert(
        "x-mesh-leg",
        HeaderValue::from_str(&m.leg).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    Ok((StatusCode::OK, headers, weights))
}

async fn get_qleg_meta(
    State(st): State<RpcState>,
    Path(leg): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let id = mesh_ai::QuantumId::parse(&leg)
        .ok_or((StatusCode::BAD_REQUEST, "unknown quantum leg".into()))?;
    let q = st.ai.lock().await;
    let pack = q
        .quantum()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no quantum brains".into()))?;
    let m = pack.meta(id);
    Ok(Json(serde_json::json!(m)))
}

async fn get_qleg_model(
    State(st): State<RpcState>,
    Path(leg): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let id = mesh_ai::QuantumId::parse(&leg)
        .ok_or((StatusCode::BAD_REQUEST, "unknown quantum leg".into()))?;
    let q = st.ai.lock().await;
    let pack = q
        .quantum()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no quantum brains".into()))?;
    let m = pack.meta(id);
    Ok(Json(serde_json::json!({
        "leg": m.leg,
        "epoch": m.epoch,
        "digest_hex": m.digest_hex,
        "last_loss": m.last_loss,
        "last_acc": m.last_acc,
        "advances": m.advances,
        "train_steps_total": m.train_steps_total,
        "weights_hex": hex::encode(pack.weights(id)),
        "weights_bin": format!("/v1/qleg/{leg}/bin"),
    })))
}

async fn get_qleg_bin(
    State(st): State<RpcState>,
    Path(leg): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let id = mesh_ai::QuantumId::parse(&leg)
        .ok_or((StatusCode::BAD_REQUEST, "unknown quantum leg".into()))?;
    let q = st.ai.lock().await;
    let pack = q
        .quantum()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no quantum brains".into()))?;
    let m = pack.meta(id);
    let weights = pack.weights(id).to_vec();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        "x-mesh-epoch",
        HeaderValue::from_str(&m.epoch.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    headers.insert(
        "x-mesh-digest",
        HeaderValue::from_str(&m.digest_hex).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert(
        "x-mesh-leg",
        HeaderValue::from_str(&m.leg).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    Ok((StatusCode::OK, headers, weights))
}

async fn list_workers(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let q = st.ai.lock().await;
    let workers: Vec<_> = q
        .workers()
        .map(|w| {
            serde_json::json!({
                "address": w.address,
                "gpu_name": w.gpu_name,
                "vram_mb": w.vram_mb,
                "train_slots": mesh_ai::effective_train_slots(w),
                "kinds": w.kinds,
                "brain_backends": w.brain_backends,
                "brain_contract": w.brain_contract,
                "os_family": w.os_family,
                "brain_contract": w.brain_contract,
            })
        })
        .collect();
    Json(serde_json::json!({ "workers": workers }))
}

async fn research_status(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let q = st.ai.lock().await;
    let brain = q.brain().map(|b| b.meta());
    let brain_v2 = q.brain_v2().map(|b| b.meta());
    let quantum = q.quantum().map(|p| p.all_meta());
    Json(serde_json::json!({
        "verify_ok": q.verify_ok(),
        "verify_fail": q.verify_fail(),
        "verify_ok_rate": q.verify_ok_rate(),
        "protocol_eval_ok": q.protocol_eval_ok(),
        "research_scenarios_touched": q.research_scenarios_touched(),
        "research_scenarios": q.research_scenario_ids(),
        "pending": q.pending_len(),
        "inflight": q.inflight_len(),
        "completed": q.completed(),
        "scenarios": mesh_ai::research_catalog(),
        "brain": brain,
        "brain_v2": brain_v2,
        "quantum_guardians": quantum,
        "quantum_activity": q.quantum_activity(),
        "note": "shared brain + Trilemma + Quantum guardians + protocol sims",
    }))
}

async fn research_scenarios() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "scenarios": mesh_ai::research_catalog(),
        "note": "Blockchain self-improvement questions — GPU-paid protocol sims (Build/18, Build/21)",
    }))
}

fn shard_strict_enabled(board_warm: bool, edge_local_board: bool) -> bool {
    if std::env::var("MESH_AI_SHARD_STRICT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return true;
    }
    // Auto-strict when hybrid edge board has work (or workers) ready.
    edge_local_board && board_warm
}

fn shard_mismatch_response(
    worker: &str,
    board_warm: bool,
    edge_local_board: bool,
) -> Option<Result<Json<serde_json::Value>, (StatusCode, HeaderMap, String)>> {
    if !shard_strict_enabled(board_warm, edge_local_board) {
        return None;
    }
    let (my_id, count) = mesh_types::local_ai_shard_config();
    if count <= 1 {
        return None;
    }
    let want = mesh_types::ai_shard_for_worker(worker, count);
    if want == my_id {
        return None;
    }
    let urls = mesh_types::ai_shard_urls(count);
    let try_url = urls.get(want as usize).cloned().unwrap_or_default();
    Some(Err(ai_plain_err(
        StatusCode::MISDIRECTED_REQUEST,
        format!(
            "wrong AI shard: worker→{want} this_node={my_id}; try {try_url}"
        ),
    )))
}

fn client_ip(headers: &HeaderMap, connect: Option<&SocketAddr>) -> String {
    let trust_xff = std::env::var("MESH_AI_TRUST_XFF")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if trust_xff {
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = xff.split(',').next() {
                let ip = first.trim();
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        }
    }
    connect
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn check_board_limits(
    st: &RpcState,
    kind: &str,
    worker: &str,
    ip: &str,
) -> Result<(), (StatusCode, HeaderMap, String)> {
    let window = std::time::Duration::from_secs(60);
    let (worker_key, worker_limit) = match kind {
        "adv" => (format!("adv:{worker}"), ai_adv_limit()),
        "job" => (format!("job:{worker}"), ai_job_limit()),
        "res" => (format!("res:{worker}"), ai_res_limit()),
        _ => (format!("{kind}:{worker}"), 60),
    };
    let ip_key = format!("ip:{ip}");
    if let Err(ms) = st.ai_limit.check_all(
        &[
            (worker_key.as_str(), worker_limit),
            ("board:global", ai_global_limit()),
            (ip_key.as_str(), ai_ip_limit()),
        ],
        window,
    ) {
        let mut headers = HeaderMap::new();
        let secs = ((ms + 999) / 1000).max(1);
        if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
            headers.insert(header::RETRY_AFTER, v);
        }
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            format!("{kind} rate limit — retry in {ms}ms"),
        ));
    }
    Ok(())
}

fn ai_plain_err(code: StatusCode, msg: impl Into<String>) -> (StatusCode, HeaderMap, String) {
    (code, HeaderMap::new(), msg.into())
}

async fn advertise(
    State(st): State<RpcState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(cap): Json<Capability>,
) -> Result<Json<serde_json::Value>, (StatusCode, HeaderMap, String)> {
    crate::routes::require_ai_token(&st, &headers).map_err(|(c, m)| ai_plain_err(c, m))?;
    let ip = client_ip(&headers, Some(&addr));
    check_board_limits(&st, "adv", &cap.address, &ip)?;
    let (board_warm, edge_local) = {
        let q = st.ai.lock().await;
        let warm = q.queue_depth() > 0 || q.workers().next().is_some();
        (warm, q.brain().is_none())
    };
    if let Some(resp) = shard_mismatch_response(&cap.address, board_warm, edge_local) {
        return resp;
    }
    let id = {
        let mut q = st.ai.lock().await;
        q.advertise(cap)
            .map_err(|e| ai_plain_err(StatusCode::BAD_REQUEST, e.to_string()))?
    };
    // New GPU online → refill board immediately so workers don't idle.
    research_tick(&st).await;
    Ok(Json(serde_json::json!({ "worker_id": id })))
}

#[derive(Deserialize)]
struct WorkerReq {
    worker: String,
}

async fn take_job(
    State(st): State<RpcState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<WorkerReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, HeaderMap, String)> {
    crate::routes::require_ai_token(&st, &headers).map_err(|(c, m)| ai_plain_err(c, m))?;
    let ip = client_ip(&headers, Some(&addr));
    check_board_limits(&st, "job", &req.worker, &ip)?;
    let (board_warm, edge_local) = {
        let q = st.ai.lock().await;
        let warm = q.queue_depth() > 0 || q.workers().next().is_some();
        (warm, q.brain().is_none())
    };
    if let Some(resp) = shard_mismatch_response(&req.worker, board_warm, edge_local) {
        return resp;
    }

    // Long-poll up to ~2s so idle workers don't hammer the board.
    let deadline = Instant::now() + std::time::Duration::from_millis(2_000);
    let job = loop {
        {
            let mut q = st.ai.lock().await;
            match q.take_job(&req.worker) {
                Ok(job) => break job,
                Err(mesh_ai::OrchError::NoJob) => {
                    if should_empty_topup() {
                        drop(q);
                        research_tick(&st).await;
                        continue;
                    }
                }
                Err(mesh_ai::OrchError::UnknownWorker) => {
                    return Err(ai_plain_err(StatusCode::BAD_REQUEST, "unknown worker"));
                }
                Err(e) => return Err(ai_plain_err(StatusCode::BAD_REQUEST, e.to_string())),
            }
        }
        if Instant::now() >= deadline {
            return Err(ai_plain_err(StatusCode::NO_CONTENT, "no job"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    };

    Ok(Json(serde_json::json!({
        "job_id": job.job_id,
        "kind": job.kind,
        "input_hex": job.input_hex,
        "input_commitment": job.input_commitment,
    })))
}

#[derive(Deserialize)]
struct ResultReq {
    worker: String,
    job_id: String,
    output_hex: String,
    #[serde(default)]
    latency_ms: Option<u64>,
}

#[derive(Deserialize)]
struct ResultsReq {
    #[serde(default)]
    results: Vec<ResultReq>,
}

fn orch_err_status(e: &impl std::fmt::Display) -> StatusCode {
    if e.to_string().contains("stale") {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_REQUEST
    }
}

fn orch_err(e: &impl std::fmt::Display) -> (StatusCode, HeaderMap, String) {
    ai_plain_err(orch_err_status(e), e.to_string())
}

async fn credit_and_announce_receipt(st: &RpcState, receipt: mesh_types::AiJobReceipt) {
    {
        let mut c = st.chain.lock().await;
        let _ = c.record_ai_receipt(receipt.clone());
        let _ = c.credit_local_service(mesh_types::NodeServiceKind::AiRouting, 1);
    }
    crate::routes::invalidate_templates_pub(st);
    if let Some(net) = &st.network {
        let receipt_bytes = bincode::serialize(&receipt).unwrap_or_default();
        net.announce_ai_result(
            receipt.job_id.clone(),
            receipt.worker.to_hex(),
            receipt.output_hash,
            receipt.latency_ms,
            receipt.weight,
            receipt_bytes,
        );
    }
}

fn receipt_json(receipt: &mesh_types::AiJobReceipt, brain_epoch: Option<u64>, brain_v2_epoch: Option<u64>) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "job_id": receipt.job_id,
        "weight": receipt.weight,
        "job_kind": format!("{:?}", receipt.job_kind),
        "research_scenario": receipt.research_scenario,
        "score_primary": receipt.score_primary,
        "brain_epoch": brain_epoch,
        "brain_v2_epoch": brain_v2_epoch,
    })
}

async fn submit_result(
    State(st): State<RpcState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ResultReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, HeaderMap, String)> {
    crate::routes::require_ai_token(&st, &headers).map_err(|(c, m)| ai_plain_err(c, m))?;
    let ip = client_ip(&headers, Some(&addr));
    check_board_limits(&st, "res", &req.worker, &ip)?;
    let started = Instant::now();
    let height = {
        let c = st.chain.lock().await;
        c.height()
    };
    let latency = req
        .latency_ms
        .unwrap_or_else(|| started.elapsed().as_millis() as u64);
    // Heavy verify off the AI mutex (Build/27 N9) — board stays responsive during re-train.
    let worker = req.worker.clone();
    let job_id = req.job_id.clone();
    let output_hex = req.output_hex.clone();
    let (pending, fail_hint) = {
        let mut q = st.ai.lock().await;
        match q.prepare_complete(&worker, &job_id, &output_hex) {
            Ok(p) => {
                let hint = p.quantum_fail_hint();
                (p, hint)
            }
            Err(e) => {
                return Err(orch_err(&e));
            }
        }
    };
    let audit_every = {
        let c = st.chain.lock().await;
        c.active_envelopes().brain_audit_every
    };
    let audit_nonce = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        job_id.hash(&mut h);
        worker.hash(&mut h);
        h.finish()
    };
    let verified = match tokio::task::spawn_blocking(move || {
        pending.run_cpu_audited(audit_every, audit_nonce)
    })
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            {
                let mut q = st.ai.lock().await;
                if let Some((subject, detail)) = fail_hint {
                    q.push_quantum_story("failed", &subject, &detail);
                }
                q.note_verify_fail();
            }
            return Err(orch_err(&e));
        }
        Err(e) => {
            return Err(ai_plain_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };
    let receipt = {
        let mut q = st.ai.lock().await;
        match q.finish_complete(verified, latency, height) {
            Ok(r) => r,
            Err(e) => {
                q.note_verify_fail();
                return Err(orch_err(&e));
            }
        }
    };

    credit_and_announce_receipt(&st, receipt.clone()).await;

    let (brain_epoch, brain_v2_epoch) = {
        let q = st.ai.lock().await;
        (
            q.brain().map(|b| b.epoch),
            q.brain_v2().map(|b| b.epoch),
        )
    };

    Ok(Json(receipt_json(&receipt, brain_epoch, brain_v2_epoch)))
}

/// Batch verify/apply (Build/27 N9) — Light/Leg parallel; Shared may stale per-item.
async fn submit_results(
    State(st): State<RpcState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ResultsReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, HeaderMap, String)> {
    crate::routes::require_ai_token(&st, &headers).map_err(|(c, m)| ai_plain_err(c, m))?;
    let max = brain_verify_batch_max();
    if req.results.is_empty() || req.results.len() > max {
        return Err(ai_plain_err(
            StatusCode::BAD_REQUEST,
            format!("results batch must be 1..={max}"),
        ));
    }
    let ip = client_ip(&headers, Some(&addr));
    for item in &req.results {
        check_board_limits(&st, "res", &item.worker, &ip)?;
    }
    let started = Instant::now();
    let height = {
        let c = st.chain.lock().await;
        c.height()
    };
    let default_lat = started.elapsed().as_millis() as u64;
    let n = req.results.len();
    let job_ids: Vec<String> = req.results.iter().map(|r| r.job_id.clone()).collect();

    let mut rows: Vec<Option<serde_json::Value>> = vec![None; n];
    let mut to_verify: Vec<(usize, u64, mesh_ai::PendingVerify, Option<(String, String)>)> =
        Vec::new();
    {
        let mut q = st.ai.lock().await;
        for (i, item) in req.results.iter().enumerate() {
            let latency = item.latency_ms.unwrap_or(default_lat);
            match q.prepare_complete(&item.worker, &item.job_id, &item.output_hex) {
                Ok(p) => {
                    let hint = p.quantum_fail_hint();
                    to_verify.push((i, latency, p, hint));
                }
                Err(e) => {
                    rows[i] = Some(serde_json::json!({
                        "ok": false,
                        "job_id": item.job_id,
                        "error": e.to_string(),
                    }));
                }
            }
        }
    }

    let mut latencies = vec![default_lat; n];
    let mut pendings = Vec::with_capacity(to_verify.len());
    let mut verify_order = Vec::with_capacity(to_verify.len());
    let mut fail_hints: Vec<Option<(String, String)>> = Vec::with_capacity(to_verify.len());
    for (i, latency, pending, hint) in to_verify {
        latencies[i] = latency;
        verify_order.push(i);
        pendings.push(pending);
        fail_hints.push(hint);
    }

    let audit_every = {
        let c = st.chain.lock().await;
        c.active_envelopes().brain_audit_every
    };
    let cpu = if pendings.is_empty() {
        Vec::new()
    } else {
        tokio::task::spawn_blocking(move || {
            mesh_ai::run_cpu_batch_audited(pendings, audit_every)
        })
        .await
        .map_err(|e| ai_plain_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let mut finish_items = Vec::new();
    let mut finish_order = Vec::new();
    let mut ok_receipts: Vec<(usize, mesh_types::AiJobReceipt)> = Vec::new();
    {
        let mut q = st.ai.lock().await;
        for ((ord, result), hint) in verify_order.into_iter().zip(cpu).zip(fail_hints) {
            match result {
                Ok(v) => {
                    finish_order.push(ord);
                    finish_items.push((v, latencies[ord], height));
                }
                Err(e) => {
                    if let Some((subject, detail)) = hint {
                        q.push_quantum_story("failed", &subject, &detail);
                    }
                    q.note_verify_fail();
                    rows[ord] = Some(serde_json::json!({
                        "ok": false,
                        "job_id": job_ids[ord],
                        "error": e.to_string(),
                    }));
                }
            }
        }
        let finished = q.finish_completes(finish_items);
        for (ord, result) in finish_order.into_iter().zip(finished) {
            match result {
                Ok(receipt) => ok_receipts.push((ord, receipt)),
                Err(e) => {
                    rows[ord] = Some(serde_json::json!({
                        "ok": false,
                        "job_id": job_ids[ord],
                        "error": e.to_string(),
                    }));
                }
            }
        }
    }

    let (brain_epoch, brain_v2_epoch) = {
        let q = st.ai.lock().await;
        (
            q.brain().map(|b| b.epoch),
            q.brain_v2().map(|b| b.epoch),
        )
    };

    for (ord, receipt) in ok_receipts {
        rows[ord] = Some(receipt_json(&receipt, brain_epoch, brain_v2_epoch));
        credit_and_announce_receipt(&st, receipt).await;
    }

    let results: Vec<_> = rows
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            r.unwrap_or_else(|| {
                serde_json::json!({
                    "ok": false,
                    "job_id": job_ids[i],
                    "error": "internal batch gap",
                })
            })
        })
        .collect();
    let any_ok = results
        .iter()
        .any(|r| r.get("ok") == Some(&serde_json::Value::Bool(true)));
    Ok(Json(serde_json::json!({
        "ok": any_ok,
        "results": results,
    })))
}

async fn research_tick(st: &RpcState) {
    // Rate-limit meta fsync: dirty AI scores were fsyncing every 2s under the chain lock
    // and stalling getnodeinfo / mine RPC for multiple seconds on NAS disks.
    {
        static TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n % 80 == 0 {
            let mut c = st.chain.lock().await;
            let _ = c.flush_store();
        }
    }
    // Snapshot under lock; build pulse outside so RPC/P2P stay responsive.
    let (
        height,
        tip,
        gpu_w,
        node_w,
        receipts,
        env,
        node_count,
    ) = {
        let c = st.chain.lock().await;
        (
            c.height(),
            c.tip_hash().to_string(),
            c.store().gpu_scores().values().sum::<u64>(),
            c.store().node_scores().values().sum::<u64>(),
            c.store().ai_receipts().to_vec(),
            c.active_envelopes(),
            c.store().node_scores().len() as u32,
        )
    };
    let pulse = mesh_ai::build_mesh_pulse(height, tip, gpu_w, node_w, &receipts);
    let quantum_inputs =
        mesh_ai::quantum_score_inputs(&receipts, pulse.height, pulse.gpu_vs_height_signal);
    let (
        height,
        signal,
        avg_lat,
        progress,
        primary,
        orphan,
        detect,
        backlog,
        threshold,
        stipend_cap,
        prefer_v2,
        v2_min,
        v2_vram_floor,
        leg_enable,
        leg_parallel,
        quantum_enable,
        quantum_parallel,
        mean_link,
        mean_lat_p95,
        node_count,
        quantum_inputs,
    ) = (
        pulse.height,
        pulse.gpu_vs_height_signal,
        pulse.markets.avg_latency_ms,
        pulse.markets.research_progress,
        pulse.markets.research_scores.mean_primary,
        pulse.markets.research_scores.mean_orphan_risk,
        pulse.markets.research_scores.mean_detect_rate,
        pulse.markets.research_scores.mean_backlog_ratio,
        env.soft_adapt_signal_threshold,
        env.idle_stipend_bps_cap,
        env.brain_prefer_v2,
        env.brain_v2_min_workers,
        env.brain_v2_vram_floor_mb,
        env.leg_train_enable,
        env.leg_parallel,
        env.quantum_train_enable,
        env.quantum_parallel,
        pulse.markets.research_scores.mean_linkability,
        pulse.markets.research_scores.mean_latency_p95_ms,
        node_count,
        quantum_inputs,
    );

    let growth = growth_factor(height);

    let mut q = st.ai.lock().await;
    if q.workers().next().is_none() {
        if let Some(brain) = q.brain_mut() {
            let offset = (height.wrapping_mul(17) % 3500) as u32;
            match brain.local_step(height, 2, 64, offset) {
                Ok(ep) => tracing::info!(epoch = ep, height, "seed stepped shared brain (no workers)"),
                Err(e) => tracing::debug!(error = %e, "seed brain step skipped"),
            }
        }
        return;
    }

    let slots = q.total_train_slots();
    let total_vram = q.total_vram_mb();
    let max_depth = q
        .target_queue_depth(u32::from(stipend_cap))
        .min(ABS_MAX_QUEUE_DEPTH);

    let board = {
        let (epochs, smart) = match q.legs() {
            Some(p) => (p.epochs(), p.smart()),
            None => (mesh_ai::LegEpochs::default(), mesh_ai::LegSmart::default()),
        };
        mesh_ai::build_trilemma_board(
            detect,
            orphan,
            backlog,
            mean_lat_p95,
            mean_link,
            primary,
            node_count.max(1),
            q.workers().count() as u32,
            epochs,
            smart,
        )
    };

    let quantum_board = {
        let (epochs, smart) = match q.quantum() {
            Some(p) => (p.epochs(), p.smart()),
            None => (mesh_ai::QuantumEpochs::default(), mesh_ai::QuantumSmart::default()),
        };
        let mut board = mesh_ai::build_quantum_board(
            quantum_inputs.pqc_primary,
            quantum_inputs.pqc_detect,
            quantum_inputs.grover_primary,
            quantum_inputs.grover_orphan,
            quantum_inputs.grover_backlog,
            quantum_inputs.harvest_primary,
            quantum_inputs.harvest_linkability,
            epochs,
            smart,
        );
        board.note = format!(
            "{} — feed weakest={}; readiness is the min needle",
            quantum_inputs.honesty, board.weakest
        );
        board
    };

    fill_ai_queue(
        &mut q,
        max_depth,
        slots,
        growth,
        height,
        signal,
        avg_lat,
        orphan,
        detect,
        backlog,
        progress,
        primary,
        threshold,
        prefer_v2,
        v2_min,
        v2_vram_floor,
        leg_enable,
        leg_parallel,
        quantum_enable,
        quantum_parallel,
        &board,
        &quantum_board,
    );

    let fresh = q.take_ungossiped();
    let epoch = q.brain().map(|b| b.epoch).unwrap_or(0);
    let epoch_v2 = q.brain_v2().map(|b| b.epoch).unwrap_or(0);
    let depth = q.queue_depth();
    drop(q);
    for job in &fresh {
        announce_job(st, job);
    }
    tracing::debug!(
        height,
        growth,
        slots,
        total_vram,
        signal,
        brain_epoch = epoch,
        brain_v2_epoch = epoch_v2,
        prefer_v2,
        sec = board.sec,
        balance = board.balance,
        weakest = %board.weakest,
        depth,
        gossiped = fresh.len(),
        "node AI tick: guardians + protocol + shared brain"
    );
}

fn fill_ai_queue(
    q: &mut mesh_ai::JobQueue,
    max_depth: usize,
    slots: u32,
    growth: u64,
    height: u64,
    signal: f64,
    avg_lat: f64,
    orphan: f64,
    detect: f64,
    backlog: f64,
    progress: f64,
    primary: f64,
    threshold: f64,
    prefer_v2: u8,
    v2_min: u32,
    v2_vram_floor: u32,
    leg_enable: u8,
    leg_parallel: u32,
    quantum_enable: u8,
    quantum_parallel: u32,
    board: &mesh_ai::TrilemmaBoard,
    quantum_board: &mesh_ai::QuantumBoard,
) {
    if q.queue_depth() >= max_depth {
        return;
    }

    // At most two jobs per tick — rotate kinds so the miner sees a stream,
    // not a dump of legs + quantum + benches that stalls GPU Fusion.
    const DRIP: usize = 2;
    static ROT: AtomicU64 = AtomicU64::new(0);
    let rot = ROT.fetch_add(1, Ordering::Relaxed);
    let mut placed = 0usize;
    let _ = (leg_parallel, quantum_parallel);

    let has_protocol = q.any_worker_advertises_kind("protocol_eval");
    let has_brain = q.any_worker_advertises_kind("ml_train")
        || q.any_worker_advertises_kind("ml_train_shared");
    let has_leg = q.any_worker_advertises_kind("leg_train");
    let has_quantum = q.any_worker_advertises_kind("quantum_train");
    let has_bench = q.any_worker_advertises_kind("benchmark");

    let enqueue_protocol = |q: &mut mesh_ai::JobQueue, n: usize| -> bool {
        if !has_protocol {
            return false;
        }
        let scenario = if board.sec < 55 && n == 0 {
            ResearchScenario::SecurityAdversary
        } else {
            pick_chain_question(
                signal,
                avg_lat + (n as f64) * 50.0,
                orphan,
                detect,
                backlog,
                progress,
                primary,
            )
        };
        let _ = q.enqueue_research(scenario, height, signal);
        true
    };

    let enqueue_brain = |q: &mut mesh_ai::JobQueue| -> bool {
        let use_v2 = has_brain && q.should_enqueue_brain_v2(prefer_v2, v2_min, v2_vram_floor);
        if use_v2 {
            if q.shared_v2_jobs_queued_count() >= 1 {
                return false;
            }
            let (steps, samples) = q.sized_shared_train_v2(growth, v2_vram_floor);
            let offset = (height.wrapping_mul(17) % 3500) as u32;
            return q
                .enqueue_ml_train_shared_v2(steps, 50, samples.max(64), offset)
                .is_some();
        }
        if q.brain().is_some() && !q.shared_job_queued() {
            let (steps, samples) = q.sized_shared_train(None, growth);
            let offset = (height.wrapping_mul(17) % 3500) as u32;
            let _ = q.enqueue_ml_train_shared(steps, 50, samples.max(64), offset);
            return true;
        }
        false
    };

    let enqueue_leg = |q: &mut mesh_ai::JobQueue| -> bool {
        if !has_leg || leg_enable == 0 || q.legs().is_none() || q.leg_jobs_queued() >= 1 {
            return false;
        }
        let Some(leg) = mesh_ai::legs_priority(board).into_iter().next() else {
            return false;
        };
        let steps = (48u64.saturating_mul(growth)).clamp(32, 128) as u32;
        let samples = (64u64.saturating_mul(growth)).clamp(48, 128) as u32;
        let offset = (height.wrapping_mul(13) % 200) as u32;
        q.enqueue_leg_train(leg, steps, 50, samples, offset)
            .is_some()
    };

    let enqueue_quantum = |q: &mut mesh_ai::JobQueue| -> bool {
        if quantum_enable == 0 {
            return false;
        }
        if has_quantum
            && q.quantum().is_some()
            && q.quantum_jobs_queued() < 1
        {
            if let Some(leg) = mesh_ai::quantum_priority(quantum_board).into_iter().next() {
                let steps = (48u64.saturating_mul(growth)).clamp(32, 128) as u32;
                let samples = (64u64.saturating_mul(growth)).clamp(48, 128) as u32;
                let offset = (height.wrapping_mul(17) % 200) as u32;
                if q.enqueue_quantum_train(leg, steps, 50, samples, offset)
                    .is_some()
                {
                    return true;
                }
            }
        }
        if !has_protocol {
            return false;
        }
        let quantum_sims = [
            ResearchScenario::QuantumPqc,
            ResearchScenario::QuantumGrover,
            ResearchScenario::QuantumHarvest,
        ];
        let scenario = quantum_sims
            .into_iter()
            .find(|s| !q.scenario_touched(s.as_str()))
            .unwrap_or(quantum_sims[(height as usize) % quantum_sims.len()]);
        let _ = q.enqueue_research(scenario, height, signal);
        true
    };

    let enqueue_bench = |q: &mut mesh_ai::JobQueue| -> bool {
        if !has_bench || signal >= threshold {
            return false;
        }
        let rounds = ((2_000u64.saturating_mul(growth)).saturating_mul(slots.max(1) as u64) / 2)
            .clamp(2_000, 8_000) as u32;
        let _ = q.enqueue_benchmark(rounds);
        true
    };

    match rot % 6 {
        0 | 3 => {
            if enqueue_protocol(q, placed) {
                placed += 1;
            }
            if placed < DRIP && q.queue_depth() < max_depth && enqueue_brain(q) {
                placed += 1;
            }
            if placed < DRIP && q.queue_depth() < max_depth && enqueue_brain(q) {
                placed += 1;
            }
        }
        1 => {
            if enqueue_brain(q) {
                placed += 1;
            }
            if placed < DRIP && q.queue_depth() < max_depth && enqueue_protocol(q, placed) {
                placed += 1;
            }
            if placed < DRIP && q.queue_depth() < max_depth && enqueue_brain(q) {
                placed += 1;
            }
        }
        2 => {
            if enqueue_protocol(q, placed) {
                placed += 1;
            }
            if placed < DRIP && q.queue_depth() < max_depth && enqueue_leg(q) {
                placed += 1;
            }
            if placed < DRIP && q.queue_depth() < max_depth && enqueue_brain(q) {
                placed += 1;
            }
        }
        4 => {
            if enqueue_protocol(q, placed) {
                placed += 1;
            }
            if placed < DRIP && q.queue_depth() < max_depth && enqueue_quantum(q) {
                placed += 1;
            }
            if placed < DRIP && q.queue_depth() < max_depth && enqueue_brain(q) {
                placed += 1;
            }
        }
        _ => {
            if enqueue_protocol(q, placed) {
                placed += 1;
            }
            if placed < DRIP && q.queue_depth() < max_depth {
                if enqueue_bench(q) || enqueue_brain(q) {
                    placed += 1;
                }
            }
        }
    }
    let _ = placed;
}

fn growth_factor(height: u64) -> u64 {
    match height {
        0..=100 => 1,
        101..=1_000 => 2,
        1_001..=5_000 => 3,
        5_001..=20_000 => 4,
        _ => 5,
    }
}

/// Map MeshPulse / scores → the blockchain question GPUs should study next.
fn pick_chain_question(
    signal: f64,
    avg_lat: f64,
    orphan: f64,
    detect: f64,
    backlog: f64,
    progress: f64,
    primary: f64,
) -> ResearchScenario {
    // Periodically include quantum-era pressure tests in the classical rotation.
    let bucket = ((signal * 1000.0) as u64)
        .wrapping_add((avg_lat as u64).wrapping_mul(3))
        .wrapping_add((progress * 100.0) as u64);
    if bucket % 7 == 0 {
        return match bucket % 3 {
            0 => ResearchScenario::QuantumPqc,
            1 => ResearchScenario::QuantumGrover,
            _ => ResearchScenario::QuantumHarvest,
        };
    }
    if orphan > 0.45 {
        return ResearchScenario::BlockPropagation;
    }
    if detect > 0.0 && detect < 0.65 {
        return ResearchScenario::SecurityAdversary;
    }
    if backlog > 0.55 || avg_lat > 1_500.0 {
        return ResearchScenario::ScaleThroughput;
    }
    if signal < 0.25 {
        return ResearchScenario::RoutingEfficiency;
    }
    if progress < 0.25 {
        return ResearchScenario::MarketBalance;
    }
    if primary > 0.0 && primary < 0.45 {
        return ResearchScenario::VerifierQuorum;
    }
    ResearchScenario::PrivacyLeakage
}
