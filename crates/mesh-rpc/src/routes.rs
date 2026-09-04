use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use mesh_crypto::Keypair;
use mesh_types::{Address, Amount};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::RpcState;

fn bearer_or_mesh_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-mesh-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| {
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string()))
        })
}

/// Wallet / operator surface (`MESH_RPC_TOKEN` cookie). Fail-closed: no token = 401.
fn require_wallet_token(st: &RpcState, headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    let Some(expected) = st.rpc_token.as_ref() else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "wallet RPC token required (rpc.token cookie not armed)".into(),
        ));
    };
    match bearer_or_mesh_token(headers) {
        Some(t) if token_eq(&t, expected) => Ok(()),
        _ => Err((StatusCode::UNAUTHORIZED, "wallet RPC token required".into())),
    }
}

/// AI board mutate surface (`MESH_AI_TOKEN`). Independent of wallet token.
pub(crate) fn require_ai_token(st: &RpcState, headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    let Some(expected) = st.ai_token.as_ref() else {
        return Ok(());
    };
    match bearer_or_mesh_token(headers) {
        Some(t) if token_eq(&t, expected) => Ok(()),
        _ => Err((StatusCode::UNAUTHORIZED, "AI board token required".into())),
    }
}

/// Constant-time compare for variable-length tokens (pad to 128, fold length in).
fn token_eq(presented: &str, expected: &str) -> bool {
    use subtle::ConstantTimeEq;
    const N: usize = 128;
    let a = presented.as_bytes();
    let b = expected.as_bytes();
    let mut aa = [0u8; N];
    let mut bb = [0u8; N];
    let al = a.len().min(N);
    let bl = b.len().min(N);
    aa[..al].copy_from_slice(&a[..al]);
    bb[..bl].copy_from_slice(&b[..bl]);
    let body_ok = aa.ct_eq(&bb);
    let len_ok = (a.len() as u64).ct_eq(&(b.len() as u64));
    bool::from(body_ok & len_ok)
}

pub fn rpc_router() -> Router<RpcState> {
    Router::new()
        .route("/v1/getnodeinfo", get(get_node_info))
        .route("/v1/getbalance", get(get_balance))
        .route("/v1/noderewards", get(get_node_rewards))
        .route("/v1/nodeservices", get(get_node_services))
        .route("/v1/setoperator", post(set_operator))
        .route("/v1/getnewaddress", post(get_new_address))
        .route("/v1/sendtoaddress", post(send_to_address))
        .route("/v1/listtransactions", get(list_transactions))
        .route("/v1/getrewards", get(get_rewards))
        .route("/v1/utxos", get(list_utxos))
        .route("/v1/submittx", post(submit_tx))
        .route("/v1/mine", post(mine_blocks))
        .route("/v1/getblocktemplate", get(get_block_template))
        .route("/v1/submitblock", post(submit_block))
        .route("/v1/exam/submit", post(submit_exam))
        .route("/v1/exam/status", get(exam_status))
        .route("/v1/miningstatus", get(get_mining_status))
        .route("/v1/getblock", get(get_block))
        .route("/v1/mempool", get(get_mempool))
        .route("/v1/submitvote", post(submit_vote))
        .route("/v1/aireceipt", post(post_ai_receipt))
        .route("/v1/nodescore", post(post_node_score))
        .route("/v1/finality", get(get_finality))
        .route("/v1/finality/attest", post(post_finality_attest))
        .route("/v1/nodebond", get(get_node_bond).post(post_node_bond))
        .route("/v1/nodeunbond", post(post_node_unbond))
        .route("/v1/nodeslash", post(post_node_slash))
        .route("/v1/nodeslashsettle", post(post_node_slash_settle))
        .route("/v1/archive/info", get(get_archive_info))
        .route("/v1/archive/headers", get(get_archive_headers))
        .route("/v1/archive/blocks", get(get_archive_blocks))
        .route("/v1/gpuscores", get(get_gpu_scores))
        .route("/v1/markets", get(get_markets))
        .route("/v1/meshpulse", get(get_mesh_pulse))
        .route("/v1/proposals", get(list_proposals))
        .route("/v1/proposals/generate", post(generate_proposal))
        .route("/v1/proposals/activate", post(activate_proposal))
        .route("/v1/proposals/reject", post(reject_proposal))
        .route("/v1/proposals/vote", post(vote_proposal))
        .route("/v1/envelopes", get(get_envelopes))
        .route("/v1/snapshot", get(get_snapshot))
        .route("/v1/snapshot/download", get(get_snapshot_download))
        .route("/v1/snapshot/utxos", get(get_snapshot_utxos))
        .route("/v1/snapshot/pruneplan", get(get_prune_plan))
        .route("/v1/snapshot/prune", post(post_snapshot_prune))
        .route("/", get(explorer_page))
        .route("/explorer", get(explorer_page))
}

#[cfg(test)]
mod token_eq_tests {
    use super::token_eq;

    #[test]
    fn equal_tokens_match() {
        assert!(token_eq("abc", "abc"));
        assert!(token_eq(&"x".repeat(64), &"x".repeat(64)));
    }

    #[test]
    fn prefix_and_length_mismatch() {
        assert!(!token_eq("secret", "secre"));
        assert!(!token_eq("secret", "secret!"));
        assert!(!token_eq("", "x"));
    }
}

#[derive(Serialize)]
struct NodeInfo {
    height: u64,
    tip: String,
    genesis: String,
    next_difficulty: u32,
    /// Soft hint only (consensus difficulty ± activated bias). Not consensus-critical.
    soft_diff_hint: u32,
    blocks: usize,
    mempool: usize,
    peer_id: Option<String>,
    /// Connected P2P peers.
    peers: usize,
    /// Median libp2p ping RTT across connected peers (ms), if sampled.
    median_peer_rtt_ms: Option<u64>,
    /// Per-peer RTT samples (capped), lowest first.
    peer_rtts: Vec<PeerRtt>,
    /// Soft RTT dampener applied to local node-market credits (1000 = full).
    relay_rtt_factor_milli: u64,
    /// Hot wallet address (RPC send / getnewaddress).
    address: String,
    /// Node-market payout address (useful-work credits). Falls back to hot wallet.
    operator_address: String,
  /// Optional edge RPC base URLs (templates/AI load-split). Empty = seed-only.
    edges: Vec<String>,
    /// Local AI shard id (`MESH_AI_SHARD_ID`).
    ai_shard_id: u32,
    /// Board shard count (`MESH_AI_SHARD_COUNT`).
    ai_shard_count: u32,
    /// Reachable shard RPC bases (`MESH_AI_SHARDS` / defaults).
    ai_shard_urls: Vec<String>,
    /// Auth surfaces currently armed on this node.
    auth: AuthSurfaces,
    /// Advertised node services (archive/snapshot/ai_routing).
    services: Vec<String>,
    /// Cold-pruned (hot WAL only; bodies below hot_from_height dropped).
    pruned: bool,
    /// Lowest height still held locally in the hot WAL.
    hot_from_height: u64,
    /// UTXO checkpoint file present next to the chain store.
    utxo_ckpt: bool,
    /// `MESH_COLD_PRUNE` armed (POST /v1/snapshot/prune allowed).
    cold_prune_env: bool,
    /// `MESH_AUTO_PRUNE` armed (opt-in prune after append).
    auto_prune_env: bool,
    /// Effective keep window for prune plan (`MESH_KEEP_BLOCKS`, min 128).
    keep_blocks: u64,
    /// Consensus coinbase spend delay (blocks). Stamped on-chain as `|mat:20`.
    coinbase_maturity: u64,
    /// Hard cap in whole MESH (`mesh_types::SUPPLY_CAP_MESH`).
    supply_cap_mesh: u64,
    /// Atomic units minted by blocks `0 ..= height` (this tip included).
    emitted_atomic: String,
    /// Lab economic finality (Build/36 F2). 0 = nothing locked.
    finalized_height: u64,
    finalized_hash: String,
    /// True only when `MESH_FINALITY_HEIGHT` is reached (default off).
    finality_active: bool,
}

#[derive(Serialize)]
struct PeerRtt {
    peer_id: String,
    rtt_ms: u64,
}

#[derive(Serialize)]
struct AuthSurfaces {
    /// Wallet/gov token required (`MESH_RPC_TOKEN`).
    wallet: bool,
    /// AI board token required (`MESH_AI_TOKEN`).
    ai: bool,
    /// Public mine (`getblocktemplate` / `submitblock`) is always open.
    mine_public: bool,
}

