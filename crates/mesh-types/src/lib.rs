//! Core types for MonkeyMesh: hashes, amounts, addresses, transactions, blocks.

mod address;
mod amount;
mod block;
mod governance;
mod hash;
mod market;
mod payout;
mod transaction;
mod utxo;

pub use address::Address;
pub use amount::{Amount, AmountError};
pub use block::{Block, BlockHeader, BlockId};
pub use governance::{
    ImprovementCertificate, ParamEpoch, ParamProposal, ProposalStatus, ProposalVote,
    ProtocolEnvelopes, VoteChoice, BPS_CEIL_CPU, BPS_CEIL_GPU, BPS_CEIL_NODE, BPS_FLOOR_CPU,
    BPS_FLOOR_GPU, BPS_FLOOR_NODE, MIN_GROVER_CERTS_FOR_RETARGET,
};
pub use hash::{Hash, HASH_LEN};
pub use market::{
    is_paid_research_kind, AiJobKind, AiJobReceipt, DeviceRole, GpuShare, MarketKind,
    NodeServiceAttestation, NodeServiceKind,
};
pub use payout::{
    coinbase_lane, coinbase_payout_label, gpu_vault_address, is_gpu_vault_address,
    is_node_vault_address, node_vault_address, parse_pomc_layout, CoinbaseLane, CoinbasePayoutLabel,
    PomcLayout, GPU_VAULT_PUBKEY_TAG, NODE_VAULT_PUBKEY_TAG,
};
pub use transaction::{Transaction, TxId, TxInput, TxOutput};
pub use utxo::{OutPoint, Utxo};

/// MeshHash-CPU target block time (seconds) from PoMC spec.
pub const TARGET_BLOCK_TIME_SECS: u64 = 5;

/// Coinbase outputs are immature until this many confirmations.
/// Consensus — stamped on every new coinbase as `|mat:20`. Not env-overridable.
pub const COINBASE_MATURITY: u64 = 20;

/// First live height that stamped `|mat:20`. New blocks at this height and
/// above must carry the tag; older blocks stay valid for IBD.
pub const MATURITY_TAG_REQUIRED_HEIGHT: u64 = 27_673;

/// Hard cap: lifetime of the coded schedule
/// `50 MESH × 25_228_800 blocks × 2` (halving series). Not 21B.
pub const SUPPLY_CAP_MESH: u64 = 2_522_880_000;

/// Decimals for display (1 MESH = 10^DECIMALS atomic units).
pub const DECIMALS: u32 = 8;

/// Reward market splits (basis points / 10_000). Legacy isolated markets.
pub const CPU_MARKET_BPS: u16 = 4_000; // 40%
pub const GPU_MARKET_BPS: u16 = 4_000; // 40%
pub const NODE_MARKET_BPS: u16 = 2_000; // 20%

/// Build/31 shared contributor pot + small node slice (height-gated).
pub const CONTRIBUTOR_MARKET_BPS: u16 = 9_000; // 90%
pub const NODE_MARKET_BPS_V2: u16 = 1_000; // 10%
/// Block-finder units inside the contributor pot (pre–fair-split unit share).
pub const CONTRIB_BLOCK_UNITS: u64 = 1_000;
/// Default height for 90/10 shared pot. Override with `MESH_SHARED_BPS_HEIGHT`.
/// Fresh testnet: 1 (genesis coinbase stays 40/40/20). Legacy public tip used 70000.
pub const DEFAULT_SHARED_BPS_HEIGHT: u64 = 1;

