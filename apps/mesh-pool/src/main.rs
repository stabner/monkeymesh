//! MESH HTTP testnet pool — GBT with pool coinbase, forward submitblock, credit miners.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use mesh_crypto::Keypair;
use mesh_types::{Address, COINBASE_MATURITY, TARGET_BLOCK_TIME_SECS};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

/// Rolling window for hashrate / jobs-per-second estimates.
const RATE_WINDOW_SECS: u64 = 120;
/// Miner counts as connected if they pulled a template recently.
const MINER_ACTIVE_SECS: u64 = 180;

#[derive(Parser, Debug)]
#[command(name = "mesh-pool")]
struct Args {
    /// Bind address (default 0.0.0.0:12500 — Hashmonkeys scheme: DFPPS+ / MESH suffix 500).
    #[arg(long, default_value = "0.0.0.0:12500")]
    bind: String,
    /// Upstream mine RPC (edge getblocktemplate / submitblock).
    #[arg(long, default_value = "http://127.0.0.1:18081")]
    upstream: String,
    /// Pool wallet key file (created if missing).
    #[arg(long, default_value = "data/pool.key")]
    keyfile: PathBuf,
    /// Credits ledger JSON.
    #[arg(long, default_value = "data/pool_credits.json")]
    credits: PathBuf,
    /// Recent accepted blocks ledger (for web UI explorer — not Miningcore DB).
    #[arg(long, default_value = "data/pool_blocks.json")]
    blocks: PathBuf,
    /// Optional coinbase override. When unset, each miner’s `?address=` / `X-Mesh-Miner` is paid.
    #[arg(long)]
    payout_address: Option<String>,
}