async fn get_node_info(State(st): State<RpcState>) -> Json<NodeInfo> {
    let address = {
        let w = st.wallet.read().await;
        w.address().to_string()
    };
    // Keep the chain lock short — never call network helpers while holding it
    // (lock-order stalls with P2P RTT maps were multi-second getnodeinfo delays).
    let (
        blocks,
        height,
        pruned,
        hot_from_height,
        tip,
        genesis,
        next_difficulty,
        soft_diff_hint,
        mempool,
        relay_rtt_factor_milli,
        utxo_ckpt,
        operator_address,
        finalized_height,
        finalized_hash,
        finality_active,
    ) = {
        let c = st.chain.lock().await;
        let blocks = c.store().len();
        let height = c.height();
        let pruned = c.store().is_pruned();
        let hot_from_height = c.store().hot_from_height();
        let utxo_ckpt = c.store().path().with_extension("utxo.ckpt").exists();
        let operator_address = c
            .node_operator
            .map(|a| a.to_string())
            .unwrap_or_else(|| address.clone());
        (
            blocks,
            height,
            pruned,
            hot_from_height,
            c.tip_hash().to_string(),
            c.genesis_hash().to_string(),
            c.next_difficulty(),
            c.soft_mining_diff_hint(),
            c.mempool().len(),
            c.relay_rtt_factor_milli,
            utxo_ckpt,
            operator_address,
            c.finalized_height(),
            c.finalized_hash().to_hex(),
            mesh_chain::finality_active_at(height),
        )
    };
    let full_history =
        !pruned && blocks > 0 && blocks as u64 == height.saturating_add(1);
    let mut services = vec!["tx_relay".into(), "block_relay".into()];
    if full_history {
        services.push("archive".into());
        services.push("snapshot".into());
    }
    if st.ai_upstream.is_some()
        || !std::env::var("MESH_EDGE_MODE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    {
        services.push("ai_routing".into());
    }
    let cold_prune_env = std::env::var("MESH_COLD_PRUNE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let auto_prune_env = std::env::var("MESH_AUTO_PRUNE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let keep_blocks = std::env::var("MESH_KEEP_BLOCKS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(2_048u64)
        .max(mesh_chain::MIN_COLD_PRUNE_KEEP);
    let (ai_shard_id, ai_shard_count) = mesh_types::local_ai_shard_config();
    let ai_shard_urls = mesh_types::ai_shard_urls(ai_shard_count);
    let median_peer_rtt_ms = st.network.as_ref().and_then(|n| n.median_peer_rtt_ms());
    let peer_rtts: Vec<PeerRtt> = st
        .network
        .as_ref()
        .map(|n| {
            n.peer_rtt_snapshot()
                .into_iter()
                .take(16)
                .map(|(peer_id, rtt_ms)| PeerRtt { peer_id, rtt_ms })
                .collect()
        })
        .unwrap_or_default();
    Json(NodeInfo {
        height,
        tip,
        genesis,
        next_difficulty,
        soft_diff_hint,
        blocks,
        mempool,
        peer_id: st.network.as_ref().map(|n| n.local_peer_id.to_string()),
        peers: st.network.as_ref().map(|n| n.peer_count()).unwrap_or(0),
        median_peer_rtt_ms,
        peer_rtts,
        relay_rtt_factor_milli,
        address,
        operator_address,
        edges: st.rpc_edges.clone(),
        ai_shard_id,
        ai_shard_count,
        ai_shard_urls,
        auth: AuthSurfaces {
            wallet: st.rpc_token.is_some(),
            ai: st.ai_token.is_some(),
            mine_public: true,
        },
        services,
        pruned,
        hot_from_height,
        utxo_ckpt,
        cold_prune_env,
        auto_prune_env,
        keep_blocks,
        coinbase_maturity: mesh_types::COINBASE_MATURITY,
        supply_cap_mesh: mesh_types::SUPPLY_CAP_MESH,
        emitted_atomic: {
            let next = height.saturating_add(1);
            mesh_chain::emitted_before_atomic(next).to_string()
        },
        finalized_height,
        finalized_hash,
        finality_active,
    })
}

#[derive(Deserialize)]
struct AddressQuery {
    address: Option<String>,
}

#[derive(Serialize)]
struct BalanceResp {
    address: String,
    balance: String,
    atomic: u64,
    spendable: String,
    spendable_atomic: u64,
    immature: String,
    immature_atomic: u64,
}

async fn get_balance(
    State(st): State<RpcState>,
    Query(q): Query<AddressQuery>,
) -> Result<Json<BalanceResp>, (StatusCode, String)> {
    let addr = resolve_address(&st, q.address).await?;
    let c = st.chain.lock().await;
    let bal = c.balance(&addr);
    let spendable = c.mature_balance(&addr);
    let immature = bal.checked_sub(spendable).unwrap_or(Amount::ZERO);
    Ok(Json(BalanceResp {
        address: addr.to_string(),
        balance: bal.to_string(),
        atomic: bal.atomic(),
        spendable: spendable.to_string(),
        spendable_atomic: spendable.atomic(),
        immature: immature.to_string(),
        immature_atomic: immature.atomic(),
    }))
}

#[derive(Serialize)]
struct NodeRewardsResp {
    address: String,
    balance: String,
    balance_atomic: u64,
    spendable_atomic: u64,
    pending_weight: u64,
    pending_total_weight: u64,
    /// Rough share of next node-market coinbase (atomic), 0 if unknown.
    estimated_share_atomic: u64,
    estimated_share: String,
    peers: usize,
    bonded: bool,
    bond_eligible: bool,
    bond_locked_atomic: u64,
    bond_unlock_after_height: u64,
    bond_slashed: bool,
    min_bond_atomic: u64,
    peer_id: Option<String>,
    /// Soft reputation milli from attestation diversity (1000 = full).
    reputation_milli: u64,
    /// Soft RTT dampener for local credits (1000 = full).
    relay_rtt_factor_milli: u64,
}

async fn get_node_rewards(
    State(st): State<RpcState>,
    Query(q): Query<AddressQuery>,
) -> Result<Json<NodeRewardsResp>, (StatusCode, String)> {
    let addr = resolve_operator_or_wallet(&st, q.address).await?;
    let c = st.chain.lock().await;
    let bal = c.balance(&addr);
    let pending_weight = c.pending_node_weight(&addr);
    let pending_total_weight: u64 = c.store().node_scores().values().sum();
    let height = c.height().saturating_add(1);
    let node_pool = mesh_chain::node_market_reward(height);
    let estimated = if pending_total_weight == 0 || pending_weight == 0 {
        Amount::ZERO
    } else {
        Amount::from_atomic(
            (node_pool.atomic() as u128)
                .saturating_mul(pending_weight as u128)
                .saturating_div(pending_total_weight as u128) as u64,
        )
    };
    let bond = c.node_bond(&addr);
    let spendable = c.spendable_balance(&addr);
    let reputation_milli = c.node_reputation_milli(&addr);
    let relay_rtt_factor_milli = c.relay_rtt_factor_milli;
    Ok(Json(NodeRewardsResp {
        address: addr.to_string(),
        balance: bal.to_string(),
        balance_atomic: bal.atomic(),
        spendable_atomic: spendable.atomic(),
        pending_weight,
        pending_total_weight,
        estimated_share_atomic: estimated.atomic(),
        estimated_share: estimated.to_string(),
        peers: st.network.as_ref().map(|n| n.peer_count()).unwrap_or(0),
        bonded: bond
            .as_ref()
            .map(|b| !b.slashed && b.unlock_after_height == 0 && b.locked_atomic() > 0)
            .unwrap_or(false),
        bond_eligible: c.is_node_bond_eligible(&addr),
        bond_locked_atomic: bond.as_ref().map(|b| b.locked_atomic()).unwrap_or(0),
        bond_unlock_after_height: bond.as_ref().map(|b| b.unlock_after_height).unwrap_or(0),
        bond_slashed: bond.as_ref().map(|b| b.slashed).unwrap_or(false),
        min_bond_atomic: mesh_chain::MIN_NODE_BOND_ATOMIC,
        peer_id: bond.map(|b| b.peer_id),
        reputation_milli,
        relay_rtt_factor_milli,
    }))
}

async fn get_node_services(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    let recent: Vec<_> = c
        .recent_service_attestations()
        .into_iter()
        .rev()
        .take(32)
        .map(|a| {
            serde_json::json!({
                "operator": a.operator.to_string(),
                "service": a.service.as_str(),
                "weight": a.weight,
                "credited": a.credited,
                "bps": a.service.weight_bps(),
                "attested_at": a.attested_at,
            })
        })
        .collect();
    let weights = serde_json::json!({
        "tx_relay": mesh_types::NodeServiceKind::TxRelay.weight_bps(),
        "block_relay": mesh_types::NodeServiceKind::BlockRelay.weight_bps(),
        "snapshot": mesh_types::NodeServiceKind::Snapshot.weight_bps(),
        "archive": mesh_types::NodeServiceKind::Archive.weight_bps(),
        "ai_routing": mesh_types::NodeServiceKind::AiRouting.weight_bps(),
    });
    let local_rep = c
        .node_operator
        .map(|op| c.node_reputation_milli(&op))
        .unwrap_or(0);
    let relay_rtt_factor_milli = c.relay_rtt_factor_milli;
    Json(serde_json::json!({
        "weights_bps": weights,
        "recent": recent,
        "reputation_milli": local_rep,
        "relay_rtt_factor_milli": relay_rtt_factor_milli,
        "reputation_scale": {
            "empty": 0,
            "one_kind": 600,
            "two_kinds": 800,
            "three_plus": 1000
        },
        "rtt_scale_ms": {
            "none": 1000,
            "le_50": 1000,
            "le_200": 850,
            "gt_200": 700
        },
        "note": "Paid only for attested useful work (relay / AI routing / snapshot / archive). Idle nodes get 0. credited = weight × service_bps/1000 × idle_stipend_cap/1000 × reputation_milli/1000 × relay_rtt_factor_milli/1000",
    }))
}

#[derive(Serialize)]
struct AddressResp {
    address: String,
}

async fn get_new_address(
    State(st): State<RpcState>,
    headers: HeaderMap,
) -> Result<Json<AddressResp>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    if st.wallet_path.exists() {
        let kp = st.wallet.read().await;
        return Ok(Json(AddressResp {
            address: kp.address().to_string(),
        }));
    }
    let kp = Keypair::generate();
    let addr = kp.address();
    if let Some(parent) = st.wallet_path.parent() {
        std::fs::create_dir_all(parent).map_err(internal)?;
    }
    mesh_crypto::write_secret_file_no_clobber(&st.wallet_path, kp.to_hex().as_bytes())
        .map_err(internal)?;
    *st.wallet.write().await = kp;
    Ok(Json(AddressResp {
        address: addr.to_string(),
    }))
}

#[derive(Deserialize)]
struct SendReq {
    address: String,
    amount: String,
    #[serde(default)]
    memo: String,
}

#[derive(Serialize)]
struct SendResp {
    txid: String,
}

async fn send_to_address(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(req): Json<SendReq>,
) -> Result<Json<SendResp>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let to = Address::from_hex(&req.address).ok_or_else(|| bad("bad address"))?;
    let amount = Amount::parse_mesh(&req.amount).ok_or_else(|| bad("bad amount"))?;
    let kp = st.wallet.read().await.clone();
    let txid = {
        let mut c = st.chain.lock().await;
        c.send(&kp, to, amount, req.memo).map_err(|e| bad(e.to_string()))?
    };
    // Re-fetch tx for gossip if present in mempool
    if let Some(net) = &st.network {
        let c = st.chain.lock().await;
        if let Some(tx) = c.mempool().iter().find(|t| t.txid() == txid).cloned() {
            net.announce_tx(tx);
        }
    }
    Ok(Json(SendResp {
        txid: txid.to_string(),
    }))
}

#[derive(Serialize)]
struct TxItem {
    height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<u64>,
    txid: String,
    memo: String,
    outputs: Vec<OutItem>,
    in_mempool: bool,
}

#[derive(Serialize)]
struct OutItem {
    address: String,
    amount: String,
    atomic: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    lane: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paid_for: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vault: Option<bool>,
}

fn out_item(tx: &mesh_types::Transaction, vout: usize, o: &mesh_types::TxOutput) -> OutItem {
    let n = tx.outputs.len();
    let lab = if tx.is_coinbase() {
        Some(mesh_types::coinbase_payout_label(
            &tx.memo,
            vout as u32,
            n,
            Some(&o.address),
        ))
    } else {
        None
    };
    OutItem {
        address: o.address.to_string(),
        amount: o.amount.to_string(),
        atomic: o.amount.atomic(),
        lane: lab.map(|l| l.lane.as_str().to_string()),
        title: lab.map(|l| l.title.to_string()),
        paid_for: lab.map(|l| l.paid_for.to_string()),
        vault: lab.and_then(|l| l.vault.then_some(true)),
    }
}

