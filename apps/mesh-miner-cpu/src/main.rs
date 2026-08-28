use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use mesh_chain::Chain;
use mesh_crypto::Keypair;
use mesh_types::{Address, Block};
use meshhash_cpu::{
    benchmark_light, format_hashrate, meshhash_cpu_with_params, pow_search_inputs, RateWindow,
};
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "mesh-miner-cpu", about = "MonkeyMesh CPU miner")]
struct Args {
    /// Node RPC URL (recommended). When set, mines via getblocktemplate/submitblock.
    #[arg(long)]
    rpc: Option<String>,

    /// Coinbase / payout address (custom mining address)
    #[arg(long)]
    address: Option<String>,

    /// Solo chain file (only when --rpc is omitted)
    #[arg(long, default_value = "data/chain.bin")]
    chain: PathBuf,

    /// Miner secret key hex (generates ephemeral if omitted)
    #[arg(long)]
    key: Option<String>,

    /// Wallet key file (hex secret) — used when --address is omitted
    #[arg(long, default_value = "data/miner.key")]
    keyfile: PathBuf,

    /// Number of blocks to mine (0 = forever / until Ctrl+C)
    #[arg(long, default_value_t = 0)]
    blocks: u64,

    /// Max nonces to try per block before refetching template
    #[arg(long, default_value_t = 5_000_000)]
    max_nonces: u64,

    /// Run a short MeshHash-CPU light benchmark and exit
    #[arg(long)]
    benchmark: bool,

    /// Worker / rig name — pool identity is `address.worker`
    #[arg(long, default_value = "default")]
    worker_name: String,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let args = Args::parse();

    if args.benchmark {
        let hs = benchmark_light(64);
        println!("MeshHash-CPU (light) ≈ {hs:.2} H/s");
        return Ok(());
    }

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        let _ = ctrlc::set_handler(move || {
            stop.store(true, Ordering::SeqCst);
            eprintln!("\nStopping miner…");
        });
    }

    let payout = resolve_payout(&args)?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 64);
    let worker = args
        .worker_name
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect::<String>();
    let worker = if worker.is_empty() {
        "default".to_string()
    } else {
        worker
    };
    let miner_id = format!("{payout}.{worker}");
    info!(%payout, %miner_id, threads, "mining payout address (multi-thread MeshHash)");

    let target = if args.blocks == 0 {
        u64::MAX
    } else {
        args.blocks
    };
    let mut mined = 0u64;

    if let Some(rpc) = &args.rpc {
        let mut urls = mesh_types::parse_rpc_list(rpc);
        for u in mesh_types::prefer_mine_rpc_urls() {
            if !urls.iter().any(|x| x == &u) {
                urls.push(u);
            }
        }
        urls = mesh_types::edge_first_rpc_urls(&urls);
        // Discover advertised edges from the first reachable tip (Build/27 B1).
        if let Some(extra) = discover_rpc_edges(urls.first().map(|s| s.as_str()).unwrap_or("")) {
            urls = mesh_types::edge_first_rpc_urls(&mesh_types::merge_rpc_urls(&urls, &extra));
        }
        info!(?urls, "RPC mining mode (edge-first)");
        mined = mine_rpc_loop(&urls, &payout, &miner_id, args.max_nonces, target, &stop)?;
    } else {
        let mut chain = Chain::open_or_genesis(&args.chain)?;
        info!(
            height = chain.height(),
            tip = %chain.tip_hash(),
            next_diff = chain.next_difficulty(),
            "solo chain mode"
        );
        while mined < target && !stop.load(Ordering::SeqCst) {
            let diff = chain.next_difficulty();
            match chain.mine_next(payout, args.max_nonces)? {
                Some(block) => {
                    mined += 1;
                    info!(
                        height = block.header.height,
                        id = %block.id(),
                        nonce = block.header.nonce,
                        difficulty = block.header.difficulty,
                        balance = %chain.balance(&payout),
                        "solo mined"
                    );
                }
                None => {
                    warn!(
                        max_nonces = args.max_nonces,
                        difficulty = diff,
                        "no solution in nonce window"
                    );
                    break;
                }
            }
        }
    }

    info!(mined, "miner exit");
    Ok(())
}