#[derive(Clone)]
struct AppState {
    upstream: String,
    pool_address: String,
    /// Forced coinbase (operator). Empty = pay the miner’s wallet.
    payout_override: Option<String>,
    inner: Arc<Mutex<PoolInner>>,
    credits_path: PathBuf,
    blocks_path: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
struct FoundBlock {
    height: u64,
    #[serde(default)]
    hash: String,
    miner: String,
    /// MiningCore-style `wallet.worker` (same as `miner` when the miner sent a worker).
    #[serde(default)]
    worker: String,
    /// Unix seconds
    created: u64,
}

struct PoolInner {
    blocks_found: u64,
    /// miner address → blocks credited
    credits: HashMap<String, u64>,
    /// newest-first accepted blocks (capped)
    recent_blocks: Vec<FoundBlock>,
    jobs: u64,
    /// (unix_secs, expected hashes) for accepted full-diff blocks
    work_events: VecDeque<(u64, f64)>,
    /// (unix_secs) per GBT pull — jobs/s
    job_events: VecDeque<u64>,
    /// miner → last GBT / submit seen
    miners_seen: HashMap<String, u64>,
    last_height: u64,
    last_difficulty: u32,
}

#[derive(Deserialize)]
struct AddressQuery {
    address: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct BlockTemplateResp {
    height: u64,
    difficulty: u32,
    soft_diff_hint: u32,
    light_pow: bool,
    #[serde(default = "default_pow_version")]
    pow_version: u8,
    #[serde(default)]
    pow_recipe: String,
    #[serde(default)]
    assigned_role: String,
    #[serde(default)]
    mesh_strength: u64,
    address: String,
    block_hex: String,
    #[serde(default)]
    job_id: String,
    #[serde(default)]
    pool: bool,
    #[serde(default)]
    exam_root: String,
    #[serde(default)]
    exam_scenario: String,
    #[serde(default)]
    exam_title: String,
    #[serde(default)]
    exam_payload_hex: String,
    #[serde(default)]
    exam_job_id: String,
    #[serde(default)]
    fair_split: bool,
    #[serde(default)]
    cpu_bps: u16,
    #[serde(default)]
    gpu_bps: u16,
    #[serde(default)]
    node_bps: u16,
}

fn default_pow_version() -> u8 {
    1
}

#[derive(Deserialize)]
struct SubmitBlockReq {
    block_hex: String,
}

#[derive(Serialize)]
struct SubmitBlockResp {
    accepted: bool,
    height: Option<u64>,
    id: Option<String>,
    credited_to: Option<String>,
    message: String,
}

#[derive(Serialize)]
struct PoolStats {
    ok: bool,
    pool_address: String,
    /// Coinbase destination: operator override, or `miner` (pays `?address=` / X-Mesh-Miner).
    payout_address: String,
    upstream: String,
    blocks_found: u64,
    jobs_served: u64,
    credits: HashMap<String, u64>,
    /// Estimated MeshHash H/s over the last ~2 minutes (from accepted blocks × 2^diff).
    pool_hashrate: f64,
    /// Network estimate: 2^diff / target block time.
    network_hashrate: f64,
    jobs_per_second: f64,
    connected_miners: u64,
    block_height: u64,
    difficulty: u32,
    /// Newest-first accepted blocks (same shape used by the gateway bridge).
    #[serde(default)]
    recent_blocks: Vec<FoundBlockView>,
    /// Coinbase spend delay (matches `mesh_types::COINBASE_MATURITY`).
    coinbase_maturity: u64,
}

#[derive(Clone, Serialize)]
struct FoundBlockView {
    height: u64,
    hash: String,
    miner: String,
    worker: String,
    created: u64,
    confirmations: u64,
    mature: bool,
    remain: u64,
}

fn block_maturity(tip: u64, height: u64) -> (u64, bool, u64) {
    let confirmations = tip.saturating_sub(height).saturating_add(1);
    let mature = tip.saturating_add(1) >= height.saturating_add(COINBASE_MATURITY);
    let remain = COINBASE_MATURITY.saturating_sub(confirmations.min(COINBASE_MATURITY));
    (confirmations, mature, remain)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hashes_for_diff(diff: u32) -> f64 {
    let d = diff.min(62);
    2f64.powi(d as i32)
}

fn prune_work(q: &mut VecDeque<(u64, f64)>, cutoff: u64) {
    while q.front().map(|(t, _)| *t < cutoff).unwrap_or(false) {
        q.pop_front();
    }
}

fn prune_jobs(q: &mut VecDeque<u64>, cutoff: u64) {
    while q.front().map(|t| *t < cutoff).unwrap_or(false) {
        q.pop_front();
    }
}

fn rolling_hashrate(events: &VecDeque<(u64, f64)>, now: u64) -> f64 {
    let cutoff = now.saturating_sub(RATE_WINDOW_SECS);
    let mut sum = 0.0;
    let mut oldest = now;
    for (t, h) in events.iter() {
        if *t >= cutoff {
            sum += *h;
            oldest = oldest.min(*t);
        }
    }
    if sum <= 0.0 {
        return 0.0;
    }
    let span = (now.saturating_sub(oldest)).max(1) as f64;
    sum / span
}

fn jobs_per_second(events: &VecDeque<u64>, now: u64) -> f64 {
    let cutoff = now.saturating_sub(RATE_WINDOW_SECS);
    let n = events.iter().filter(|t| **t >= cutoff).count() as f64;
    if n <= 0.0 {
        return 0.0;
    }
    let oldest = events
        .iter()
        .copied()
        .filter(|t| *t >= cutoff)
        .min()
        .unwrap_or(now);
    let span = (now.saturating_sub(oldest)).max(1) as f64;
    n / span
}

fn connected_miners(seen: &HashMap<String, u64>, now: u64) -> u64 {
    let cutoff = now.saturating_sub(MINER_ACTIVE_SECS);
    seen.values().filter(|t| **t >= cutoff).count() as u64
}

fn load_or_create_key(path: &Path) -> anyhow::Result<Keypair> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let bytes = std::fs::read(path)?;
        if bytes.len() == 32 {
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            return Ok(Keypair::from_bytes(secret));
        }
        let hex = String::from_utf8_lossy(&bytes);
        return Keypair::from_hex(hex.trim()).map_err(|e| anyhow::anyhow!("bad pool key: {e}"));
    }
    let kp = Keypair::generate();
    std::fs::write(path, kp.to_bytes())?;
    info!(path = %path.display(), address = %kp.address(), "created pool wallet");
    Ok(kp)
}

fn load_credits(path: &Path) -> HashMap<String, u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_credits(path: &Path, credits: &HashMap<String, u64>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_vec_pretty(credits) {
        let _ = std::fs::write(path, body);
    }
}

fn load_blocks(path: &Path) -> Vec<FoundBlock> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_blocks(path: &Path, blocks: &[FoundBlock]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_vec_pretty(blocks) {
        let _ = std::fs::write(path, body);
    }
}

const MAX_RECENT_BLOCKS: usize = 500;

fn miner_from_headers(headers: &HeaderMap, q: &AddressQuery) -> String {
    if let Some(v) = headers.get("x-mesh-miner").and_then(|v| v.to_str().ok()) {
        let t = v.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    if let Some(a) = q.address.as_ref() {
        let t = a.trim();
        if !t.is_empty() {
            let base = t.split('.').next().unwrap_or(t);
            if Address::from_hex(base).is_some() {
                return t.to_string();
            }
        }
    }
    "anonymous".into()
}

/// Coinbase address: operator override → miner wallet (strip `.worker`) → pool key.
fn coinbase_address(miner: &str, override_addr: Option<&str>, fallback: &str) -> String {
    if let Some(raw) = override_addr {
        let t = raw.trim();
        if Address::from_hex(t).is_some() {
            return t.to_string();
        }
    }
    let base = miner.split('.').next().unwrap_or(miner).trim();
    if Address::from_hex(base).is_some() {
        base.to_string()
    } else {
        fallback.to_string()
    }
}

async fn get_template(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<AddressQuery>,
) -> Result<Json<BlockTemplateResp>, (StatusCode, String)> {
    let miner = miner_from_headers(&headers, &q);
    let payout = coinbase_address(
        &miner,
        st.payout_override.as_deref(),
        &st.pool_address,
    );
    let url = format!(
        "{}/v1/getblocktemplate?address={}",
        st.upstream.trim_end_matches('/'),
        payout
    );
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream gbt: {e}")))?;
    let mut tmpl: BlockTemplateResp = resp
        .into_json()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream json: {e}")))?;
    tmpl.address = payout;
    tmpl.pool = true;
    let job_id = {
        let mut g = st.inner.lock().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "lock".into(),
            )
        })?;
        let now = now_secs();
        g.jobs = g.jobs.saturating_add(1);
        g.last_height = tmpl.height;
        g.last_difficulty = tmpl.difficulty;
        g.miners_seen.insert(miner, now);
        g.job_events.push_back(now);
        prune_jobs(&mut g.job_events, now.saturating_sub(RATE_WINDOW_SECS));
        format!("pool-{}-{}", now, g.jobs)
    };
    tmpl.job_id = job_id;
    Ok(Json(tmpl))
}