/// Fair lane split: 45% CPU finder / 45% GPU (Fusion lane B + one exam) / 10% nodes.
/// Research units cannot move these BPS. Override with `MESH_FAIR_SPLIT_HEIGHT`.
pub const FAIR_CPU_LANE_BPS: u16 = 4_500;
pub const FAIR_GPU_LANE_BPS: u16 = 4_500;
/// Equal unit weight: one Fusion GPU-lane credit == one verified exam per address/height.
pub const FUSION_GPU_UNITS: u64 = 1_000;
pub const EXAM_LANE_UNITS: u64 = 1_000;
/// Share of the GPU 45% reserved for rematched exams (CPU helpers on the network).
/// Remainder is the finder's Fusion lane-B credit. Override is a height gate, not BPS.
pub const HELPER_EXAM_FLOOR_BPS: u16 = 5_000; // 50% of GPU lane
/// Height where homework starts to count on public testnet (tip was ~37853).
/// Env: `MESH_USEFUL_WORK_HEIGHT`.
pub const DEFAULT_USEFUL_WORK_HEIGHT: u64 = 39_000;
/// Verified brain / protocol-eval job — same unit weight as one exam MATCH.
pub const RESEARCH_LANE_UNITS: u64 = 1_000;
/// When set (`MESH_GPU_EXAM_PAY_HEIGHT`), Fusion finder credit is added only if
/// that address already has a rematched exam score. Default: useful-work height.
pub const DEFAULT_GPU_EXAM_PAY_HEIGHT: u64 = DEFAULT_USEFUL_WORK_HEIGHT;
/// Helper floor (exam half of GPU 45%). Default: useful-work height.
pub const DEFAULT_HELPER_FLOOR_HEIGHT: u64 = DEFAULT_USEFUL_WORK_HEIGHT;

fn parse_height_env(name: &str, default: u64) -> u64 {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return default;
            }
            t.parse::<u64>().unwrap_or(default)
        }
        Err(_) => default,
    }
}

pub fn useful_work_height() -> u64 {
    parse_height_env("MESH_USEFUL_WORK_HEIGHT", DEFAULT_USEFUL_WORK_HEIGHT)
}

pub fn useful_work_active(height: u64) -> bool {
    height >= useful_work_height()
}

pub fn gpu_exam_pay_height() -> u64 {
    parse_height_env("MESH_GPU_EXAM_PAY_HEIGHT", useful_work_height())
}

/// GPU 45% Fusion finder credit requires a rematched exam (anti CPU-only vacuum).
pub fn gpu_pay_requires_exam(height: u64) -> bool {
    height >= gpu_exam_pay_height()
}

/// Finder must MATCH the immune exam before `submitblock` (useful-work height).
pub fn exam_required_for_block(height: u64) -> bool {
    useful_work_active(height)
}

pub fn helper_floor_height() -> u64 {
    parse_height_env("MESH_HELPER_FLOOR_HEIGHT", useful_work_height())
}
/// Wiped testnet: height 0 stays 40/40/20; height ≥ 1 is isolated 45/45/10.
pub const DEFAULT_FAIR_SPLIT_HEIGHT: u64 = 1;

/// Height at which coinbase switches to contributor 90% / node 10%.
pub fn shared_bps_activation_height() -> u64 {
    match std::env::var("MESH_SHARED_BPS_HEIGHT") {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return DEFAULT_SHARED_BPS_HEIGHT;
            }
            t.parse::<u64>().unwrap_or(DEFAULT_SHARED_BPS_HEIGHT)
        }
        Err(_) => DEFAULT_SHARED_BPS_HEIGHT,
    }
}

pub fn shared_contrib_active(height: u64) -> bool {
    height >= shared_bps_activation_height()
}

pub fn node_market_bps_at(height: u64) -> u16 {
    if shared_contrib_active(height) {
        NODE_MARKET_BPS_V2
    } else {
        NODE_MARKET_BPS
    }
}

pub fn fair_split_activation_height() -> u64 {
    match std::env::var("MESH_FAIR_SPLIT_HEIGHT") {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return DEFAULT_FAIR_SPLIT_HEIGHT;
            }
            t.parse::<u64>().unwrap_or(DEFAULT_FAIR_SPLIT_HEIGHT)
        }
        Err(_) => DEFAULT_FAIR_SPLIT_HEIGHT,
    }
}