async fn list_transactions(
    State(st): State<RpcState>,
    Query(q): Query<AddressQuery>,
) -> Result<Json<Vec<TxItem>>, (StatusCode, String)> {
    let addr = resolve_address(&st, q.address).await?;
    let c = st.chain.lock().await;
    let mut out = Vec::new();
    for h in 0..=c.height() {
        if let Some(block) = c.get_block(h) {
            for tx in &block.txs {
                if tx_involves(tx, &addr) {
                    out.push(tx_item(tx, Some(h), Some(block.header.timestamp), false));
                }
            }
        }
    }
    for tx in c.mempool() {
        if tx_involves(tx, &addr) {
            out.push(tx_item(tx, None, None, true));
        }
    }
    Ok(Json(out))
}

#[derive(Serialize)]
struct LaneTotal {
    lane: String,
    title: String,
    paid_for: String,
    amount: String,
    atomic: u64,
    count: u64,
}

#[derive(Serialize)]
struct RewardHit {
    height: u64,
    timestamp: u64,
    txid: String,
    vout: u32,
    amount: String,
    atomic: u64,
    lane: String,
    title: String,
    paid_for: String,
    vault: bool,
    mature: bool,
    confirmations: u64,
}

#[derive(Serialize)]
struct RewardsResp {
    address: String,
    rewards: String,
    atomic: u64,
    by_lane: Vec<LaneTotal>,
    recent: Vec<RewardHit>,
}

async fn get_rewards(
    State(st): State<RpcState>,
    Query(q): Query<AddressQuery>,
) -> Result<Json<RewardsResp>, (StatusCode, String)> {
    let addr = resolve_address(&st, q.address).await?;
    let c = st.chain.lock().await;
    let tip = c.height();
    let mut total = Amount::ZERO;
    let mut lane_map: std::collections::BTreeMap<
        &'static str,
        (mesh_types::CoinbaseLane, u64, u64),
    > = std::collections::BTreeMap::new();
    let mut hits: Vec<RewardHit> = Vec::new();
    for h in 0..=tip {
        if let Some(block) = c.get_block(h) {
            if let Some(cb) = block.txs.first() {
                if cb.is_coinbase() {
                    let n = cb.outputs.len();
                    for (i, o) in cb.outputs.iter().enumerate() {
                        if o.address != addr {
                            continue;
                        }
                        total = total.checked_add(o.amount).unwrap_or(total);
                        let lab = mesh_types::coinbase_payout_label(
                            &cb.memo,
                            i as u32,
                            n,
                            Some(&o.address),
                        );
                        let e = lane_map.entry(lab.lane.as_str()).or_insert((lab.lane, 0, 0));
                        e.1 = e.1.saturating_add(o.amount.atomic());
                        e.2 = e.2.saturating_add(1);
                        let confirmations = tip.saturating_sub(h).saturating_add(1);
                        hits.push(RewardHit {
                            height: h,
                            timestamp: block.header.timestamp,
                            txid: cb.txid().to_string(),
                            vout: i as u32,
                            amount: o.amount.to_string(),
                            atomic: o.amount.atomic(),
                            lane: lab.lane.as_str().to_string(),
                            title: lab.title.to_string(),
                            paid_for: lab.paid_for.to_string(),
                            vault: lab.vault,
                            mature: tip.saturating_add(1)
                                >= h.saturating_add(mesh_chain::COINBASE_MATURITY),
                            confirmations,
                        });
                    }
                }
            }
        }
    }
    hits.reverse();
    hits.truncate(48);
    let by_lane = lane_map
        .into_iter()
        .map(|(_, (lane, atomic, count))| LaneTotal {
            lane: lane.as_str().to_string(),
            title: lane.title().to_string(),
            paid_for: lane.paid_for().to_string(),
            amount: Amount::from_atomic(atomic).to_string(),
            atomic,
            count,
        })
        .collect();
    Ok(Json(RewardsResp {
        address: addr.to_string(),
        rewards: total.to_string(),
        atomic: total.atomic(),
        by_lane,
        recent: hits,
    }))
}

#[derive(Serialize)]
struct UtxoItem {
    txid: String,
    vout: u32,
    amount: String,
    atomic: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u64>,
    confirmations: u64,
    mature: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    lane: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paid_for: Option<String>,
}

async fn list_utxos(
    State(st): State<RpcState>,
    Query(q): Query<AddressQuery>,
) -> Result<Json<Vec<UtxoItem>>, (StatusCode, String)> {
    let addr = resolve_address(&st, q.address).await?;
    let c = st.chain.lock().await;
    let tip = c.height();
    let cbs = c.recent_coinbase_heights();
    let utxos = c.utxos_for(&addr);
    Ok(Json(
        utxos
            .into_iter()
            .map(|(op, u)| {
                let mined_at = cbs.get(&op).copied();
                let confirmations = mined_at
                    .map(|h| tip.saturating_sub(h).saturating_add(1))
                    .unwrap_or(mesh_chain::COINBASE_MATURITY);
                let mature = mined_at
                    .map(|h| {
                        tip.saturating_add(1) >= h.saturating_add(mesh_chain::COINBASE_MATURITY)
                    })
                    .unwrap_or(true)
                    && !c.store().is_outpoint_locked(&op);
                let (lane, title, paid_for) = mined_at
                    .and_then(|h| c.get_block(h))
                    .and_then(|b| b.txs.first().cloned())
                    .filter(|cb| cb.is_coinbase())
                    .map(|cb| {
                        let lab = mesh_types::coinbase_payout_label(
                            &cb.memo,
                            op.vout,
                            cb.outputs.len(),
                            Some(&u.address),
                        );
                        (
                            Some(lab.lane.as_str().to_string()),
                            Some(lab.title.to_string()),
                            Some(lab.paid_for.to_string()),
                        )
                    })
                    .unwrap_or((None, None, None));
                UtxoItem {
                    txid: op.txid.to_string(),
                    vout: op.vout,
                    amount: u.amount.to_string(),
                    atomic: u.amount.atomic(),
                    height: mined_at,
                    confirmations,
                    mature,
                    lane,
                    title,
                    paid_for,
                }
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct SubmitTxReq {
    /// Bincode-serialized [`mesh_types::Transaction`] as hex.
    tx_hex: String,
}

async fn submit_tx(
    State(st): State<RpcState>,
    Json(req): Json<SubmitTxReq>,
) -> Result<Json<SendResp>, (StatusCode, String)> {
    // Public mempool surface — already signed; wallet token must not block relays (N4).
    if let Err(ms) = st
        .ai_limit
        .check("submittx", 120, std::time::Duration::from_secs(60))
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("submittx rate limit — retry in {ms}ms"),
        ));
    }
    let bytes = hex::decode(req.tx_hex.trim()).map_err(|e| bad(format!("bad tx_hex: {e}")))?;
    let tx: mesh_types::Transaction =
        bincode::deserialize(&bytes).map_err(|e| bad(format!("bad tx: {e}")))?;
    let txid = {
        let mut c = st.chain.lock().await;
        c.submit_tx(tx.clone()).map_err(|e| bad(e.to_string()))?
    };
    invalidate_templates(&st);
    if let Some(net) = &st.network {
        net.announce_tx(tx);
    }
    Ok(Json(SendResp {
        txid: txid.to_string(),
    }))
}

#[derive(Deserialize)]
struct MineReq {
    /// Number of blocks to mine (default 1).
    #[serde(default = "default_mine_blocks")]
    blocks: u64,
    /// Coinbase payout address (defaults to node hot wallet).
    address: Option<String>,
    #[serde(default = "default_max_nonces")]
    max_nonces: u64,
}

fn default_mine_blocks() -> u64 {
    1
}
fn default_max_nonces() -> u64 {
    5_000_000
}

#[derive(Serialize)]
struct MinedBlock {
    height: u64,
    id: String,
    difficulty: u32,
    nonce: u64,
    txs: usize,
}

#[derive(Serialize)]
struct MineResp {
    mined: Vec<MinedBlock>,
    height: u64,
    tip: String,
}

async fn mine_blocks(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(req): Json<MineReq>,
) -> Result<Json<MineResp>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    if req.blocks == 0 {
        return Err(bad("blocks must be > 0"));
    }
    if req.blocks > 32 {
        return Err(bad("blocks capped at 32 per request"));
    }
    if req.max_nonces > 50_000_000 {
        return Err(bad("max_nonces capped at 50000000"));
    }
    let miner = resolve_address(&st, req.address).await?;
    let mut mined = Vec::new();

    for _ in 0..req.blocks {
        let mut accepted: Option<mesh_types::Block> = None;
        // Auto-mine on the node can race the tip; retry a few times.
        for attempt in 0..8u32 {
            let (mut block, light_pow) = {
                let c = st.chain.lock().await;
                (c.mining_template(miner), c.light_pow)
            };
            let max_nonces = req.max_nonces;
            let found = tokio::task::spawn_blocking(move || {
                let ok = mesh_chain::Chain::search_pow(&mut block, light_pow, max_nonces);
                (ok, block)
            })
            .await
            .map_err(|e| bad(e.to_string()))?;

            let (ok, block) = found;
            if !ok {
                return Err(bad(format!(
                    "no PoW solution within {} nonces",
                    req.max_nonces
                )));
            }

            let result = {
                let mut c = st.chain.lock().await;
                c.accept_mined(block).map_err(|e| bad(e.to_string()))?
            };
            match result {
                Some(block) => {
                    accepted = Some(block);
                    break;
                }
                None => {
                    tracing::debug!(attempt, "mine race: tip moved, retrying");
                    tokio::task::yield_now().await;
                }
            }
        }
        let block = accepted.ok_or_else(|| bad("tip kept moving while mining; retry"))?;
        invalidate_templates(&st);
        if let Some(net) = &st.network {
            net.announce_block(block.clone());
        }
        mined.push(MinedBlock {
            height: block.header.height,
            id: block.id().to_string(),
            difficulty: block.header.difficulty,
            nonce: block.header.nonce,
            txs: block.txs.len(),
        });
    }

    let (height, tip) = {
        let c = st.chain.lock().await;
        (c.height(), c.tip_hash().to_string())
    };
    Ok(Json(MineResp { mined, height, tip }))
}

#[derive(Serialize)]
struct BlockTemplateResp {
    height: u64,
    /// Consensus difficulty the share must meet.
    difficulty: u32,
    /// Soft hint from research epochs (informational; does not change validation).
    soft_diff_hint: u32,
    light_pow: bool,
    /// MeshHash profile version (`1`, `2`, or `3` Evo). Miners must match this.
    pow_version: u8,
    /// MeshHash-Evo recipe id hex (empty before v3).
    #[serde(default)]
    pow_recipe: String,
    /// Protocol-assigned role for this miner address (Build/31).
    #[serde(default)]
    assigned_role: String,
    /// Windowed mesh strength (receipts + height).
    #[serde(default)]
    mesh_strength: u64,
    address: String,
    /// Unsolved block (nonce = 0) as hex(bincode).
    block_hex: String,
    /// Immune exam (fair-split era). Empty before the gate.
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
    /// Consensus coinbase spend delay (blocks).
    coinbase_maturity: u64,
}

fn exam_template_fields(
    height: u64,
    prev_hash: mesh_types::Hash,
    miner: &mesh_types::Address,
) -> (String, String, String, String, String, bool, u16, u16, u16) {
    let fair = mesh_types::fair_lane_split_active(height);
    let cpu_bps = mesh_types::cpu_market_bps_at(height);
    let gpu_bps = mesh_types::gpu_market_bps_at(height);
    let node_bps = mesh_types::node_market_bps_at(height);
    if !fair {
        return (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            false,
            cpu_bps,
            gpu_bps,
            node_bps,
        );
    }
    let exam = mesh_ai::assign_exam(height, &prev_hash, miner);
    (
        exam.exam_root.to_hex(),
        exam.scenario.as_str().to_string(),
        exam.title().to_string(),
        exam.payload_hex(),
        exam.job_id(miner),
        true,
        cpu_bps,
        gpu_bps,
        node_bps,
    )
}

fn invalidate_templates(st: &RpcState) {
    if let Ok(mut cache) = st.templates.lock() {
        cache.clear();
    }
}

/// Used by AI board after score-affecting receipts.
pub fn invalidate_templates_pub(st: &RpcState) {
    invalidate_templates(st);
}

fn gbt_ip_limit() -> u32 {
    std::env::var("MESH_GBT_IP_LIMIT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(240)
        .max(1)
}

fn gbt_global_limit() -> u32 {
    std::env::var("MESH_GBT_GLOBAL_LIMIT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(6_000)
        .max(1)
}

fn client_ip_for_mine(headers: &HeaderMap, connect: Option<&SocketAddr>) -> String {
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

async fn get_block_template(
    State(st): State<RpcState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<AddressQuery>,
) -> Result<Json<BlockTemplateResp>, (StatusCode, HeaderMap, String)> {
    let ip = client_ip_for_mine(&headers, Some(&addr));
    let window = std::time::Duration::from_secs(60);
    if let Err(ms) = st.ai_limit.check_all(
        &[
            (&format!("gbt:ip:{ip}"), gbt_ip_limit()),
            ("gbt:global", gbt_global_limit()),
        ],
        window,
    ) {
        let mut h = HeaderMap::new();
        let secs = ((ms + 999) / 1000).max(1);
        if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
            h.insert(header::RETRY_AFTER, v);
        }
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            h,
            format!("getblocktemplate rate limit — retry in {ms}ms"),
        ));
    }
    let addr = resolve_address(&st, q.address)
        .await
        .map_err(|(c, m)| (c, HeaderMap::new(), m))?;
    let addr_s = addr.to_string();
    // Single tip-lock: tip + mempool fp + build (avoids stale tip/mempool races).
    let (tip, mempool_fp, soft_diff_hint, scores_epoch, block, light_pow, pow_version, difficulty, pow_recipe, assigned_role, mesh_strength) = {
        let c = st.chain.lock().await;
        let tip = c.tip_hash().to_string();
        let mempool_fp = c.mempool_fingerprint();
        let soft_diff_hint = c.soft_mining_diff_hint();
        let scores_epoch = c.scores_epoch();
        if let Ok(mut cache) = st.templates.lock() {
            if let Some(hit) = cache.get(&tip, mempool_fp, soft_diff_hint, scores_epoch, &addr_s)
            {
                drop(cache);
                drop(c);
                if let Ok(mut m) = st.mining.lock() {
                    m.note_template(&addr_s, hit.height);
                }
                return Ok(Json(BlockTemplateResp {
                    height: hit.height,
                    difficulty: hit.difficulty,
                    soft_diff_hint: hit.soft_diff_hint,
                    light_pow: hit.light_pow,
                    pow_version: hit.pow_version,
                    pow_recipe: hit.pow_recipe,
                    assigned_role: hit.assigned_role,
                    mesh_strength: hit.mesh_strength,
                    address: hit.address,
                    block_hex: hit.block_hex,
                    exam_root: hit.exam_root,
                    exam_scenario: hit.exam_scenario,
                    exam_title: hit.exam_title,
                    exam_payload_hex: hit.exam_payload_hex,
                    exam_job_id: hit.exam_job_id,
                    fair_split: hit.fair_split,
                    cpu_bps: hit.cpu_bps,
                    gpu_bps: hit.gpu_bps,
                    node_bps: hit.node_bps,
                    coinbase_maturity: mesh_types::COINBASE_MATURITY,
                }));
            }
        }
        let b = c.mining_template(addr);
        let pow_version = c.pow_version_at_height(b.header.height);
        let recipe = c.evo_recipe_at(b.header.height);
        let pow_recipe = recipe.as_ref().map(|r| r.to_hex()).unwrap_or_default();
        let assigned_role = "pow_cpu".to_string();
        let mesh_strength = c.mesh_strength();
        (
            tip,
            mempool_fp,
            soft_diff_hint,
            scores_epoch,
            b.clone(),
            c.light_pow,
            pow_version,
            b.header.difficulty,
            pow_recipe,
            assigned_role,
            mesh_strength,
        )
    };
    let bytes = bincode::serialize(&block).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new(), e.to_string()))?;
    let block_hex = hex::encode(bytes);
    let (exam_root, exam_scenario, exam_title, exam_payload_hex, exam_job_id, fair_split, cpu_bps, gpu_bps, node_bps) =
        exam_template_fields(block.header.height, block.header.prev_hash, &addr);
    if let Ok(mut cache) = st.templates.lock() {
        cache.put(
            &tip,
            mempool_fp,
            soft_diff_hint,
            scores_epoch,
            crate::mining_activity::CachedTemplate {
                height: block.header.height,
                difficulty,
                soft_diff_hint,
                light_pow,
                pow_version,
                pow_recipe: pow_recipe.clone(),
                assigned_role: assigned_role.clone(),
                mesh_strength,
                address: addr_s.clone(),
                block_hex: block_hex.clone(),
                exam_root: exam_root.clone(),
                exam_scenario: exam_scenario.clone(),
                exam_title: exam_title.clone(),
                exam_payload_hex: exam_payload_hex.clone(),
                exam_job_id: exam_job_id.clone(),
                fair_split,
                cpu_bps,
                gpu_bps,
                node_bps,
            },
        );
    }
    if let Ok(mut m) = st.mining.lock() {
        m.note_template(&addr_s, block.header.height);
    }
    Ok(Json(BlockTemplateResp {
        height: block.header.height,
        difficulty,
        soft_diff_hint,
        light_pow,
        pow_version,
        pow_recipe,
        assigned_role,
        mesh_strength,
        address: addr_s,
        block_hex,
        exam_root,
        exam_scenario,
        exam_title,
        exam_payload_hex,
        exam_job_id,
        fair_split,
        cpu_bps,
        gpu_bps,
        node_bps,
        coinbase_maturity: mesh_types::COINBASE_MATURITY,
    }))
}

