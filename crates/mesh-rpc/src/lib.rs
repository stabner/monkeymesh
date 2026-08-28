//! REST wallet RPC (`Build/09_WALLET_RPC.md`) + embedded AI job board.

mod ai_orch;
mod ai_proxy;
mod mining_activity;
mod rate_limit;
mod routes;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::extract::DefaultBodyLimit;
use axum::Router;
use mesh_ai::{JobQueue, SharedBrain};
use mesh_ai_v2::SharedBrainV2;
use mesh_ai::{LegBrainPack, QuantumBrainPack};
use mesh_crypto::Keypair;
use mesh_p2p::{NetworkHandle, SharedChain};
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use axum::http::{header, Method};
use tower_http::cors::{Any, CorsLayer};

/// Axum default is 2 MiB — too small for brain-v2 results (`weights` ~1.9 MiB → hex ~3.8 MiB JSON).
const RPC_BODY_LIMIT: usize = 32 * 1024 * 1024;

pub use mining_activity::{MiningActivity, MiningStatus, TemplateCache};
pub use routes::rpc_router;
use rate_limit::AiRateLimiter;

/// Arm sticky `$data_dir/ai.token` when `MESH_AI_TOKEN` is unset (Build/27 B4).
/// Default on (`MESH_AI_TOKEN_AUTO=1`); set `MESH_AI_TOKEN_AUTO=0` to keep the board open.
/// Returns the token when armed (also sets `MESH_AI_TOKEN` in the process env).
pub fn ensure_ai_token(data_dir: &Path) -> Option<String> {
    if let Ok(existing) = std::env::var("MESH_AI_TOKEN") {
        let t = existing.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    let auto = std::env::var("MESH_AI_TOKEN_AUTO").unwrap_or_else(|_| "1".into());
    let auto_on = auto == "1" || auto.eq_ignore_ascii_case("true");
    if !auto_on {
        return None;
    }
    let _ = std::fs::create_dir_all(data_dir);
    let path = data_dir.join("ai.token");
    let token = load_or_mint_cookie(&path);
    std::env::set_var("MESH_AI_TOKEN", &token);
    Some(token)
}

/// Bitcoin Core–style wallet RPC cookie (`$data_dir/rpc.token`).
///
/// Always armed unless `MESH_RPC_TOKEN` is already set. Wallet/gov routes
/// **fail closed** — they never run without a token (unlike the old optional gate).
/// Public mine (`getblocktemplate` / `submitblock`) stays open.
pub fn ensure_rpc_token(data_dir: &Path) -> String {
    if let Ok(existing) = std::env::var("MESH_RPC_TOKEN") {
        let t = existing.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    let _ = std::fs::create_dir_all(data_dir);
    let path = data_dir.join("rpc.token");
    let token = load_or_mint_cookie(&path);
    std::env::set_var("MESH_RPC_TOKEN", &token);
    token
}

fn load_or_mint_cookie(path: &Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let t = s.trim().to_string();
            if t.is_empty() {
                mint_cookie(path)
            } else {
                let _ = mesh_crypto::restrict_secret_file(path);
                t
            }
        }
        Err(_) => mint_cookie(path),
    }
}

fn mint_cookie(path: &Path) -> String {
    let token = mesh_crypto::mint_secret_hex();
    let _ = mesh_crypto::write_secret_file(path, token.as_bytes());
    token
}

#[derive(Clone)]
pub struct RpcState {
    pub chain: SharedChain,
    pub network: Option<NetworkHandle>,
    /// Hot wallet used for getnewaddress / sendtoaddress.
    pub wallet: Arc<RwLock<Keypair>>,
    pub wallet_path: PathBuf,
    /// Wallet / operator surface: when set, wallet+gov mutating routes need
    /// `Authorization: Bearer <token>` or `X-Mesh-Token`. Does **not** gate public mine or AI.
    pub rpc_token: Option<Arc<str>>,
    /// AI board mutate surface (`/v1/advertise|job|result`). When set, workers must send the same headers.
    /// When unset, AI stays open with rate limits (public testnet).
    pub ai_token: Option<Arc<str>>,
    /// Optional edge RPC URLs advertised in getnodeinfo (`MESH_RPC_EDGES`, comma-separated).
    pub rpc_edges: Vec<String>,
    /// When set (edge mode), AI HTTP routes are proxied here.
    pub ai_upstream: Option<String>,
    /// External miner template/submit activity for the node UI feed.
    pub mining: Arc<Mutex<MiningActivity>>,
    /// Cached getblocktemplate responses (per tip + payout address).
    pub templates: Arc<Mutex<TemplateCache>>,
    /// Built-in AI orchestrator (workers connect here).
    pub ai: Arc<AsyncMutex<JobQueue>>,
    /// Per-worker AI board rate limits.
    pub ai_limit: Arc<AiRateLimiter>,
}