fn mine_rpc_loop(
    urls: &[String],
    payout: &Address,
    miner_id: &str,
    max_nonces: u64,
    target: u64,
    stop: &Arc<AtomicBool>,
) -> Result<u64> {
    let mut mined = 0u64;
    let mut url_i = 0usize;
    while mined < target && !stop.load(Ordering::SeqCst) {
        let rpc = &urls[url_i % urls.len()];
        match mine_rpc_one(rpc, payout, miner_id, max_nonces, stop) {
            Ok(true) => {
                mined += 1;
                info!(mined, %rpc, "block accepted");
            }
            Ok(false) => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                warn!(%rpc, "mine error: {e:#}");
                if urls.len() > 1 {
                    url_i = url_i.wrapping_add(1);
                    info!(next = %urls[url_i % urls.len()], "failover RPC URL");
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
    Ok(mined)
}

fn resolve_payout(args: &Args) -> Result<Address> {
    if let Some(a) = &args.address {
        return Address::from_hex(a.trim()).context("bad --address");
    }
    warn!("--address not set; deriving from key/keyfile");
    Ok(load_or_create_key(args)?.address())
}

fn load_or_create_key(args: &Args) -> Result<Keypair> {
    if let Some(hex) = &args.key {
        return Ok(Keypair::from_hex(hex)?);
    }
    if args.keyfile.exists() {
        let hex = std::fs::read_to_string(&args.keyfile)?;
        return Ok(Keypair::from_hex(hex.trim())?);
    }
    let kp = Keypair::generate();
    if let Some(parent) = args.keyfile.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&args.keyfile, kp.to_hex())?;
    info!(path = %args.keyfile.display(), "wrote new miner key");
    Ok(kp)
}

#[derive(Deserialize)]
struct TemplateResp {
    difficulty: u32,
    #[serde(default)]
    soft_diff_hint: Option<u32>,
    light_pow: bool,
    #[serde(default)]
    pow_version: u8,
    block_hex: String,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    exam_payload_hex: String,
    #[serde(default)]
    exam_title: String,
    #[serde(default)]
    exam_scenario: String,
}

#[derive(Deserialize)]
struct SubmitResp {
    accepted: bool,
    height: u64,
    id: String,
}

fn mine_rpc_one(
    rpc: &str,
    payout: &Address,
    miner_id: &str,
    max_nonces: u64,
    stop: &Arc<AtomicBool>,
) -> Result<bool> {
    let pool = rpc.contains("eu.hashmonkeys.cloud") || rpc.contains(":12500") || rpc.contains(":13500");
    let payout_s = payout.to_string();
    let addr_q = if pool { miner_id } else { payout_s.as_str() };
    let url = format!("{rpc}/v1/getblocktemplate?address={addr_q}");
    let mut get_req = ureq::get(&url).timeout(Duration::from_secs(30));
    get_req = get_req.set("X-Mesh-Miner", miner_id);
    let tmpl: TemplateResp = get_req
        .call()
        .with_context(|| format!("GET {url}"))?
        .into_json()?;
    if !tmpl.exam_payload_hex.is_empty() {
        submit_cpu_exam(rpc, payout, &tmpl);
    }
    let soft = tmpl.soft_diff_hint.unwrap_or(tmpl.difficulty);
    if soft != tmpl.difficulty {
        info!(
            height = tmpl.height,
            consensus_diff = tmpl.difficulty,
            soft_diff_hint = soft,
            "research soft mining hint (validation still uses consensus_diff)"
        );
    } else {
        info!(
            height = tmpl.height,
            difficulty = tmpl.difficulty,
            pow_version = if tmpl.pow_version == 0 { 1 } else { tmpl.pow_version },
            "template"
        );
    }
    let bytes = hex::decode(&tmpl.block_hex)?;
    let mut block: Block = bincode::deserialize(&bytes)?;
    let pow_version = if tmpl.pow_version == 0 { 1 } else { tmpl.pow_version };
    let commitment = block.header.pre_pow_commitment();
    let (work_seed, params) = pow_search_inputs(
        &commitment,
        tmpl.light_pow,
        block.header.height,
        &block.header.prev_hash,
    );
    if params.version >= 5 {
        anyhow::bail!(
            "Fusion sequential (pow v5): the GPU runs the wave, then a CPU seals that ticket. \
             Use MonkeyMesh Miner with a GPU — CPU-only mining is not the fair path."
        );
    }
    if pow_version >= 4 {
        info!(
            height = tmpl.height,
            pow_version,
            pad = params.scratchpad_size,
            rounds = params.mix_rounds,
            "fusion dual-lane"
        );
    } else if pow_version >= 3 {
        info!(
            height = tmpl.height,
            pow_version,
            pad = params.scratchpad_size,
            rounds = params.mix_rounds,
            "evo work seed"
        );
    }
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 64);
    let job_height = block.header.height;
    let stale = Arc::new(AtomicBool::new(false));
    let watch_done = spawn_tip_watch(rpc.to_string(), job_height, stale.clone());
    let found = AtomicU64::new(u64::MAX);
    let hashes = Arc::new(AtomicU64::new(0));
    let report_done = Arc::new(AtomicBool::new(false));
    let reporter = {
        let hashes = hashes.clone();
        let stale = stale.clone();
        let stop = stop.clone();
        let report_done = report_done.clone();
        thread::spawn(move || {
            let mut window = RateWindow::new();
            let mut last = 0u64;
            while !report_done.load(Ordering::Relaxed)
                && !stop.load(Ordering::Relaxed)
                && !stale.load(Ordering::Relaxed)
            {
                thread::sleep(Duration::from_millis(500));
                let n = hashes.load(Ordering::Relaxed);
                let delta = n.saturating_sub(last);
                last = n;
                let hs = window.push(delta);
                if window.should_send() && hs > 0.05 {
                    info!(fusion = %format_hashrate(hs), "hashrate");
                }
            }
        })
    };
    let chunk = (max_nonces as usize).div_ceil(threads).max(1);
    thread::scope(|scope| {
        for t in 0..threads {
            let start = (t * chunk) as u64;
            if start >= max_nonces {
                continue;
            }
            let end = max_nonces.min(start + chunk as u64);
            let found = &found;
            let stop = stop.as_ref();
            let stale = stale.as_ref();
            let hashes = hashes.as_ref();
            let params = params.clone();
            let work_seed = work_seed;
            let difficulty = tmpl.difficulty;
            scope.spawn(move || {
                for nonce in start..end {
                    if stop.load(Ordering::Relaxed)
                        || stale.load(Ordering::Relaxed)
                        || found.load(Ordering::Relaxed) != u64::MAX
                    {
                        return;
                    }
                    let pow = meshhash_cpu_with_params(&work_seed, nonce, &params);
                    hashes.fetch_add(1, Ordering::Relaxed);
                    if pow.meets_difficulty(difficulty) {
                        let _ = found.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
                            if cur == u64::MAX || nonce < cur {
                                Some(nonce)
                            } else {
                                None
                            }
                        });
                        return;
                    }
                }
            });
        }
    });
    report_done.store(true, Ordering::Relaxed);
    let _ = reporter.join();
    watch_done.store(true, Ordering::Relaxed);
    let nonce = found.load(Ordering::Relaxed);
    if nonce == u64::MAX || stale.load(Ordering::Relaxed) || tip_moved(rpc, job_height) {
        return Ok(false);
    }
    {
            block.header.nonce = nonce;
            let pow = meshhash_cpu_with_params(&work_seed, nonce, &params);
            if !pow.meets_difficulty(tmpl.difficulty) {
                warn!(nonce, height = job_height, "nonce failed Fusion seal rematch");
                return Ok(false);
            }
            if tip_moved(rpc, job_height) {
                return Ok(false);
            }
            info!(height = job_height, nonce, "Fusion seal");
            let body = json!({ "block_hex": hex::encode(bincode::serialize(&block)?) });
            let mut req = ureq::post(&format!("{rpc}/v1/submitblock?address={addr_q}"))
                .timeout(Duration::from_secs(30));
            req = req.set("X-Mesh-Miner", miner_id);
            if let Ok(token) = std::env::var("MESH_RPC_TOKEN") {
                let t = token.trim();
                if !t.is_empty() {
                    req = req.set("X-Mesh-Token", t);
                }
            }
            let resp: SubmitResp = req.send_json(body)?.into_json()?;
            if resp.accepted {
                info!(height = resp.height, id = %resp.id, nonce, "submitted");
                return Ok(true);
            }
            return Ok(false);
    }
}

