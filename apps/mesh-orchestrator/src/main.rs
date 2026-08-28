//! MonkeyMesh AI orchestrator — GPU adaptive research queue (Build/21).
//!
//! Credits verified receipts to the node RPC (`/v1/aireceipt`) so the next
//! block's GPU 40% pays workers. Soft envelopes auto-apply on the node.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use mesh_ai::{
    research_catalog, settle_amount_for_weight, Capability, JobQueue, MarketService, Marketplace,
    ResearchScenario,
};
use serde::Deserialize;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;

/// Keep research queue warm without drowning a single worker.
const MAX_RESEARCH_QUEUE_DEPTH: usize = 4;

#[derive(Clone)]
struct AppState {
    queue: Arc<Mutex<JobQueue>>,
    market: Arc<Mutex<Marketplace>>,
    /// Node RPC base, e.g. http://127.0.0.1:18080
    node_rpc: String,
    /// When false, skip posting receipts / settle if node is down (dev/smoke).
    require_node: bool,
    /// Optional `MESH_RPC_TOKEN` for mutating node routes.
    rpc_token: Option<String>,
    /// Pay worker MESH from node hot wallet after marketplace Done (legacy stub).
    settle: bool,
    /// Atomic MESH per receipt weight unit (default 100_000 = 0.001 MESH).
    settle_base_atomic: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let bind: SocketAddr = std::env::var("MESH_ORCH_BIND")
        .unwrap_or_else(|_| "127.0.0.1:18100".into())
        .parse()?;
    let node_rpc = std::env::var("MESH_NODE_RPC")
        .unwrap_or_else(|_| "http://127.0.0.1:18080".into());
    let require_node = std::env::var("MESH_ORCH_REQUIRE_NODE")
        .map(|v| v != "0" && v.to_ascii_lowercase() != "false")
        .unwrap_or(true);
    let rpc_token = std::env::var("MESH_RPC_TOKEN").ok().filter(|s| !s.is_empty());
    let settle = std::env::var("MESH_SETTLE")
        .map(|v| v != "0" && v.to_ascii_lowercase() != "false")
        .unwrap_or(true);
    let settle_base_atomic = std::env::var("MESH_SETTLE_BASE_ATOMIC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000); // 0.001 MESH per weight

    let state = AppState {
        queue: Arc::new(Mutex::new(JobQueue::default())),
        market: Arc::new(Mutex::new(Marketplace::default())),
        node_rpc,
        require_node,
        rpc_token,
        settle,
        settle_base_atomic,
    };

    let app = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/advertise", post(advertise))
        .route("/v1/job", post(take_job))
        .route("/v1/result", post(submit_result))
        .route("/v1/enqueue", post(enqueue))
        .route("/v1/workers", get(list_workers))
        .route("/v1/routing", get(routing))
        .route("/v1/research/scenarios", get(research_scenarios))
        .route("/v1/research/enqueue", post(research_enqueue))
        .route("/v1/research/status", get(research_status))
        .route("/v1/marketplace/jobs", post(mkt_submit).get(mkt_list))
        .route("/v1/marketplace/jobs/{id}", get(mkt_get))
        .route("/", get(marketplace_page))
        .route("/marketplace", get(marketplace_page))
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    tracing::info!(%bind, require_node, settle, settle_base_atomic, "mesh-orchestrator listening");
    let pulse_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            research_tick(&pulse_state).await;
        }
    });
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Always-on adaptive research: fill under-covered scenarios while workers are online.
async fn research_tick(st: &AppState) {
    let base = st.node_rpc.trim_end_matches('/');
    let (threshold, rounds, stipend_cap, min_verifier) = {
        let url = format!("{base}/v1/envelopes");
        let env = ureq::get(&url)
            .call()
            .ok()
            .and_then(|r| r.into_json::<serde_json::Value>().ok());
        let threshold = env
            .as_ref()
            .and_then(|v| {
                v.get("envelopes")?
                    .get("soft_adapt_signal_threshold")?
                    .as_f64()
            })
            .unwrap_or(0.5);
        let rounds = env
            .as_ref()
            .and_then(|v| {
                v.get("envelopes")?
                    .get("soft_benchmark_rounds")?
                    .as_u64()
            })
            .unwrap_or(2_000) as u32;
        let stipend_cap = env
            .as_ref()
            .and_then(|v| {
                v.get("envelopes")?
                    .get("idle_stipend_bps_cap")?
                    .as_u64()
            })
            .unwrap_or(1_000) as u16;
        let min_verifier = env
            .as_ref()
            .and_then(|v| {
                v.get("envelopes")?
                    .get("min_verifier_weight")?
                    .as_u64()
            })
            .unwrap_or(1);
        (threshold, rounds, stipend_cap, min_verifier)
    };

    // Stipend scales how warm we keep the queue (tight stipend → shallower queue).
    let max_depth = ((MAX_RESEARCH_QUEUE_DEPTH as u64 * stipend_cap as u64) / 1_000)
        .clamp(1, MAX_RESEARCH_QUEUE_DEPTH as u64) as usize;

    let url = format!("{base}/v1/meshpulse");
    let pulse = ureq::get(&url)
        .call()
        .ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok());
    let (height, signal, _avg_lat) = if let Some(ref pulse) = pulse {
        (
            pulse.get("height").and_then(|h| h.as_u64()).unwrap_or(0),
            pulse
                .get("gpu_vs_height_signal")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            pulse
                .pointer("/markets/avg_latency_ms")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
        )
    } else {
        (0, 0.0, 0.0)
    };

    {
        let mut q = st.queue.lock().await;
        if q.workers().next().is_none() {
            return;
        }
        if q.queue_depth() >= max_depth {
            return;
        }

        // Primary GPU work: real MNIST training (verified re-exec).
        let steps = ((rounds as u64) / 40).clamp(24, 128) as u32;
        let samples = if stipend_cap >= 750 { 512 } else { 256 };
        let seed = height.saturating_mul(1009).wrapping_add(now_secs_u64());
        let offset = (height.wrapping_mul(17) % 3500) as u32;
        let _ = q.enqueue_ml_train(steps, 50, seed, samples, offset);

        // Optional second train shard when adapt signal is weak and queue has room.
        if signal < threshold && q.queue_depth() < max_depth {
            let _ = q.enqueue_ml_train(
                steps,
                50,
                seed.wrapping_add(1),
                samples,
                (offset + samples) % 3500,
            );
        }

        tracing::debug!(
            signal,
            threshold,
            stipend_cap,
            min_verifier,
            max_depth,
            height,
            steps,
            samples,
            depth = q.queue_depth(),
            "research-tick: enqueued real MNIST ml_train work"
        );
    }
}