/// Isolated 45/45/10 lanes — GPU receipts cannot dilute the CPU finder pot.
pub fn fair_lane_split_active(height: u64) -> bool {
    shared_contrib_active(height) && height >= fair_split_activation_height()
}

/// GPU 45% splits: exam helpers (network CPUs) vs Fusion finder credit.
pub fn helper_floor_active(height: u64) -> bool {
    fair_lane_split_active(height) && height >= helper_floor_height()
}

pub fn cpu_market_bps_at(height: u64) -> u16 {
    if fair_lane_split_active(height) {
        FAIR_CPU_LANE_BPS
    } else if shared_contrib_active(height) {
        0
    } else {
        CPU_MARKET_BPS
    }
}

pub fn gpu_market_bps_at(height: u64) -> u16 {
    if fair_lane_split_active(height) {
        FAIR_GPU_LANE_BPS
    } else if shared_contrib_active(height) {
        0
    } else {
        GPU_MARKET_BPS
    }
}

/// Immune-exam receipt id: `exam:v1:{height}:{address}`.
pub fn exam_job_id(height: u64, worker: &Address) -> String {
    format!("exam:v1:{height}:{}", worker.to_hex())
}

pub fn is_exam_job_id(job_id: &str) -> bool {
    job_id.starts_with("exam:v1:")
}

/// Official public seed hostname.
pub const SEED_DNS: &str = "seednode.hashmonkeys.cloud";
/// P2P QUIC port on the seed.
pub const SEED_P2P_PORT: u16 = 39_001;
/// Wallet / explorer REST port on the seed.
pub const SEED_RPC_PORT: u16 = 18_080;
/// Edge RPC port (templates / submit failover — Build/27 B9).
pub const SEED_EDGE_RPC_PORT: u16 = 18_081;
/// Orchestrator / marketplace port on the seed host.
pub const SEED_ORCH_PORT: u16 = 18_100;

/// Default P2P dial string for standalone nodes (`host:port`).
pub fn default_seed_p2p() -> String {
    format!("{SEED_DNS}:{SEED_P2P_PORT}")
}

/// Public HTTPS GBT pool (WAN :18081 / :18083 often time out).
pub const PUBLIC_POOL_URL: &str = "https://eu.hashmonkeys.cloud";

/// Default wallet/miner RPC base URL.
pub fn default_seed_rpc_url() -> String {
    format!("http://{SEED_DNS}:{SEED_RPC_PORT}")
}

pub fn looks_like_public_pool(url: &str) -> bool {
    let n = url.to_ascii_lowercase();
    n.contains("eu.hashmonkeys.cloud")
        && !n.contains(":18080")
        && !n.contains(":18081")
        && !n.contains(":18083")
}

/// Default edge RPC base (templates/submit load-split).
pub fn default_edge_rpc_url() -> String {
    format!("http://{SEED_DNS}:{SEED_EDGE_RPC_PORT}")
}

/// RPC bases to try (seed first, then edge, then `MESH_RPC_EDGES` / `MESH_RPC_URLS`).
/// Wallet / general clients: seed-canonical. Miners should use [`prefer_mine_rpc_urls`].
pub fn default_rpc_urls() -> Vec<String> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<String>, raw: &str| {
        let u = raw.trim().trim_end_matches('/').to_string();
        if !u.is_empty() && !out.iter().any(|x| x == &u) {
            out.push(u);
        }
    };
    if let Ok(primary) = std::env::var("MESH_RPC") {
        push(&mut out, &primary);
    }
    push(&mut out, &default_seed_rpc_url());
    push(&mut out, &default_edge_rpc_url());
    push(&mut out, &format!("http://{SEED_DNS}:18083"));
    for key in ["MESH_RPC_EDGES", "MESH_RPC_URLS"] {
        if let Ok(list) = std::env::var(key) {
            for part in list.split(',') {
                push(&mut out, part);
            }
        }
    }
    out
}