fn submit_cpu_exam(rpc: &str, payout: &Address, tmpl: &TemplateResp) {
    static LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    {
        let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if *g == tmpl.height {
            return;
        }
        *g = tmpl.height;
    }
    let Ok(payload) = hex::decode(&tmpl.exam_payload_hex) else {
        return;
    };
    let digest = mesh_ai::run_protocol_eval(&payload);
    let body = json!({
        "address": payout.to_hex(),
        "height": tmpl.height,
        "digest_hex": hex::encode(digest),
    });
    let mut urls = vec![rpc.trim_end_matches('/').to_string()];
    for u in mesh_types::default_rpc_urls() {
        if !urls.iter().any(|x| x == &u) {
            urls.push(u);
        }
    }
    for base in urls {
        let url = format!("{base}/v1/exam/submit");
        match ureq::post(&url)
            .timeout(Duration::from_secs(15))
            .send_json(&body)
        {
            Ok(resp) => {
                if let Ok(v) = resp.into_json::<serde_json::Value>() {
                    if v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
                        info!(
                            height = tmpl.height,
                            scenario = %tmpl.exam_scenario,
                            title = %tmpl.exam_title,
                            rematch_ms = v.get("rematch_ms").and_then(|x| x.as_u64()).unwrap_or(0),
                            "exam rematch MATCH"
                        );
                        return;
                    }
                }
            }
            Err(e) => warn!(error = %e, "exam submit"),
        }
    }
}