#[derive(Deserialize)]
struct SubmitBlockReq {
    block_hex: String,
}

#[derive(Serialize)]
struct SubmitBlockResp {
    accepted: bool,
    height: u64,
    id: String,
}

async fn submit_block(
    State(st): State<RpcState>,
    Json(req): Json<SubmitBlockReq>,
) -> Result<Json<SubmitBlockResp>, (StatusCode, String)> {
    // Public mine surface — never gated by MESH_RPC_TOKEN (B5/N4).
    if let Err(ms) = st
        .ai_limit
        .check("submitblock", 240, std::time::Duration::from_secs(60))
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("submitblock rate limit — retry in {ms}ms"),
        ));
    }
    let bytes = hex::decode(req.block_hex.trim()).map_err(|e| bad(format!("bad hex: {e}")))?;
    let block: mesh_types::Block =
        bincode::deserialize(&bytes).map_err(|e| bad(format!("bad block: {e}")))?;
    let accepted = {
        let mut c = st.chain.lock().await;
        if mesh_types::exam_required_for_block(block.header.height) {
            let finder = block
                .txs
                .first()
                .and_then(|tx| tx.outputs.first())
                .map(|o| o.address);
            let Some(finder) = finder else {
                return Err(bad("submitblock missing coinbase finder"));
            };
            if !c.has_exam_receipt(block.header.height, &finder) {
                return Err(bad(
                    "exam MATCH required before submitblock — run the immune exam sidecar",
                ));
            }
        }
        c.accept_mined(block.clone())
            .map_err(|e| bad(e.to_string()))?
    };
    match accepted {
        Some(block) => {
            invalidate_templates(&st);
            let payout = block
                .txs
                .first()
                .and_then(|tx| tx.outputs.first())
                .map(|o| o.address.to_string())
                .unwrap_or_default();
            if let Ok(mut m) = st.mining.lock() {
                m.note_block_found(&payout, block.header.height);
            }
            if let Some(net) = &st.network {
                net.announce_block(block.clone());
            }
            Ok(Json(SubmitBlockResp {
                accepted: true,
                height: block.header.height,
                id: block.id().to_string(),
            }))
        }
        None => {
            let payout = block
                .txs
                .first()
                .and_then(|tx| tx.outputs.first())
                .map(|o| o.address.to_string())
                .unwrap_or_default();
            if let Ok(mut m) = st.mining.lock() {
                m.note_stale_submit(&payout, block.header.height);
            }
            Err(bad("stale block (tip moved); fetch a new template"))
        }
    }
}

#[derive(Deserialize)]
struct MiningStatusQuery {
    #[serde(default)]
    after: u64,
}

async fn get_mining_status(
    State(st): State<RpcState>,
    Query(q): Query<MiningStatusQuery>,
) -> Json<crate::MiningStatus> {
    let status = st
        .mining
        .lock()
        .map(|mut m| m.status_since(q.after))
        .unwrap_or_else(|_| crate::MiningStatus {
            active_miners: Vec::new(),
            events: Vec::new(),
        });
    Json(status)
}

#[derive(Deserialize)]
struct ExamSubmitReq {
    address: String,
    height: u64,
    digest_hex: String,
    #[serde(default)]
    latency_ms: Option<u64>,
}