async fn proxy_exam_submit(
    State(st): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let url = format!("{}/v1/exam/submit", st.upstream.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .timeout(std::time::Duration::from_secs(20))
        .send_json(&body)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream exam: {e}")))?;
    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream exam json: {e}")))?;
    Ok(Json(v))
}

async fn proxy_exam_status(
    State(st): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let url = format!("{}/v1/exam/status", st.upstream.trim_end_matches('/'));
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream exam status: {e}")))?;
    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream exam json: {e}")))?;
    Ok(Json(v))
}

async fn proxy_getnodeinfo(
    State(st): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let url = format!("{}/v1/getnodeinfo", st.upstream.trim_end_matches('/'));
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(8))
        .call()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream nodeinfo: {e}")))?;
    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream nodeinfo json: {e}")))?;
    Ok(Json(v))
}

async fn proxy_getrewards(
    State(st): State<AppState>,
    Query(q): Query<AddressQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut url = format!("{}/v1/getrewards", st.upstream.trim_end_matches('/'));
    if let Some(addr) = q.address.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        url = format!("{url}?address={addr}");
    }
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(20))
        .call()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream rewards: {e}")))?;
    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream rewards json: {e}")))?;
    Ok(Json(v))
}

async fn submit_block(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<AddressQuery>,
    Json(req): Json<SubmitBlockReq>,
) -> Result<Json<SubmitBlockResp>, (StatusCode, String)> {
    let miner = miner_from_headers(&headers, &q);
    let url = format!("{}/v1/submitblock", st.upstream.trim_end_matches('/'));
    let upstream_body = serde_json::json!({ "block_hex": req.block_hex });
    let resp = ureq::post(&url)
        .timeout(std::time::Duration::from_secs(30))
        .send_json(&upstream_body)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream submit: {e}")))?;
    let status = resp.status();
    let text = resp.into_string().unwrap_or_default();
    if status >= 300 {
        return Ok(Json(SubmitBlockResp {
            accepted: false,
            height: None,
            id: None,
            credited_to: None,
            message: format!("upstream HTTP {status}: {text}"),
        }));
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "accepted": true }));
    let accepted = parsed
        .get("accepted")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let height = parsed.get("height").and_then(|v| v.as_u64());
    let id = parsed
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if accepted {
        let mut g = st.inner.lock().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "lock".into(),
            )
        })?;
        let now = now_secs();
        let diff = g.last_difficulty.max(1);
        g.blocks_found = g.blocks_found.saturating_add(1);
        *g.credits.entry(miner.clone()).or_insert(0) += 1;
        g.miners_seen.insert(miner.clone(), now);
        if let Some(h) = height {
            g.last_height = h;
        }
        let found = FoundBlock {
            height: height.unwrap_or(g.last_height),
            hash: id.clone().unwrap_or_default(),
            miner: miner.clone(),
            worker: miner.clone(),
            created: now,
        };
        g.recent_blocks.insert(0, found);
        if g.recent_blocks.len() > MAX_RECENT_BLOCKS {
            g.recent_blocks.truncate(MAX_RECENT_BLOCKS);
        }
        g.work_events.push_back((now, hashes_for_diff(diff)));
        prune_work(
            &mut g.work_events,
            now.saturating_sub(RATE_WINDOW_SECS),
        );
        save_credits(&st.credits_path, &g.credits);
        save_blocks(&st.blocks_path, &g.recent_blocks);
        info!(%miner, ?height, ?id, diff, "pool block accepted");
        let body = upstream_body.clone();
        let fanout: Vec<String> = mesh_types::default_rpc_urls()
            .into_iter()
            .filter(|u| u.trim_end_matches('/') != st.upstream.trim_end_matches('/'))
            .take(6)
            .collect();
        std::thread::spawn(move || {
            for base in fanout {
                let url = format!("{}/v1/submitblock", base.trim_end_matches('/'));
                match ureq::post(&url)
                    .timeout(std::time::Duration::from_secs(8))
                    .send_json(&body)
                {
                    Ok(r) if r.status() < 300 => {
                        info!(peer = %base, "fan-out submit accepted");
                    }
                    Ok(r) => {
                        warn!(peer = %base, status = r.status(), "fan-out submit HTTP");
                    }
                    Err(e) => {
                        warn!(peer = %base, error = %e, "fan-out submit failed");
                    }
                }
            }
        });
    } else {
        warn!(%miner, msg = %text, "pool block rejected upstream");
    }
    Ok(Json(SubmitBlockResp {
        accepted,
        height,
        id,
        credited_to: if accepted { Some(miner) } else { None },
        message: if accepted {
            "ok".into()
        } else {
            text
        },
    }))
}