fn now_secs_u64() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1)
}

async fn routing(State(st): State<AppState>) -> Json<serde_json::Value> {
    let min_verifier = {
        let base = st.node_rpc.trim_end_matches('/');
        let url = format!("{base}/v1/envelopes");
        ureq::get(&url)
            .call()
            .ok()
            .and_then(|r| r.into_json::<serde_json::Value>().ok())
            .and_then(|v| {
                v.get("envelopes")?
                    .get("min_verifier_weight")?
                    .as_u64()
            })
            .unwrap_or(1)
    };
    let q = st.queue.lock().await;
    let ranks = q.rank_workers_biased(min_verifier);
    let preferred = q.preferred_worker_biased(min_verifier);
    Json(serde_json::json!({
        "preferred_worker": preferred,
        "min_verifier_weight": min_verifier,
        "ranks": ranks,
        "completed": q.completed(),
        "note": "soft routing — higher min_verifier_weight penalizes slow GPUs harder; consensus unchanged",
    }))
}

async fn health(State(st): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "ok": true,
        "service": "mesh-orchestrator",
        "adaptive_research": true,
        "marketplace_shelved": true,
        "ml_train": true,
        "ml_dataset": "MNIST-4096 (official training subset)",
        "research": true,
        "settle": st.settle,
        "settle_base_atomic": st.settle_base_atomic,
        "require_node": st.require_node,
    }))
}