async fn submit_exam(
    State(st): State<RpcState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ExamSubmitReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let ip = client_ip_for_mine(&headers, Some(&addr));
    if let Err(ms) = st.ai_limit.check_all(
        &[
            (&format!("exam:ip:{ip}"), 60),
            ("exam:global", 2_000),
        ],
        std::time::Duration::from_secs(60),
    ) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("exam rate limit — retry in {ms}ms"),
        ));
    }
    let miner = Address::from_hex(req.address.trim()).ok_or_else(|| bad("bad address"))?;
    let digest = mesh_types::Hash::from_hex(req.digest_hex.trim())
        .map_err(|_| bad("bad digest_hex"))?;
    let (tip, height, parent) = {
        let c = st.chain.lock().await;
        match c.store().tip() {
            Some(b) => (c.tip_hash(), b.header.height, b.header.prev_hash),
            None => (c.tip_hash(), 0, mesh_types::Hash::zero()),
        }
    };
    let next_h = height.saturating_add(1);
    // 5s blocks: exam POST often lands one height late. Rematch against the
    // template height that was live when the miner pulled GBT.
    let (exam_h, exam_prev) = if req.height == next_h {
        (next_h, tip)
    } else if req.height == height && height > 0 {
        (height, parent)
    } else {
        return Err(bad(format!(
            "exam height {} is stale (template height {next_h})",
            req.height
        )));
    };
    let mining = st
        .mining
        .lock()
        .map(|m| m.has_template_for(&miner.to_hex(), exam_h))
        .unwrap_or(false);
    if !mining {
        return Err(bad(
            "exam requires a live getblocktemplate for this address (Fusion share)",
        ));
    }
    let started = std::time::Instant::now();
    let exam = mesh_ai::assign_exam(exam_h, &exam_prev, &miner);
    let expected = mesh_types::Hash::from_bytes(exam.digest());
    if digest != expected {
        return Err(bad("exam digest mismatch — rematch failed"));
    }
    let latency = req
        .latency_ms
        .unwrap_or_else(|| started.elapsed().as_millis() as u64);
    let result = exam
        .scenario
        .simulate(exam.height, exam.pulse_signal);
    let receipt = mesh_types::AiJobReceipt {
        job_id: exam.job_id(&miner),
        worker: miner,
        input_commitment: mesh_types::Hash::digest(&exam.payload),
        output_hash: expected,
        latency_ms: latency,
        weight: mesh_types::EXAM_LANE_UNITS,
        verified_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        job_kind: mesh_types::AiJobKind::ProtocolEval,
        research_scenario: exam.scenario.as_str().to_string(),
        score_primary: result.scores.primary,
        score_orphan_risk: result.scores.orphan_risk,
        score_detect_rate: result.scores.detect_rate,
        score_linkability: result.scores.linkability,
        score_backlog_ratio: result.scores.backlog_ratio,
        score_latency_p95_ms: result.scores.latency_p95_ms,
    };
    let (new, credited) = {
        let mut c = st.chain.lock().await;
        let new = c.record_ai_receipt(receipt.clone()).map_err(internal)?;
        if new {
            let _ = c.credit_local_service(mesh_types::NodeServiceKind::AiRouting, 1);
        }
        let credited = c
            .store()
            .gpu_scores()
            .get(&miner.to_hex())
            .copied()
            .unwrap_or(0);
        (new, credited)
    };
    if new {
        invalidate_templates_pub(&st);
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
    Ok(Json(serde_json::json!({
        "ok": true,
        "accepted": new,
        "scenario": exam.scenario.as_str(),
        "title": exam.title(),
        "digest": expected.to_hex(),
        "rematch_ms": started.elapsed().as_millis() as u64,
        "weight": if mesh_types::fair_lane_split_active(exam_h) {
            mesh_types::EXAM_LANE_UNITS
        } else {
            0
        },
        "pending_gpu_units": credited,
        "fair_split": mesh_types::fair_lane_split_active(exam_h),
    })))
}

async fn exam_status(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    let next_h = c.height().saturating_add(1);
    let recent: Vec<serde_json::Value> = c
        .store()
        .ai_receipts()
        .iter()
        .rev()
        .filter(|r| mesh_types::is_exam_job_id(&r.job_id))
        .take(32)
        .map(|r| {
            serde_json::json!({
                "job_id": r.job_id,
                "worker": r.worker.to_hex(),
                "scenario": r.research_scenario,
                "title": mesh_ai::ResearchScenario::parse(&r.research_scenario)
                    .map(|s| s.title())
                    .unwrap_or(""),
                "primary": r.score_primary,
                "digest": r.output_hash.to_hex(),
                "latency_ms": r.latency_ms,
                "weight": r.weight,
            })
        })
        .collect();
    Json(serde_json::json!({
        "fair_split_height": mesh_types::fair_split_activation_height(),
        "fair_split_active": mesh_types::fair_lane_split_active(next_h),
        "cpu_bps": mesh_types::cpu_market_bps_at(next_h),
        "gpu_bps": mesh_types::gpu_market_bps_at(next_h),
        "node_bps": mesh_types::node_market_bps_at(next_h),
        "exam_units": mesh_types::EXAM_LANE_UNITS,
        "fusion_gpu_units": mesh_types::FUSION_GPU_UNITS,
        "note": "From height 39000: exam MATCH is required to submit a block. Helper floor pays exam/brain units from the GPU 45%. Research still cannot move BPS.",
        "useful_work_height": mesh_types::useful_work_height(),
        "useful_work_active": mesh_types::useful_work_active(next_h),
        "helper_floor": mesh_types::helper_floor_active(next_h),
        "recent": recent,
    }))
}

#[derive(Deserialize)]
struct HeightQuery {
    height: u64,
}

#[derive(Serialize)]
struct BlockResp {
    height: u64,
    id: String,
    prev: String,
    merkle: String,
    timestamp: u64,
    difficulty: u32,
    nonce: u64,
    /// Confirmations of this block (tip − height + 1).
    confirmations: u64,
    /// Coinbase spendable (`confirmations >= coinbase_maturity`).
    mature: bool,
    coinbase_maturity: u64,
    /// `|mat:N` from this block's coinbase, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    memo_maturity: Option<u64>,
    txs: Vec<TxItem>,
}

async fn get_block(
    State(st): State<RpcState>,
    Query(q): Query<HeightQuery>,
) -> Result<Json<BlockResp>, (StatusCode, String)> {
    let c = st.chain.lock().await;
    let b = c
        .get_block(q.height)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("no block at {}", q.height)))?;
    let tip = c.height();
    let confirmations = tip.saturating_sub(b.header.height).saturating_add(1);
    let memo_maturity = b
        .txs
        .first()
        .and_then(|tx| mesh_types::parse_pomc_layout(&tx.memo))
        .and_then(|l| l.maturity);
    Ok(Json(BlockResp {
        height: b.header.height,
        id: b.id().to_string(),
        prev: b.header.prev_hash.to_string(),
        merkle: b.header.merkle_root.to_string(),
        timestamp: b.header.timestamp,
        difficulty: b.header.difficulty,
        nonce: b.header.nonce,
        confirmations,
        mature: tip.saturating_add(1)
            >= b.header.height.saturating_add(mesh_types::COINBASE_MATURITY),
        coinbase_maturity: mesh_types::COINBASE_MATURITY,
        memo_maturity,
        txs: b
            .txs
            .iter()
            .map(|tx| tx_item(tx, Some(b.header.height), Some(b.header.timestamp), false))
            .collect(),
    }))
}

#[derive(Serialize)]
struct MempoolResp {
    txs: Vec<TxItem>,
}

async fn get_mempool(State(st): State<RpcState>) -> Json<MempoolResp> {
    let c = st.chain.lock().await;
    Json(MempoolResp {
        txs: c
            .mempool()
            .iter()
            .map(|tx| tx_item(tx, None, None, true))
            .collect(),
    })
}

async fn explorer_page() -> Html<&'static str> {
    Html(include_str!("../static/explorer.html"))
}

#[derive(Serialize)]
struct VoteResp {
    ok: bool,
    message: String,
}

async fn submit_vote() -> (StatusCode, Json<VoteResp>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(VoteResp {
            ok: false,
            message: "use POST /v1/proposals/vote { id, choice: yes|no } — one vote per node_id"
                .into(),
        }),
    )
}

fn tx_involves(tx: &mesh_types::Transaction, addr: &Address) -> bool {
    tx.outputs.iter().any(|o| o.address == *addr)
}

fn tx_item(
    tx: &mesh_types::Transaction,
    height: Option<u64>,
    timestamp: Option<u64>,
    in_mempool: bool,
) -> TxItem {
    TxItem {
        height,
        timestamp,
        txid: tx.txid().to_string(),
        memo: tx.memo.clone(),
        outputs: tx
            .outputs
            .iter()
            .enumerate()
            .map(|(i, o)| out_item(tx, i, o))
            .collect(),
        in_mempool,
    }
}

async fn resolve_address(
    st: &RpcState,
    address: Option<String>,
) -> Result<Address, (StatusCode, String)> {
    match address {
        Some(s) => Address::from_hex(&s).ok_or_else(|| bad("bad address")),
        None => Ok(st.wallet.read().await.address()),
    }
}

/// Node-market views default to the operator payout address, not the hot key.
async fn resolve_operator_or_wallet(
    st: &RpcState,
    address: Option<String>,
) -> Result<Address, (StatusCode, String)> {
    if let Some(s) = address {
        return Address::from_hex(&s).ok_or_else(|| bad("bad address"));
    }
    if let Some(op) = st.chain.lock().await.node_operator {
        return Ok(op);
    }
    Ok(st.wallet.read().await.address())
}

#[derive(Deserialize)]
struct SetOperatorBody {
    address: String,
}

#[derive(Serialize)]
struct SetOperatorResp {
    address: String,
    pending_weight: u64,
    bonded: bool,
    bond_eligible: bool,
}

async fn set_operator(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(body): Json<SetOperatorBody>,
) -> Result<Json<SetOperatorResp>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let addr = Address::from_hex(body.address.trim()).ok_or_else(|| bad("bad address"))?;
    let mut c = st.chain.lock().await;
    c.set_node_operator(addr);
    let _ = c.register_node_bond(addr, "");
    let pending_weight = c.pending_node_weight(&addr);
    let bonded = c
        .node_bond(&addr)
        .map(|b| !b.slashed && b.unlock_after_height == 0 && b.locked_atomic() > 0)
        .unwrap_or(false);
    let bond_eligible = c.is_node_bond_eligible(&addr);
    Ok(Json(SetOperatorResp {
        address: addr.to_string(),
        pending_weight,
        bonded,
        bond_eligible,
    }))
}

#[derive(Deserialize)]
struct AiReceiptBody {
    job_id: String,
    worker: String,
    input_commitment: String,
    output_hash: String,
    latency_ms: u64,
    weight: u64,
    #[serde(default)]
    verified_at: Option<u64>,
    #[serde(default)]
    job_kind: Option<String>,
    #[serde(default)]
    research_scenario: String,
    #[serde(default)]
    score_primary: f64,
    #[serde(default)]
    score_orphan_risk: f64,
    #[serde(default)]
    score_detect_rate: f64,
    #[serde(default)]
    score_linkability: f64,
    #[serde(default)]
    score_backlog_ratio: f64,
    #[serde(default)]
    score_latency_p95_ms: f64,
}