async fn pool_stats(State(st): State<AppState>) -> Json<PoolStats> {
    let mut g = st.inner.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_secs();
    prune_work(
        &mut g.work_events,
        now.saturating_sub(RATE_WINDOW_SECS),
    );
    prune_jobs(&mut g.job_events, now.saturating_sub(RATE_WINDOW_SECS));
    let diff = g.last_difficulty.max(1);
    let pool_hr = rolling_hashrate(&g.work_events, now);
    // If pool is finding nearly every block, rolling HR is the live signal; else fall back
    // to consensus estimate so the card is never blank while templates are being served.
    let network_hr = hashes_for_diff(diff) / (TARGET_BLOCK_TIME_SECS.max(1) as f64);
    let pool_hashrate = if pool_hr > 0.0 {
        pool_hr
    } else if !g.job_events.is_empty() {
        // Active miners pulling jobs but no accept in window yet — show network estimate
        // scaled by connected miners presence (still non-zero for the UI).
        network_hr
    } else {
        0.0
    };
    Json(PoolStats {
        ok: true,
        pool_address: st.pool_address.clone(),
        payout_address: st
            .payout_override
            .clone()
            .unwrap_or_else(|| "miner".into()),
        upstream: st.upstream.clone(),
        blocks_found: g.blocks_found,
        jobs_served: g.jobs,
        credits: g.credits.clone(),
        pool_hashrate,
        network_hashrate: network_hr,
        jobs_per_second: jobs_per_second(&g.job_events, now),
        connected_miners: connected_miners(&g.miners_seen, now),
        block_height: g.last_height,
        difficulty: diff,
        recent_blocks: g
            .recent_blocks
            .iter()
            .take(100)
            .map(|b| {
                let (confirmations, mature, remain) = block_maturity(g.last_height, b.height);
                FoundBlockView {
                    height: b.height,
                    hash: b.hash.clone(),
                    miner: b.miner.clone(),
                    worker: b.worker.clone(),
                    created: b.created,
                    confirmations,
                    mature,
                    remain,
                }
            })
            .collect(),
        coinbase_maturity: COINBASE_MATURITY,
    })
}