async fn research_scenarios() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "scenarios": research_catalog(),
        "note": "Adaptive protocol research — GPU-paid; soft envelopes auto-apply (Build/21)",
    }))
}

#[derive(Deserialize)]
struct ResearchEnqueueReq {
    /// spam_recovery | routing_efficiency | market_balance | verifier_quorum
    scenario: String,
    #[serde(default)]
    height: Option<u64>,
    #[serde(default)]
    pulse_signal: Option<f64>,
}

async fn research_enqueue(
    State(st): State<AppState>,
    Json(req): Json<ResearchEnqueueReq>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let scenario = ResearchScenario::parse(&req.scenario).ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "scenario must be spam_recovery|routing_efficiency|market_balance|verifier_quorum"
                .into(),
        )
    })?;
    let height = req.height.unwrap_or(0);
    let signal = req.pulse_signal.unwrap_or(0.0);
    let mut q = st.queue.lock().await;
    let job = q.enqueue_research(scenario, height, signal);
    Ok(Json(serde_json::json!({
        "job_id": job.job_id,
        "scenario": scenario.as_str(),
        "kind": "protocol_eval",
        "input_commitment": job.input_commitment.to_string(),
    })))
}

async fn research_status(State(st): State<AppState>) -> Json<serde_json::Value> {
    let q = st.queue.lock().await;
    Json(serde_json::json!({
        "verify_ok": q.verify_ok(),
        "verify_fail": q.verify_fail(),
        "verify_ok_rate": q.verify_ok_rate(),
        "protocol_eval_ok": q.protocol_eval_ok(),
        "research_scenarios_touched": q.research_scenarios_touched(),
        "pending": q.pending_len(),
        "inflight": q.inflight_len(),
        "completed": q.completed(),
        "scenarios": research_catalog(),
        "note": "orchestrator-local research metrics (protocol sims v2; not consensus)",
    }))
}

async fn advertise(
    State(st): State<AppState>,
    Json(cap): Json<Capability>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut q = st.queue.lock().await;
    let id = q
        .advertise(cap)
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(serde_json::json!({ "worker_id": id })))
}

#[derive(Deserialize)]
struct WorkerReq {
    worker: String,
}

async fn take_job(
    State(st): State<AppState>,
    Json(req): Json<WorkerReq>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let job = {
        let mut q = st.queue.lock().await;
        q.take_job(&req.worker)
            .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?
    };
    {
        let mut m = st.market.lock().await;
        m.on_worker_assigned(&job.job_id, &req.worker);
    }
    Ok(Json(serde_json::json!(job)))
}

#[derive(Deserialize)]
struct ResultReq {
    worker: String,
    job_id: String,
    output_hex: String,
    #[serde(default)]
    latency_ms: Option<u64>,
}