async fn post_ai_receipt(
    State(st): State<RpcState>,
    Json(body): Json<AiReceiptBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Public mesh surface — rate-limited; wallet token must not block GPU/orch settlement (B5).
    if let Err(ms) = st
        .ai_limit
        .check("aireceipt", 180, std::time::Duration::from_secs(60))
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("aireceipt rate limit — retry in {ms}ms"),
        ));
    }
    let worker = Address::from_hex(&body.worker).ok_or_else(|| bad("bad worker address"))?;
    let input_commitment = mesh_types::Hash::from_hex(&body.input_commitment)
        .map_err(|_| bad("bad input_commitment"))?;
    let output_hash =
        mesh_types::Hash::from_hex(&body.output_hash).map_err(|_| bad("bad output_hash"))?;
    let job_kind = match body.job_kind.as_deref() {
        Some("benchmark") => mesh_types::AiJobKind::Benchmark,
        Some("protocol_eval") => mesh_types::AiJobKind::ProtocolEval,
        Some("agent_assist") => mesh_types::AiJobKind::AgentAssist,
        Some("ml_train") => mesh_types::AiJobKind::MlTrain,
        _ => mesh_types::AiJobKind::Echo,
    };
    if mesh_types::is_exam_job_id(&body.job_id) {
        return Err(bad(
            "exam receipts must use POST /v1/exam/submit (digest rematch required)",
        ));
    }
    let receipt = mesh_types::AiJobReceipt {
        job_id: body.job_id,
        worker,
        input_commitment,
        output_hash,
        latency_ms: body.latency_ms,
        weight: body.weight.max(1),
        verified_at: body.verified_at.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        }),
        job_kind,
        research_scenario: body.research_scenario,
        score_primary: body.score_primary,
        score_orphan_risk: body.score_orphan_risk,
        score_detect_rate: body.score_detect_rate,
        score_linkability: body.score_linkability,
        score_backlog_ratio: body.score_backlog_ratio,
        score_latency_p95_ms: body.score_latency_p95_ms,
    };
    let resp = {
        let mut c = st.chain.lock().await;
        c.record_ai_receipt_imported(receipt.clone())
            .map_err(internal)?;
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
        Json(serde_json::json!({
            "ok": true,
            "last_auto_adapt_proposal_id": c.last_auto_adapt_proposal_id(),
            "last_auto_adapt_at_height": c.last_auto_adapt_at_height(),
            "last_auto_adapt_eval_count": c.last_auto_adapt_eval_count(),
            "param_epoch": c.param_epoch(),
        }))
    };
    invalidate_templates(&st);
    Ok(resp)
}

#[derive(Deserialize)]
struct NodeScoreBody {
    address: String,
    #[serde(default = "default_weight")]
    weight: u64,
}

fn default_weight() -> u64 {
    1
}

async fn post_node_score(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(body): Json<NodeScoreBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    // Operator surface — not a public mint. Relay work is credited locally
    // via credit_local_service; this endpoint cannot invent node-market share.
    if let Err(ms) = st
        .ai_limit
        .check("nodescore", 30, std::time::Duration::from_secs(60))
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("nodescore rate limit — retry in {ms}ms"),
        ));
    }
    let addr = Address::from_hex(&body.address).ok_or_else(|| bad("bad address"))?;
    {
        let mut c = st.chain.lock().await;
        if !c.is_node_bond_eligible(&addr) {
            return Err((
                StatusCode::FORBIDDEN,
                format!(
                    "node bond required: hold ≥ {} atomic MESH and POST /v1/nodebond",
                    mesh_chain::MIN_NODE_BOND_ATOMIC
                ),
            ));
        }
        c.credit_node_score(addr, body.weight.max(1).min(8))
            .map_err(internal)?;
    }
    invalidate_templates(&st);
    Ok(Json(serde_json::json!({ "ok": true, "weight": body.weight.max(1) })))
}

async fn get_finality(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    Json(c.finality_status())
}

async fn post_finality_attest(
    State(st): State<RpcState>,
    Json(att): Json<mesh_chain::FinalityAttestation>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Public, signature-gated (same class as aireceipt). Cookie is not a vote.
    if let Err(ms) = st
        .ai_limit
        .check("finality", 60, std::time::Duration::from_secs(60))
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("finality rate limit — retry in {ms}ms"),
        ));
    }
    let (ingest, height) = {
        let mut c = st.chain.lock().await;
        let height = c.height();
        let ingest = c
            .record_finality_attestation(att.clone())
            .map_err(|e| bad(e.to_string()))?;
        (ingest, height)
    };
    if let Some(net) = &st.network {
        if ingest.new_vote {
            net.announce_finality_attest(att);
        }
        if let Some(addr) = ingest.slashed {
            net.announce_slash_mark(addr.to_string(), String::new(), height, 0, String::new());
        }
    }
    Ok(Json(serde_json::json!({
        "ok": ingest.slashed.is_none(),
        "new_vote": ingest.new_vote,
        "advanced": ingest.advanced,
        "slashed": ingest.slashed.map(|a| a.to_string()),
    })))
}

#[derive(Deserialize)]
struct NodeBondBody {
    #[serde(default)]
    peer_id: String,
}

async fn get_node_bond(
    State(st): State<RpcState>,
    Query(q): Query<AddressQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let addr = resolve_address(&st, q.address).await?;
    let c = st.chain.lock().await;
    let bond = c.node_bond(&addr);
    Ok(Json(serde_json::json!({
        "address": addr.to_string(),
        "balance_atomic": c.balance(&addr).atomic(),
        "spendable_atomic": c.spendable_balance(&addr).atomic(),
        "min_bond_atomic": mesh_chain::MIN_NODE_BOND_ATOMIC,
        "unlock_cooldown_blocks": mesh_chain::BOND_UNLOCK_COOLDOWN_BLOCKS,
        "bonded": bond.as_ref().map(|b| !b.slashed && b.unlock_after_height == 0).unwrap_or(false),
        "eligible": c.is_node_bond_eligible(&addr),
        "bond": bond,
        "note": "POST /v1/nodebond locks >= min UTXOs; /v1/nodeunbond starts cooldown; /v1/nodeslash assigns to slash vault (soft) + freezes",
    })))
}

async fn post_node_bond(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(body): Json<NodeBondBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let addr = {
        let w = st.wallet.read().await;
        w.address()
    };
    let peer_id = if body.peer_id.trim().is_empty() {
        st.network
            .as_ref()
            .map(|n| n.local_peer_id.to_string())
            .unwrap_or_default()
    } else {
        body.peer_id.trim().to_string()
    };
    let rec = {
        let mut c = st.chain.lock().await;
        c.register_node_bond(addr, &peer_id)
            .map_err(|e| bad(e.to_string()))?
    };
    Ok(Json(serde_json::json!({
        "ok": true,
        "address": addr.to_string(),
        "bond": rec,
        "min_bond_atomic": mesh_chain::MIN_NODE_BOND_ATOMIC,
    })))
}

#[derive(Deserialize)]
struct SlashBody {
    address: String,
}

async fn post_node_unbond(
    State(st): State<RpcState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let addr = {
        let w = st.wallet.read().await;
        w.address()
    };
    let rec = {
        let mut c = st.chain.lock().await;
        // Try finalize if cooldown elapsed; else request.
        match c.finalize_node_unbond(addr) {
            Ok(r) => r,
            Err(_) => c.request_node_unbond(addr).map_err(|e| bad(e.to_string()))?,
        }
    };
    Ok(Json(serde_json::json!({ "ok": true, "bond": rec })))
}

async fn post_node_slash(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(body): Json<SlashBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let addr = Address::from_hex(&body.address).ok_or_else(|| bad("bad address"))?;
    let wallet = st.wallet.read().await.clone();
    let peer_id = st
        .network
        .as_ref()
        .map(|n| n.local_peer_id.to_string())
        .unwrap_or_default();
    let (rec, settle_txid, settle_tx, height) = {
        let mut c = st.chain.lock().await;
        let height = c.height();
        let rec = c.slash_node_bond(addr).map_err(|e| bad(e.to_string()))?;
        let (settle_txid, settle_tx) = if addr == wallet.address() && !rec.locked.is_empty() {
            match c.submit_slash_settle(&wallet) {
                Ok(id) => {
                    invalidate_templates(&st);
                    let tx = c.mempool().iter().find(|t| t.txid() == id).cloned();
                    (Some(id.to_string()), tx)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "auto slash settle skipped");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };
        (rec, settle_txid, settle_tx, height)
    };
    if let Some(net) = &st.network {
        net.announce_slash_mark(
            addr.to_string(),
            settle_txid.clone().unwrap_or_default(),
            height,
            rec.slashed_to_vault_atomic,
            peer_id,
        );
        // Gossip settle body with the mark so peers can adopt the preferred txid (N5 race).
        if let Some(tx) = settle_tx {
            net.announce_tx(tx);
        }
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "bond": rec,
        "slash_vault": mesh_chain::deferred_slash_vault().to_string(),
        "slashed_to_vault_atomic": rec.slashed_to_vault_atomic,
        "settle_txid": settle_txid,
        "note": "soft slash + SlashMark gossip; settle tx gossiped when local wallet owns bond",
    })))
}

async fn post_node_slash_settle(
    State(st): State<RpcState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let (txid, tx) = {
        let w = st.wallet.read().await;
        let mut c = st.chain.lock().await;
        let txid = c.submit_slash_settle(&w).map_err(|e| bad(e.to_string()))?;
        let tx = c
            .mempool()
            .iter()
            .find(|t| t.txid() == txid)
            .cloned();
        (txid, tx)
    };
    invalidate_templates(&st);
    if let (Some(net), Some(tx)) = (&st.network, tx) {
        net.announce_tx(tx);
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "txid": txid.to_string(),
        "slash_vault": mesh_chain::deferred_slash_vault().to_string(),
        "note": "settle in mempool + gossiped — confirms when mined; spends locked outs to slash vault",
    })))
}

async fn get_archive_info(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    let blocks = c.store().len();
    let height = c.height();
    let snap = c.store().tip_snapshot();
    let full = blocks > 0 && blocks as u64 == height.saturating_add(1) && !c.store().is_pruned();
    Json(serde_json::json!({
        "service": "archive",
        "tip_height": height,
        "tip": snap.tip,
        "genesis": c.genesis_hash().to_string(),
        "blocks": blocks,
        "utxos": snap.utxos,
        "pruned": c.store().is_pruned(),
        "hot_from_height": c.store().hot_from_height(),
        "has_full_history": full,
        "endpoints": [
            "/v1/archive/headers",
            "/v1/archive/blocks",
            "/v1/snapshot",
            "/v1/snapshot/download",
            "/v1/snapshot/utxos",
            "/v1/snapshot/pruneplan",
            "/v1/snapshot/prune"
        ],
        "note": "Build/06 archive hosting — headers + blocks + UTXO checkpoint",
    }))
}