#[derive(Deserialize)]
struct BlocksQuery {
    limit: Option<u64>,
}

async fn list_blocks(
    State(st): State<AppState>,
    Query(q): Query<BlocksQuery>,
) -> Json<serde_json::Value> {
    let g = st.inner.lock().unwrap_or_else(|e| e.into_inner());
    let limit = q.limit.unwrap_or(50).clamp(1, 250) as usize;
    let tip = g.last_height;
    let blocks: Vec<_> = g
        .recent_blocks
        .iter()
        .take(limit)
        .map(|b| {
            let (confirmations, mature, remain) = block_maturity(tip, b.height);
            FoundBlockView {
                height: b.height,
                hash: b.hash.clone(),
                miner: b.miner.clone(),
                worker: b.worker.clone(),
                created: b.created,
                confirmations,
                mature,
                remain,
            }
        })
        .collect();
    Json(serde_json::json!({
        "ok": true,
        "coinbase_maturity": COINBASE_MATURITY,
        "tip": tip,
        "blocks": blocks
    }))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "mesh-pool" }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let kp = load_or_create_key(&args.keyfile)?;
    let pool_address = kp.address().to_string();
    let payout_override = args
        .payout_address
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && Address::from_hex(s).is_some())
        .map(|s| s.to_string());
    if let Some(p) = payout_override.as_ref() {
        info!(%p, "pool coinbase override (all templates)");
    } else {
        info!("pool coinbase = miner wallet from ?address= / X-Mesh-Miner");
    }
    let credits = load_credits(&args.credits);
    let recent_blocks = load_blocks(&args.blocks);
    let blocks_found: u64 = if !recent_blocks.is_empty() {
        recent_blocks.len() as u64
    } else {
        credits.values().sum()
    };
    let last_height = recent_blocks.first().map(|b| b.height).unwrap_or(0);
    let payout_note = payout_override
        .clone()
        .unwrap_or_else(|| "miner-wallet".into());
    let state = AppState {
        upstream: args.upstream.trim_end_matches('/').to_string(),
        pool_address: pool_address.clone(),
        payout_override,
        credits_path: args.credits.clone(),
        blocks_path: args.blocks.clone(),
        inner: Arc::new(Mutex::new(PoolInner {
            blocks_found,
            credits,
            recent_blocks,
            jobs: 0,
            work_events: VecDeque::new(),
            job_events: VecDeque::new(),
            miners_seen: HashMap::new(),
            last_height,
            last_difficulty: 1,
        })),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/ai/health", get(health))
        .route("/v1/getblocktemplate", get(get_template))
        .route("/v1/submitblock", post(submit_block))
        .route("/v1/exam/submit", post(proxy_exam_submit))
        .route("/v1/exam/status", get(proxy_exam_status))
        .route("/v1/getrewards", get(proxy_getrewards))
        .route("/v1/getnodeinfo", get(proxy_getnodeinfo))
        .route("/v1/poolstats", get(pool_stats))
        .route("/v1/blocks", get(list_blocks))
        .layer(CorsLayer::permissive())
        .with_state(state);
    let addr: SocketAddr = args.bind.parse()?;
    info!(%addr, %pool_address, payout = %payout_note, upstream = %args.upstream, "mesh-pool listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::coinbase_address;

    #[test]
    fn pays_miner_wallet_not_pool_key() {
        let miner = "mesh01424ad9495486c59b60c21f43707ca05a26ef68ff.rig1";
        let pool = "mesh018f149ff546e6c612562135af2ce7963c9b6c44a5";
        assert_eq!(
            coinbase_address(miner, None, pool),
            "mesh01424ad9495486c59b60c21f43707ca05a26ef68ff"
        );
    }

    #[test]
    fn override_wins() {
        let miner = "mesh0190c94bfe941747b3f1190030417c55abb5463f3c.rig1";
        let pool = "mesh018f149ff546e6c612562135af2ce7963c9b6c44a5";
        let pay = "mesh01424ad9495486c59b60c21f43707ca05a26ef68ff";
        assert_eq!(coinbase_address(miner, Some(pay), pool), pay);
    }

    #[test]
    fn anonymous_falls_back() {
        let pool = "mesh018f149ff546e6c612562135af2ce7963c9b6c44a5";
        assert_eq!(coinbase_address("anonymous", None, pool), pool);
    }
}