async fn submit_result(
    State(st): State<AppState>,
    Json(req): Json<ResultReq>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let started = Instant::now();
    let receipt = {
        let mut q = st.queue.lock().await;
        let latency = req
            .latency_ms
            .unwrap_or_else(|| started.elapsed().as_millis() as u64);
        q.complete(&req.worker, &req.job_id, &req.output_hex, latency)
            .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?
    };

    let market_job_id = {
        let mut m = st.market.lock().await;
        m.on_worker_result(
            &req.job_id,
            &req.worker,
            &receipt.output_hash.to_string(),
            &req.output_hex,
            true,
            None,
        )
    };

    let mut settlement_txid: Option<String> = None;
    let mut settlement_amount: Option<String> = None;
    let mut settlement_error: Option<String> = None;

    if let Some(ref mid) = market_job_id {
        if st.settle {
            let amount = settle_amount_for_weight(st.settle_base_atomic, receipt.weight);
            match post_sendtoaddress(&st, &req.worker, &amount, mid).await {
                Ok(txid) => {
                    let mut m = st.market.lock().await;
                    m.mark_settled(mid, &amount, &txid);
                    settlement_txid = Some(txid);
                    settlement_amount = Some(amount);
                }
                Err(e) => {
                    let mut m = st.market.lock().await;
                    m.mark_settle_failed(mid, &e);
                    settlement_error = Some(e.clone());
                    if st.require_node {
                        return Err((
                            axum::http::StatusCode::BAD_GATEWAY,
                            format!("marketplace settle failed: {e}"),
                        ));
                    }
                    tracing::warn!(error = %e, "settle failed; continuing (require_node=0)");
                }
            }
        } else {
            let mut m = st.market.lock().await;
            m.mark_settle_skipped(mid, "MESH_SETTLE=0");
        }
    }

    let url = format!("{}/v1/aireceipt", st.node_rpc.trim_end_matches('/'));
    let body = serde_json::json!({
        "job_id": receipt.job_id,
        "worker": receipt.worker.to_string(),
        "input_commitment": receipt.input_commitment.to_string(),
        "output_hash": receipt.output_hash.to_string(),
        "latency_ms": receipt.latency_ms,
        "weight": receipt.weight,
        "verified_at": receipt.verified_at,
        "job_kind": match receipt.job_kind {
            mesh_types::AiJobKind::Echo => "echo",
            mesh_types::AiJobKind::Benchmark => "benchmark",
            mesh_types::AiJobKind::ProtocolEval => "protocol_eval",
            mesh_types::AiJobKind::AgentAssist => "agent_assist",
            mesh_types::AiJobKind::MlTrain => "ml_train",
        },
        "research_scenario": receipt.research_scenario,
        "score_primary": receipt.score_primary,
        "score_orphan_risk": receipt.score_orphan_risk,
        "score_detect_rate": receipt.score_detect_rate,
        "score_linkability": receipt.score_linkability,
        "score_backlog_ratio": receipt.score_backlog_ratio,
        "score_latency_p95_ms": receipt.score_latency_p95_ms,
    });
    match post_node_json(&st, &url, &body) {
        Ok(_) => {}
        Err(e) => {
            if st.require_node {
                return Err((
                    axum::http::StatusCode::BAD_GATEWAY,
                    format!("node aireceipt failed: {e}"),
                ));
            }
            tracing::warn!(error = %e, "aireceipt failed; continuing (require_node=0)");
        }
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "job_id": receipt.job_id,
        "weight": receipt.weight,
        "worker": receipt.worker.to_string(),
        "market_job_id": market_job_id,
        "settlement_status": match (&settlement_txid, &settlement_error, &market_job_id, st.settle) {
            (Some(_), _, _, _) => "paid",
            (_, Some(_), _, _) => "failed",
            (_, _, Some(_), false) => "skipped",
            _ => "none",
        },
        "settlement_amount": settlement_amount,
        "settlement_txid": settlement_txid,
        "settlement_error": settlement_error,
    })))
}

fn post_node_json(
    st: &AppState,
    url: &str,
    body: &serde_json::Value,
) -> Result<(), String> {
    let mut req = ureq::post(url);
    if let Some(token) = &st.rpc_token {
        req = req.set("X-Mesh-Token", token);
    }
    match req.send_json(body) {
        Ok(resp) if (200..300).contains(&resp.status()) => Ok(()),
        Ok(resp) => Err(format!("HTTP {}", resp.status())),
        Err(e) => Err(e.to_string()),
    }
}