async fn get_archive_headers(
    State(st): State<RpcState>,
    Query(q): Query<ArchiveQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (from, limit, height, tip, headers) = {
        let mut c = st.chain.lock().await;
        let tip_h = c.height();
        let limit = q.limit.unwrap_or(100).min(500);
        let from = resolve_archive_from(q.from, q.to, q.behind_tip, tip_h, limit);
        let blocks = c.blocks_from(from, limit);
        if !blocks.is_empty() {
            let _ = c.credit_local_service(mesh_types::NodeServiceKind::Archive, 1);
        }
        let headers: Vec<_> = blocks
            .iter()
            .map(|b| {
                serde_json::json!({
                    "height": b.header.height,
                    "id": b.id().to_string(),
                    "prev": b.header.prev_hash.to_string(),
                    "difficulty": b.header.difficulty,
                    "timestamp": b.header.timestamp,
                    "nonce": b.header.nonce,
                    "txs": b.txs.len(),
                })
            })
            .collect();
        (from, limit, tip_h, c.tip_hash().to_string(), headers)
    };
    Ok(Json(serde_json::json!({
        "service": "archive",
        "tip_height": height,
        "tip": tip,
        "from": from,
        "limit": limit,
        "count": headers.len(),
        "headers": headers,
    })))
}

async fn get_archive_blocks(
    State(st): State<RpcState>,
    Query(q): Query<ArchiveQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut c = st.chain.lock().await;
    let tip_h = c.height();
    let limit = q.limit.unwrap_or(50).min(200);
    let from = resolve_archive_from(q.from, q.to, q.behind_tip, tip_h, limit);
    let lim = if let Some(to) = q.to {
        if to < from {
            return Err(bad("to < from"));
        }
        (to.saturating_sub(from).saturating_add(1) as u32).min(limit)
    } else {
        limit
    };
    let blocks = c.blocks_from(from, lim);
    if !blocks.is_empty() {
        let w = (blocks.len() as u64).min(8).max(1);
        let _ = c.credit_local_service(mesh_types::NodeServiceKind::Archive, w);
    }
    Ok(Json(serde_json::json!({
        "service": "archive",
        "tip_height": tip_h,
        "tip": c.tip_hash().to_string(),
        "from": from,
        "to": q.to,
        "count": blocks.len(),
        "blocks": blocks_to_wire(&blocks),
        "note": "Build/06 archive hosting — pulls credit local node market when bonded",
    })))
}

fn resolve_archive_from(
    from: Option<u64>,
    to: Option<u64>,
    behind_tip: Option<u64>,
    tip_h: u64,
    limit: u32,
) -> u64 {
    if let Some(b) = behind_tip {
        let window = b.max(1);
        return tip_h.saturating_sub(window.saturating_sub(1));
    }
    if let (None, Some(to)) = (from, to) {
        return to.saturating_sub(u64::from(limit.saturating_sub(1)));
    }
    from.unwrap_or(0)
}