/// Parse a comma-separated RPC/orch list (trims, dedupes, strips trailing `/`).
pub fn parse_rpc_list(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let u = part.trim().trim_end_matches('/').to_string();
        if !u.is_empty() && !out.iter().any(|x| x == &u) {
            out.push(u);
        }
    }
    out
}

fn looks_like_edge_rpc(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains(&format!(":{SEED_EDGE_RPC_PORT}"))
        || u.contains(":18081")
        || u.contains(":18083")
        || u.contains("edge")
}

/// Reorder so edge RPC bases come first (Build/27 B1/B9 mine load-split).
pub fn edge_first_rpc_urls(urls: &[String]) -> Vec<String> {
    let mut edges = Vec::new();
    let mut rest = Vec::new();
    for u in urls {
        let t = u.trim().trim_end_matches('/').to_string();
        if t.is_empty() {
            continue;
        }
        if looks_like_edge_rpc(&t) {
            if !edges.iter().any(|x| x == &t) {
                edges.push(t);
            }
        } else if !rest.iter().any(|x| x == &t) && !edges.iter().any(|x| x == &t) {
            rest.push(t);
        }
    }
    edges.extend(rest);
    edges
}

/// Public pool first, then seed, then LAN edges. WAN `:18081`/`:18083` time out (~20s).
pub fn public_pool_first(urls: &[String]) -> Vec<String> {
    let mut pools = Vec::new();
    let mut rest = Vec::new();
    for u in urls {
        let t = u.trim().trim_end_matches('/').to_string();
        if t.is_empty() {
            continue;
        }
        if looks_like_public_pool(&t) {
            if !pools.iter().any(|x| x == &t) {
                pools.push(t);
            }
        } else if !rest.iter().any(|x| x == &t) && !pools.iter().any(|x| x == &t) {
            rest.push(t);
        }
    }
    pools.extend(rest);
    pools
}

/// Mine-preferred defaults: HTTPS pool first (reachable from the internet).
pub fn prefer_mine_rpc_urls() -> Vec<String> {
    let mut urls = vec![PUBLIC_POOL_URL.to_string()];
    urls = merge_rpc_urls(&urls, &default_rpc_urls());
    public_pool_first(&urls)
}

/// Merge discovered edge URLs (e.g. from `/v1/getnodeinfo.edges`) into a list.
pub fn merge_rpc_urls(base: &[String], extra: &[String]) -> Vec<String> {
    let mut out = base.to_vec();
    for u in extra {
        let t = u.trim().trim_end_matches('/').to_string();
        if !t.is_empty() && !out.iter().any(|x| x == &t) {
            out.push(t);
        }
    }
    out
}

/// Default marketplace base URL.
pub fn default_seed_orch_url() -> String {
    format!("http://{SEED_DNS}:{SEED_ORCH_PORT}")
}

/// Seed dial list: official DNS only. LAN IPs can be added in Settings.
pub fn default_seed_connects() -> Vec<String> {
    vec![default_seed_p2p()]
}

/// AI board shard id for a worker address (`hash % shard_count`).
pub fn ai_shard_for_worker(worker_address: &str, shard_count: u32) -> u32 {
    let n = shard_count.max(1);
    let h = blake3::hash(worker_address.trim().as_bytes());
    let v = u32::from_le_bytes([h.as_bytes()[0], h.as_bytes()[1], h.as_bytes()[2], h.as_bytes()[3]]);
    v % n
}