async fn post_sendtoaddress(
    st: &AppState,
    address: &str,
    amount: &str,
    market_job_id: &str,
) -> Result<String, String> {
    let url = format!("{}/v1/sendtoaddress", st.node_rpc.trim_end_matches('/'));
    let body = serde_json::json!({
        "address": address,
        "amount": amount,
        "memo": format!("mkt-settle:{market_job_id}"),
    });
    // ureq is sync; run off the async runtime briefly via spawn_blocking.
    let token = st.rpc_token.clone();
    let url_c = url.clone();
    let body_c = body.clone();
    tokio::task::spawn_blocking(move || {
        let mut req = ureq::post(&url_c);
        if let Some(t) = &token {
            req = req.set("X-Mesh-Token", t);
        }
        let resp = req.send_json(&body_c).map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.into_string().unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}: {text}"));
        }
        let v: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("bad json: {e} ({text})"))?;
        v.get("txid")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "sendtoaddress missing txid".into())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
struct EnqueueReq {
    #[serde(default = "default_echo")]
    kind: String,
    #[serde(default)]
    input_hex: Option<String>,
    #[serde(default)]
    rounds: Option<u32>,
}

fn default_echo() -> String {
    "echo".into()
}

async fn enqueue(
    State(st): State<AppState>,
    Json(req): Json<EnqueueReq>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut q = st.queue.lock().await;
    let job = if req.kind == "benchmark" {
        q.enqueue_benchmark(req.rounds.unwrap_or(1000))
    } else if req.kind == "protocol_eval" {
        let input = match req.input_hex {
            Some(h) => hex::decode(h)
                .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?,
            None => b"protocol-eval".to_vec(),
        };
        q.enqueue_protocol_eval(input)
    } else if req.kind == "ml_train" {
        q.enqueue_ml_train(
            req.rounds.unwrap_or(48),
            50,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(1),
            256,
            0,
        )
    } else {
        let input = match req.input_hex {
            Some(h) => hex::decode(h)
                .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?,
            None => b"monkeymesh-echo".to_vec(),
        };
        q.enqueue_echo(input)
    };
    Ok(Json(serde_json::json!({
        "job_id": job.job_id,
        "kind": format!("{:?}", job.kind),
    })))
}

async fn list_workers(State(st): State<AppState>) -> Json<serde_json::Value> {
    let q = st.queue.lock().await;
    let workers: Vec<_> = q.workers().cloned().collect();
    Json(serde_json::json!({ "workers": workers }))
}

#[derive(Deserialize)]
struct MktSubmitReq {
    /// echo | llm | embeddings | image | agent
    service: String,
    prompt: String,
}

async fn marketplace_page() -> Html<&'static str> {
    Html(include_str!("../static/marketplace.html"))
}

async fn mkt_submit(
    State(st): State<AppState>,
    Json(req): Json<MktSubmitReq>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let service = MarketService::parse(&req.service).ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "service must be echo|llm|embeddings|image|agent".into(),
        )
    })?;
    let mut q = st.queue.lock().await;
    let mut m = st.market.lock().await;
    let job = m.submit(&mut q, service, req.prompt).map_err(|e| {
        let code = match e {
            mesh_ai::MarketError::RateLimited | mesh_ai::MarketError::Capacity => {
                axum::http::StatusCode::TOO_MANY_REQUESTS
            }
            _ => axum::http::StatusCode::BAD_REQUEST,
        };
        (code, e.to_string())
    })?;
    Ok(Json(serde_json::json!({ "job": job })))
}

async fn mkt_list(State(st): State<AppState>) -> Json<serde_json::Value> {
    let m = st.market.lock().await;
    let jobs: Vec<_> = m.list().into_iter().cloned().collect();
    Json(serde_json::json!({ "jobs": jobs }))
}

async fn mkt_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let m = st.market.lock().await;
    let job = m.get(&id).cloned().ok_or_else(|| {
        (
            axum::http::StatusCode::NOT_FOUND,
            format!("unknown job {id}"),
        )
    })?;
    Ok(Json(serde_json::json!({ "job": job })))
}