fn blocks_to_wire(blocks: &[mesh_types::Block]) -> Vec<serde_json::Value> {
    blocks
        .iter()
        .map(|b| {
            serde_json::json!({
                "height": b.header.height,
                "id": b.id().to_string(),
                "txs": b.txs.len(),
                "block_hex": hex::encode(bincode::serialize(b).unwrap_or_default()),
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct ArchiveQuery {
    from: Option<u64>,
    to: Option<u64>,
    /// Serve the last N blocks ending at tip (inclusive window size).
    behind_tip: Option<u64>,
    limit: Option<u32>,
}

async fn get_gpu_scores(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    Json(serde_json::json!({
        "gpu_scores": c.store().gpu_scores(),
        "node_scores": c.store().node_scores(),
    }))
}

async fn get_markets(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    let h = c.height();
    let next = h.saturating_add(1);
    let gpu = mesh_chain::gpu_market_reward(next);
    let helper = mesh_types::helper_floor_active(next);
    let exam = if helper {
        gpu.split_bps(mesh_types::HELPER_EXAM_FLOOR_BPS)
    } else {
        mesh_types::Amount::ZERO
    };
    let fusion = if helper {
        Amount::from_atomic(gpu.atomic().saturating_sub(exam.atomic()))
    } else {
        gpu
    };
    Json(serde_json::json!({
        "height": h,
        "next_height": next,
        "fair_split": mesh_types::fair_lane_split_active(next),
        "fair_split_height": mesh_types::fair_split_activation_height(),
        "cpu_bps": mesh_types::cpu_market_bps_at(next),
        "gpu_bps": mesh_types::gpu_market_bps_at(next),
        "node_bps": mesh_types::node_market_bps_at(next),
        "supply_cap_mesh": mesh_types::SUPPLY_CAP_MESH,
        "emitted_atomic": mesh_chain::emitted_before_atomic(next).to_string(),
        "block_reward": mesh_chain::block_reward(next).to_string(),
        "cpu_market": mesh_chain::cpu_market_reward(next).to_string(),
        "gpu_market": gpu.to_string(),
        "gpu_exam_market": if helper { exam.to_string() } else { String::new() },
        "gpu_fusion_market": if helper { fusion.to_string() } else { String::new() },
        "helper_floor": helper,
        "finder_unify": mesh_types::finder_unify_active(next),
        "finder_unify_height": mesh_types::finder_unify_height(),
        "useful_work_height": mesh_types::useful_work_height(),
        "useful_work_active": mesh_types::useful_work_active(next),
        "exam_required": mesh_types::exam_required_for_block(next),
        "coinbase_maturity": mesh_types::COINBASE_MATURITY,
        "node_market": mesh_chain::node_market_reward(next).to_string(),
        "deferred_gpu_vault": mesh_chain::deferred_gpu_vault().to_string(),
        "deferred_node_vault": mesh_chain::deferred_node_vault().to_string(),
        "deferred_slash_vault": mesh_chain::deferred_slash_vault().to_string(),
        "slashed_vault_atomic": c.store().slashed_vault_atomic(),
        "pending_gpu_scores": c.store().gpu_scores(),
        "pending_node_scores": c.store().node_scores(),
    }))
}

async fn get_mesh_pulse(State(st): State<RpcState>) -> Json<mesh_ai::MeshPulse> {
    let mut pulse = {
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
    if let Some(b) = st.ai.lock().await.brain() {
        let m = b.meta();
        pulse.brain_epoch = m.epoch;
        pulse.brain_digest_hex = m.digest_hex;
        pulse.brain_acc = m.last_acc;
        pulse.brain_advances = m.advances;
        pulse.note = format!(
            "{} | shared brain epoch={} advances={}",
            pulse.note, m.epoch, m.advances
        );
    }
    {
        let q = st.ai.lock().await;
        let scores = &pulse.markets.research_scores;
        let (epochs, smart) = match q.legs() {
            Some(p) => (p.epochs(), p.smart()),
            None => (mesh_ai::LegEpochs::default(), mesh_ai::LegSmart::default()),
        };
        let board = mesh_ai::build_trilemma_board(
            scores.mean_detect_rate,
            scores.mean_orphan_risk,
            scores.mean_backlog_ratio,
            scores.mean_latency_p95_ms,
            scores.mean_linkability,
            scores.mean_primary,
            1,
            q.workers().count() as u32,
            epochs,
            smart,
        );
        pulse.note = format!(
            "{} | trilemma sec={} scale={} decent={} transpar={} balance={} weak={}",
            pulse.note,
            board.sec,
            board.scale,
            board.decent,
            board.transpar,
            board.balance,
            board.weakest
        );
        pulse.trilemma = Some(board);
    }
    let receipts = {
        let c = st.chain.lock().await;
        c.store().ai_receipts().to_vec()
    };
    {
        let q = st.ai.lock().await;
        let inputs =
            mesh_ai::quantum_score_inputs(&receipts, pulse.height, pulse.gpu_vs_height_signal);
        let (epochs, smart) = match q.quantum() {
            Some(p) => (p.epochs(), p.smart()),
            None => (
                mesh_ai::QuantumEpochs::default(),
                mesh_ai::QuantumSmart::default(),
            ),
        };
        let mut qboard = mesh_ai::build_quantum_board(
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
        qboard.note = format!(
            "{} — feed weakest={}; readiness is the min needle",
            inputs.honesty, qboard.weakest
        );
        pulse.note = format!(
            "{} | quantum pqc={} grover={} secrecy={} ready={} weak={}",
            pulse.note,
            qboard.pqc,
            qboard.grover,
            qboard.secrecy,
            qboard.readiness,
            qboard.weakest
        );
        pulse.quantum = Some(qboard);
    }
    Json(pulse)
}

async fn list_proposals(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    let peer = st.network.as_ref().map(|n| n.local_peer_id.to_string());
    let latest = c.epoch_history().last().cloned();
    Json(serde_json::json!({
        "proposals": c.proposals(),
        "active_envelopes": c.active_envelopes(),
        "local_node_id": peer,
        "last_auto_adapt_proposal_id": c.last_auto_adapt_proposal_id(),
        "last_auto_adapt_at_height": c.last_auto_adapt_at_height(),
        "last_auto_adapt_eval_count": c.last_auto_adapt_eval_count(),
        "param_epoch": c.param_epoch(),
        "epoch_history": c.epoch_history(),
        "latest_epoch": latest,
        "consensus_difficulty": c.next_difficulty(),
        "soft_diff_hint": c.soft_mining_diff_hint(),
        "retarget": {
            "interval": c.retarget_params().interval,
            "step": c.retarget_params().step,
            "min_floor": c.retarget_params().min_floor,
        },
        "quantum_gate": {
            "grover_eval_count": c.grover_eval_count(),
            "grover_certs_since_retarget_adapt": c.grover_certs_since_retarget_adapt(),
            "min_grover_certs_for_retarget": mesh_types::MIN_GROVER_CERTS_FOR_RETARGET,
        },
        "note": "Build/30 soft+quantum-gated retarget epochs. BPS unchanged without hard governance.",
    }))
}

async fn generate_proposal(
    State(st): State<RpcState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let mut c = st.chain.lock().await;
    let p = c.generate_adaptive_proposal().map_err(internal)?;
    Ok(Json(serde_json::json!({ "proposal": p })))
}

#[derive(Deserialize)]
struct ProposalIdBody {
    id: String,
}

#[derive(Deserialize)]
struct ProposalVoteBody {
    id: String,
    /// "yes" | "no"
    choice: String,
    /// Defaults to this node's libp2p peer id.
    #[serde(default)]
    node_id: Option<String>,
}

async fn vote_proposal(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(body): Json<ProposalVoteBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let choice = match body.choice.trim().to_ascii_lowercase().as_str() {
        "yes" | "activate" | "y" => mesh_types::VoteChoice::Yes,
        "no" | "reject" | "n" => mesh_types::VoteChoice::No,
        _ => return Err(bad("choice must be yes|no")),
    };
    let node_id = body
        .node_id
        .filter(|s| !s.trim().is_empty())
        .or_else(|| st.network.as_ref().map(|n| n.local_peer_id.to_string()))
        .ok_or_else(|| bad("node_id required (no peer id on this node)"))?;
    let mut c = st.chain.lock().await;
    let p = c
        .cast_proposal_vote(&body.id, &node_id, choice)
        .map_err(|e| bad(e.to_string()))?;
    let (yes, no) = p.vote_counts();
    Ok(Json(serde_json::json!({
        "ok": true,
        "proposal": p,
        "node_id": node_id,
        "yes": yes,
        "no": no,
        "note": "one vote per node_id; majority decides soft envelopes (BPS unchanged)",
    })))
}

async fn activate_proposal(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(body): Json<ProposalIdBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Operator shortcut → counted as a yes vote from this node id.
    require_wallet_token(&st, &headers)?;
    let node_id = st
        .network
        .as_ref()
        .map(|n| n.local_peer_id.to_string())
        .ok_or_else(|| bad("node peer id unavailable; use /v1/proposals/vote"))?;
    let mut c = st.chain.lock().await;
    let p = c
        .cast_proposal_vote(&body.id, &node_id, mesh_types::VoteChoice::Yes)
        .map_err(|e| bad(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "proposal": p,
        "note": "recorded as yes vote from this node_id (not unlimited activate)",
    })))
}

async fn reject_proposal(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(body): Json<ProposalIdBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let node_id = st
        .network
        .as_ref()
        .map(|n| n.local_peer_id.to_string())
        .ok_or_else(|| bad("node peer id unavailable; use /v1/proposals/vote"))?;
    let mut c = st.chain.lock().await;
    let p = c
        .cast_proposal_vote(&body.id, &node_id, mesh_types::VoteChoice::No)
        .map_err(|e| bad(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "proposal": p,
        "note": "recorded as no vote from this node_id",
    })))
}

async fn get_snapshot(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    let snap = c.store().tip_snapshot();
    Json(serde_json::json!({
        "height": snap.height,
        "tip": snap.tip,
        "genesis": c.genesis_hash().to_string(),
        "blocks": snap.blocks,
        "utxos": snap.utxos,
        "mempool": c.mempool().len(),
        "param_epoch": c.param_epoch(),
        "download": "/v1/snapshot/download",
        "note": "Lightweight catch-up meta (Build/10 SNAPSHOT v0) — use /v1/snapshot/download for block batch",
    }))
}

async fn get_snapshot_download(
    State(st): State<RpcState>,
    Query(q): Query<ArchiveQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut c = st.chain.lock().await;
    let tip_h = c.height();
    let limit = q.limit.unwrap_or(64).min(200);
    let from = resolve_archive_from(q.from, q.to, q.behind_tip, tip_h, limit);
    let blocks = c.blocks_from(from, limit);
    if !blocks.is_empty() {
        let w = (blocks.len() as u64).min(8).max(1);
        let _ = c.credit_local_service(mesh_types::NodeServiceKind::Snapshot, w);
    }
    let snap = c.store().tip_snapshot();
    Ok(Json(serde_json::json!({
        "service": "snapshot",
        "height": snap.height,
        "tip": snap.tip,
        "genesis": c.genesis_hash().to_string(),
        "utxos": snap.utxos,
        "from": from,
        "count": blocks.len(),
        "blocks": blocks_to_wire(&blocks),
        "note": "HTTP snapshot batch (same shape as P2P GetSnapshot) — import via submitblock/P2P",
    })))
}

#[derive(Deserialize)]
struct UtxoSnapQuery {
    offset: Option<usize>,
    limit: Option<usize>,
}

async fn get_snapshot_utxos(
    State(st): State<RpcState>,
    Query(q): Query<UtxoSnapQuery>,
) -> Json<serde_json::Value> {
    let mut c = st.chain.lock().await;
    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(500).min(2_000);
    let snap = c.store().tip_snapshot();
    let rows = c.store().utxo_export(offset, limit);
    if !rows.is_empty() {
        let _ = c.credit_local_service(mesh_types::NodeServiceKind::Snapshot, 2);
    }
    let utxos: Vec<_> = rows
        .into_iter()
        .map(|(txid, vout, address, atomic)| {
            serde_json::json!({
                "txid": txid,
                "vout": vout,
                "address": address,
                "atomic": atomic,
            })
        })
        .collect();
    Json(serde_json::json!({
        "service": "snapshot",
        "height": snap.height,
        "tip": snap.tip,
        "utxo_count": snap.utxos,
        "offset": offset,
        "count": utxos.len(),
        "utxos": utxos,
        "note": "UTXO checkpoint export — cold prune plan at /v1/snapshot/pruneplan",
    }))
}

#[derive(Deserialize)]
struct PruneQuery {
    keep_blocks: Option<u64>,
}

async fn get_prune_plan(
    State(st): State<RpcState>,
    Query(q): Query<PruneQuery>,
) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    let keep = q.keep_blocks.unwrap_or(2_048);
    let plan = c.store().cold_prune_plan(keep);
    Json(serde_json::json!({
        "plan": plan,
        "pruned": c.store().is_pruned(),
        "hot_from_height": c.store().hot_from_height(),
        "genesis": c.genesis_hash().to_string(),
        "min_keep": mesh_chain::MIN_COLD_PRUNE_KEEP,
        "env_enabled": std::env::var("MESH_COLD_PRUNE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        "note": "POST /v1/snapshot/prune requires wallet token + MESH_COLD_PRUNE=1 + confirm=1",
    }))
}

#[derive(Deserialize)]
struct PruneBody {
    #[serde(default)]
    keep_blocks: Option<u64>,
    /// Must be 1 / true to apply.
    #[serde(default)]
    confirm: Option<serde_json::Value>,
}

async fn post_snapshot_prune(
    State(st): State<RpcState>,
    headers: HeaderMap,
    Json(body): Json<PruneBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_wallet_token(&st, &headers)?;
    let env_ok = std::env::var("MESH_COLD_PRUNE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !env_ok {
        return Err(bad(
            "cold prune disabled — set MESH_COLD_PRUNE=1 on the node",
        ));
    }
    let confirmed = match &body.confirm {
        Some(serde_json::Value::Bool(true)) => true,
        Some(serde_json::Value::Number(n)) => n.as_u64() == Some(1),
        Some(serde_json::Value::String(s)) => {
            s == "1" || s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes")
        }
        _ => false,
    };
    if !confirmed {
        return Err(bad("confirm=1 required"));
    }
    let keep = body.keep_blocks.unwrap_or(2_048);
    let plan = {
        let mut c = st.chain.lock().await;
        c.apply_cold_prune(keep).map_err(|e| bad(e.to_string()))?
    };
    invalidate_templates(&st);
    Ok(Json(serde_json::json!({
        "ok": true,
        "applied": true,
        "plan": plan,
        "note": "hot WAL truncated; utxo.ckpt written — archive peers with full history still serve old bodies",
    })))
}

async fn get_envelopes(State(st): State<RpcState>) -> Json<serde_json::Value> {
    let c = st.chain.lock().await;
    let env = c.active_envelopes();
    let latest = c.epoch_history().last().cloned();
    let consensus = c.next_difficulty();
    let soft = c.soft_mining_diff_hint();
    let rp = c.retarget_params();
    Json(serde_json::json!({
        "envelopes": env,
        "param_epoch": c.param_epoch(),
        "epoch_history": c.epoch_history(),
        "latest_epoch": latest,
        "last_auto_adapt_proposal_id": c.last_auto_adapt_proposal_id(),
        "last_auto_adapt_at_height": c.last_auto_adapt_at_height(),
        "last_auto_adapt_eval_count": c.last_auto_adapt_eval_count(),
        "consensus_difficulty": consensus,
        "soft_diff_hint": soft,
        "retarget": {
            "interval": rp.interval,
            "step": rp.step,
            "min_floor": rp.min_floor,
        },
        "contributor_bps": if mesh_types::shared_contrib_active(c.height().saturating_add(1)) {
            mesh_types::CONTRIBUTOR_MARKET_BPS
        } else {
            mesh_types::CPU_MARKET_BPS
        },
        "cpu_lane_bps": mesh_types::cpu_market_bps_at(c.height().saturating_add(1)),
        "gpu_lane_bps": mesh_types::gpu_market_bps_at(c.height().saturating_add(1)),
        "node_bps": mesh_types::node_market_bps_at(c.height().saturating_add(1)),
        "fair_split": mesh_types::fair_lane_split_active(c.height().saturating_add(1)),
        "fair_split_height": mesh_types::fair_split_activation_height(),
        "shared_contrib": mesh_types::shared_contrib_active(c.height().saturating_add(1)),
        "mesh_strength": c.mesh_strength(),
        "evo": c.evo_recipe_at(c.height().saturating_add(1)).map(|r| serde_json::json!({
            "period": r.period,
            "pad": r.scratchpad_size,
            "rounds": r.mix_rounds,
            "recipe": r.to_hex(),
            "role_tilt": r.role_tilt,
        })),
        "quantum_gate": {
            "grover_eval_count": c.grover_eval_count(),
            "last_retarget_adapt_grover_count": c.last_retarget_adapt_grover_count(),
            "grover_certs_since_retarget_adapt": c.grover_certs_since_retarget_adapt(),
            "min_grover_certs_for_retarget": mesh_types::MIN_GROVER_CERTS_FOR_RETARGET,
            "recent_certs": c.improvement_certs().iter().rev().take(16).collect::<Vec<_>>(),
        },
        "effects": {
            "soft_adapt_signal_threshold": "orch research-tick: enqueue extra GPU benchmarks when gpu_vs_height_signal is below this",
            "soft_benchmark_rounds": "size of those benchmark jobs (scaled by idle_stipend_bps_cap)",
            "min_verifier_weight": "GPU receipt credit floor on the node; orch soft-routing penalizes slow workers harder",
            "suggested_cpu_diff_bias": "soft_diff_hint = consensus difficulty ± bias (miners see it; validation unchanged)",
            "idle_stipend_bps_cap": "scales node relay credits and gates/scales research benchmark intensity",
            "leg_train_enable": "research tick: enqueue trilemma guardian leg-train jobs when enabled",
            "leg_parallel": "max simultaneous trilemma leg-train jobs per research tick",
            "quantum_train_enable": "research tick: enqueue quantum guardian leg-train jobs when enabled",
            "quantum_parallel": "max simultaneous quantum leg-train jobs per research tick",
            "brain_audit_every": "full re-exec 1-in-K for light protocol/benchmark jobs; shared brain always full-verify",
            "retarget_interval": "consensus: blocks between difficulty retargets (10..=40); quantum-gated",
            "retarget_step": "consensus: max difficulty bits moved per retarget (1..=2); quantum-gated",
            "min_difficulty_floor": "consensus: minimum leading-zero difficulty (1..=16); quantum-gated",
        },
        "note": "Build/30: soft epochs + quantum-gated bounded retarget. Marketplace shelved. BPS/crypto human-only.",
    }))
}

fn bad(msg: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg.into())
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