fn peek_tip_height(rpc: &str) -> Option<u64> {
    #[derive(Deserialize)]
    struct Tip {
        #[serde(default)]
        height: u64,
    }
    let mut urls = vec![format!("{}/v1/getnodeinfo", rpc.trim_end_matches('/'))];
    let seed = format!(
        "{}/v1/getnodeinfo",
        mesh_types::default_seed_rpc_url().trim_end_matches('/')
    );
    if !urls.iter().any(|u| u.eq_ignore_ascii_case(&seed)) {
        urls.push(seed);
    }
    for url in urls {
        let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(3)).call() else {
            continue;
        };
        if let Ok(tip) = resp.into_json::<Tip>() {
            return Some(tip.height);
        }
    }
    None
}

fn tip_moved(rpc: &str, job_height: u64) -> bool {
    peek_tip_height(rpc).is_some_and(|h| h >= job_height)
}

fn spawn_tip_watch(rpc: String, job_height: u64, stale: Arc<AtomicBool>) -> Arc<AtomicBool> {
    let done = Arc::new(AtomicBool::new(false));
    let watch_done = done.clone();
    thread::spawn(move || {
        while !watch_done.load(Ordering::Relaxed) {
            if tip_moved(&rpc, job_height) {
                stale.store(true, Ordering::Relaxed);
                break;
            }
            thread::sleep(Duration::from_millis(750));
        }
    });
    done
}

/// Pull `edges` from `/v1/getnodeinfo` when present (seed advertises MESH_RPC_EDGES).
fn discover_rpc_edges(rpc: &str) -> Option<Vec<String>> {
    if rpc.is_empty() {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct Info {
        #[serde(default)]
        edges: Vec<String>,
    }
    let url = format!("{}/v1/getnodeinfo", rpc.trim_end_matches('/'));
    let info: Info = ureq::get(&url)
        .timeout(Duration::from_secs(3))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    if info.edges.is_empty() {
        None
    } else {
        Some(info.edges)
    }
}