/// Local shard config from env (`MESH_AI_SHARD_ID`, `MESH_AI_SHARD_COUNT`).
pub fn local_ai_shard_config() -> (u32, u32) {
    let count = std::env::var("MESH_AI_SHARD_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1u32)
        .max(1);
    let id = std::env::var("MESH_AI_SHARD_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u32)
        % count;
    (id, count)
}

/// Shard RPC map: `MESH_AI_SHARDS=0=http://seed:18080,1=http://edge:18081` or edges+seed fallback.
pub fn ai_shard_urls(shard_count: u32) -> Vec<String> {
    let mut urls = vec![String::new(); shard_count.max(1) as usize];
    if let Ok(raw) = std::env::var("MESH_AI_SHARDS") {
        for part in raw.split(',') {
            let part = part.trim();
            if let Some((id_s, url)) = part.split_once('=') {
                if let Ok(id) = id_s.trim().parse::<u32>() {
                    if (id as usize) < urls.len() {
                        urls[id as usize] = url.trim().trim_end_matches('/').to_string();
                    }
                }
            }
        }
    }
    let fallbacks = default_rpc_urls();
    for (i, slot) in urls.iter_mut().enumerate() {
        if slot.is_empty() {
            *slot = fallbacks
                .get(i)
                .cloned()
                .unwrap_or_else(|| fallbacks.first().cloned().unwrap_or_else(default_seed_rpc_url));
        }
    }
    urls
}

/// Pin the worker's preferred AI shard URL first (Build/27 N3 sticky).
/// Does **not** re-sort by edge — shard affinity beats mine edge-first.
pub fn prefer_worker_ai_shard(
    urls: &[String],
    worker_address: &str,
    shard_count: u32,
    shard_urls: &[String],
) -> Vec<String> {
    let count = shard_count.max(1);
    let mut out = Vec::new();
    if count > 1 {
        let sid = ai_shard_for_worker(worker_address, count) as usize;
        if let Some(u) = shard_urls.get(sid).cloned().filter(|s| !s.is_empty()) {
            out.push(u.trim().trim_end_matches('/').to_string());
        }
    }
    for u in urls.iter().chain(shard_urls.iter()) {
        let t = u.trim().trim_end_matches('/').to_string();
        if !t.is_empty() && !out.iter().any(|x| x == &t) {
            out.push(t);
        }
    }
    out
}

/// Extract redirect target from shard mismatch errors (`… try http://…`).
pub fn parse_wrong_shard_try_url(err: &str) -> Option<String> {
    let lower = err.to_ascii_lowercase();
    let looks = lower.contains("wrong ai shard")
        || lower.contains("misdirected")
        || err.contains("421")
        || lower.contains("try http");
    if !looks {
        return None;
    }
    let idx = lower.find("try http")?;
    let rest = &err[idx + 4..];
    let url = rest
        .split(|c: char| c.is_whitespace() || c == ';' || c == '"' || c == '\'')
        .next()?
        .trim()
        .trim_end_matches('/')
        .to_string();
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url)
    } else {
        None
    }
}

#[cfg(test)]
mod shard_tests {
    use super::*;

    #[test]
    fn prefer_mine_puts_https_pool_first() {
        let urls = prefer_mine_rpc_urls();
        assert_eq!(urls[0], PUBLIC_POOL_URL);
        let mixed = public_pool_first(&[
            "http://seednode.hashmonkeys.cloud:18081".into(),
            PUBLIC_POOL_URL.into(),
        ]);
        assert_eq!(mixed[0], PUBLIC_POOL_URL);
    }

    #[test]
    fn prefer_pins_worker_shard_first() {
        let shards = vec![
            "http://seed:18080".into(),
            "http://edge:18081".into(),
        ];
        let base = vec!["http://seed:18080".into(), "http://edge:18081".into()];
        let sid = ai_shard_for_worker("mesh01abc", 2);
        let out = prefer_worker_ai_shard(&base, "mesh01abc", 2, &shards);
        assert_eq!(out[0], shards[sid as usize]);
    }

    #[test]
    fn parse_try_url_from_mismatch() {
        let msg = "wrong AI shard: worker→1 this_node=0; try http://edge:18081";
        assert_eq!(
            parse_wrong_shard_try_url(msg).as_deref(),
            Some("http://edge:18081")
        );
        assert!(parse_wrong_shard_try_url("job HTTP 500").is_none());
    }
}