impl RpcState {
    pub fn with_defaults(
        chain: SharedChain,
        network: Option<NetworkHandle>,
        wallet: Arc<RwLock<Keypair>>,
        wallet_path: PathBuf,
        rpc_token: Option<Arc<str>>,
    ) -> Self {
        let ai_token = std::env::var("MESH_AI_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(Arc::<str>::from);
        let rpc_edges = std::env::var("MESH_RPC_EDGES")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self::with_auth(
            chain,
            network,
            wallet,
            wallet_path,
            rpc_token,
            ai_token,
            rpc_edges,
        )
    }

    pub fn with_auth(
        chain: SharedChain,
        network: Option<NetworkHandle>,
        wallet: Arc<RwLock<Keypair>>,
        wallet_path: PathBuf,
        rpc_token: Option<Arc<str>>,
        ai_token: Option<Arc<str>>,
        rpc_edges: Vec<String>,
    ) -> Self {
        let ai_upstream = std::env::var("MESH_AI_UPSTREAM")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty());
        // Hybrid edge: local non-brain board + proxy brain (default when upstream set).
        let ai = if edge_ai_local_enabled(&ai_upstream) {
            tracing::info!("edge hybrid AI: local non-brain JobQueue (no shared brain/legs)");
            JobQueue::default()
        } else {
            let brain_path = wallet_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("shared_brain.bin");
            let brain = SharedBrain::load_or_genesis(&brain_path);
            tracing::info!(
                epoch = brain.epoch,
                path = %brain_path.display(),
                "shared network brain ready"
            );
            let brain_v2_path = wallet_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("shared_brain_v2.bin");
            let brain_v2 = SharedBrainV2::load_or_genesis(&brain_v2_path);
            tracing::info!(
                epoch = brain_v2.epoch,
                path = %brain_v2_path.display(),
                contract = mesh_ai_v2::BRAIN_CONTRACT,
                "shared network brain v2 ready"
            );
            let legs_path = wallet_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("leg_brains.bin");
            let legs = LegBrainPack::load_or_genesis(&legs_path);
            tracing::info!(
                path = %legs_path.display(),
                sec = legs.epoch(mesh_ai::LegId::Security),
                "trilemma guardian legs ready"
            );
            let quantum_path = wallet_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("quantum_brains.bin");
            let quantum = QuantumBrainPack::load_or_genesis(&quantum_path);
            tracing::info!(
                path = %quantum_path.display(),
                pqc = quantum.epoch(mesh_ai::QuantumId::Pqc),
                "quantum research guardians ready"
            );
            JobQueue::with_brains_legs_quantum(brain, brain_v2, legs, quantum)
        };
        Self {
            chain,
            network,
            wallet,
            wallet_path,
            rpc_token,
            ai_token,
            rpc_edges,
            ai_upstream,
            mining: Arc::new(Mutex::new(MiningActivity::default())),
            templates: Arc::new(Mutex::new(TemplateCache::default())),
            ai: Arc::new(AsyncMutex::new(ai)),
            ai_limit: Arc::new(AiRateLimiter::default()),
        }
    }
}

/// Explorer + public mine need browser CORS. Credentials stay off so a
/// malicious page cannot ride a cookie (Geth: never `*` + credentials).
fn rpc_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::HeaderName::from_static("x-mesh-token"),
        ])
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn edge_ai_local_enabled(ai_upstream: &Option<String>) -> bool {
    if !env_flag("MESH_EDGE_MODE") || ai_upstream.is_none() {
        return false;
    }
    std::env::var("MESH_EDGE_AI_LOCAL")
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
        .unwrap_or(true)
}

pub async fn serve_rpc(state: RpcState, bind: SocketAddr) -> Result<()> {
    let edge = env_flag("MESH_EDGE_MODE");
    let hybrid = edge_ai_local_enabled(&state.ai_upstream);

    let app = if edge {
        if hybrid {
            tracing::info!(
                upstream = ?state.ai_upstream,
                "EDGE hybrid: local non-brain board + brain proxied to seed"
            );
            ai_orch::spawn_ai_inbound(state.clone());
            ai_orch::spawn_research_tick(state.clone()).await;
            Router::new()
                .merge(rpc_router())
                .merge(ai_orch::ai_board_router())
                .merge(ai_proxy::ai_brain_proxy_router())
                .layer(DefaultBodyLimit::max(RPC_BODY_LIMIT))
                .layer(rpc_cors())
                .with_state(state)
        } else if state.ai_upstream.is_some() {
            tracing::info!(
                upstream = ?state.ai_upstream,
                "EDGE mode: mine RPC local + full AI proxied to seed (MESH_EDGE_AI_LOCAL=0)"
            );
            Router::new()
                .merge(rpc_router())
                .merge(ai_proxy::ai_proxy_router())
                .layer(DefaultBodyLimit::max(RPC_BODY_LIMIT))
                .layer(rpc_cors())
                .with_state(state)
        } else {
            tracing::info!("EDGE mode: chain/mine RPC only (set MESH_AI_UPSTREAM to proxy AI)");
            Router::new()
                .merge(rpc_router())
                .route(
                    "/v1/ai/health",
                    axum::routing::get(|| async {
                        axum::Json(serde_json::json!({
                            "ok": true,
                            "edge": true,
                            "note": "Set MESH_AI_UPSTREAM to proxy AI to seed",
                        }))
                    }),
                )
                .layer(DefaultBodyLimit::max(RPC_BODY_LIMIT))
                .layer(rpc_cors())
                .with_state(state)
        }
    } else {
        ai_orch::spawn_ai_inbound(state.clone());
        ai_orch::spawn_research_tick(state.clone()).await;
        Router::new()
            .merge(rpc_router())
            .merge(ai_orch::ai_router())
            .layer(DefaultBodyLimit::max(RPC_BODY_LIMIT))
            .layer(rpc_cors())
            .with_state(state)
    };

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, edge, "wallet RPC listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
