mod finality_loop;
mod http_sync;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use mesh_chain::Chain;
use mesh_crypto::{Keypair, VaultFile};
use mesh_p2p::{parse_listen_addr, parse_seed_addr, run_node, NodeConfig, SharedChain};
use mesh_rpc::{serve_rpc, RpcState};
use mesh_types::Address;
use tokio::sync::{Mutex, RwLock};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "mesh-node", about = "MonkeyMesh local node")]
struct Args {
    #[arg(long, default_value = "data/chain.bin")]
    chain: PathBuf,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Print tip height, hash, and store path
    Info,
    /// Show a block by height
    GetBlock { height: u64 },
    /// Show balance for an address
    GetBalance { address: String },
    /// List mempool transactions
    Mempool,
    /// Run P2P node over libp2p QUIC (listen / sync / optional mine)
    Serve {
        /// Listen multiaddr or host:port (UDP/QUIC). Example: `127.0.0.1:39001`
        #[arg(long, default_value = "127.0.0.1:39001")]
        listen: String,
        /// Seed peers to dial (multiaddr or host:port). Repeatable.
        #[arg(long = "connect")]
        connect: Vec<String>,
        /// libp2p identity file (ed25519 protobuf)
        #[arg(long, default_value = "data/p2p.key")]
        p2p_key: PathBuf,
        /// REST wallet RPC bind address (empty disables)
        #[arg(long, default_value = "127.0.0.1:18080")]
        rpc: String,
        /// Hot wallet key for RPC send/getnewaddress
        #[arg(long, default_value = "data/wallet.key")]
        wallet: PathBuf,
        /// Cold node-market payout address (overrides vault / hot wallet)
        #[arg(long)]
        operator_address: Option<String>,
        /// Vault JSON path — reads plaintext `address` only (no unlock)
        #[arg(long)]
        operator_vault: Option<PathBuf>,
        /// Mine blocks while serving
        #[arg(long)]
        mine: bool,
        #[arg(long, default_value = "data/miner.key")]
        miner_key: PathBuf,
        /// Blocks to mine then keep serving (0 = mine forever while serving)
        #[arg(long, default_value_t = 0)]
        mine_blocks: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let args = Args::parse();

    match args.cmd {
        Cmd::Info | Cmd::GetBlock { .. } | Cmd::GetBalance { .. } | Cmd::Mempool => {
            run_query(&args).await
        }
        Cmd::Serve {
            listen,
            connect,
            p2p_key,
            rpc,
            wallet,
            operator_address,
            operator_vault,
            mine,
            miner_key,
            mine_blocks,
        } => {
            run_serve(
                args.chain,
                listen,
                connect,
                p2p_key,
                rpc,
                wallet,
                operator_address,
                operator_vault,
                mine,
                miner_key,
                mine_blocks,
            )
            .await
        }
    }
}

async fn run_query(args: &Args) -> Result<()> {
    if !args.chain.exists() {
        bail!(
            "chain not found at {} — run mesh-miner-cpu or mesh-node serve first",
            args.chain.display()
        );
    }
    let chain = Chain::open(&args.chain)?;

    match &args.cmd {
        Cmd::Info => {
            println!("path:    {}", chain.store().path().display());
            println!("height:  {}", chain.height());
            println!("tip:     {}", chain.tip_hash());
            println!("genesis: {}", chain.genesis_hash());
            println!("next_diff: {}", chain.next_difficulty());
            println!("blocks:  {}", chain.store().len());
            println!("mempool: {}", chain.mempool().len());
            if let Some(tip) = chain.tip() {
                println!("time:    {}", tip.header.timestamp);
                println!("diff:    {}", tip.header.difficulty);
                println!("txs:     {}", tip.txs.len());
            }
        }
        Cmd::GetBlock { height } => match chain.get_block(*height) {
            Some(b) => {
                println!("height: {}", b.header.height);
                println!("id:     {}", b.id());
                println!("prev:   {}", b.header.prev_hash);
                println!("merkle: {}", b.header.merkle_root);
                println!("nonce:  {}", b.header.nonce);
                println!("diff:   {}", b.header.difficulty);
                for (i, tx) in b.txs.iter().enumerate() {
                    println!("tx[{i}] {} memo={}", tx.txid(), tx.memo);
                    for out in &tx.outputs {
                        println!("  -> {} {}", out.address, out.amount);
                    }
                }
            }
            None => bail!("no block at height {height}"),
        },
        Cmd::GetBalance { address } => {
            let addr = Address::from_hex(address).ok_or_else(|| anyhow::anyhow!("bad address"))?;
            println!("{}", chain.balance(&addr));
        }
        Cmd::Mempool => {
            let pool = chain.mempool();
            if pool.is_empty() {
                println!("(empty)");
            } else {
                for tx in pool {
                    println!(
                        "{}  outs={}  memo={}",
                        tx.txid(),
                        tx.outputs.len(),
                        tx.memo
                    );
                }
            }
        }
        Cmd::Serve { .. } => unreachable!(),
    }
    Ok(())
}

async fn run_serve(
    chain_path: PathBuf,
    listen: String,
    seeds: Vec<String>,
    p2p_key: PathBuf,
    rpc_bind: String,
    wallet_path: PathBuf,
    operator_address: Option<String>,
    operator_vault: Option<PathBuf>,
    mine: bool,
    miner_key: PathBuf,
    mine_blocks: u64,
) -> Result<()> {
    let listen = parse_listen_addr(&listen)?;
    let seeds = expand_official_seed_dials(seeds)
        .iter()
        .map(|s| parse_seed_addr(s))
        .collect::<Result<Vec<_>>>()?;

    let mut chain = if http_sync::join_official_enabled() {
        http_sync::bootstrap_official_replica(&chain_path)?
    } else {
        Chain::open_or_genesis(&chain_path)?
    };
    if chain.apply_env_retarget_override()? {
        info!(
            interval = chain.retarget_params().interval,
            next_diff = chain.next_difficulty(),
            "MESH_FORCE_RETARGET_INTERVAL applied"
        );
    } else if chain.heal_legacy_retarget_interval()? {
        info!(
            interval = chain.retarget_params().interval,
            next_diff = chain.next_difficulty(),
            "healed stored retarget 20 → 15 (live testnet)"
        );
    }
    let wallet = load_or_create_key(&wallet_path)?;
    // N10: cold payout addr (address-only vault load; never unlocks cold keys).
    let operator = resolve_node_operator(
        wallet.address(),
        operator_address.as_deref(),
        operator_vault.as_deref(),
    );
    chain.node_operator = Some(operator);
    info!(
        height = chain.height(),
        genesis = %chain.genesis_hash(),
        next_diff = chain.next_difficulty(),
        node_operator = %operator,
        wallet = %wallet.address(),
        pruned = chain.store().is_pruned(),
        hot_from = chain.store().hot_from_height(),
        "chain ready"
    );
    if std::env::var("MESH_COLD_PRUNE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        let keep = std::env::var("MESH_KEEP_BLOCKS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(2_048u64);
        let auto = std::env::var("MESH_AUTO_PRUNE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        info!(
            keep_blocks = keep,
            auto_prune = auto,
            "MESH_COLD_PRUNE armed (archive seed should leave this unset)"
        );
    }
    let shared: SharedChain = Arc::new(Mutex::new(chain));

    // Anti-Sybil: register node bond for operator when stake is available.
    {
        let peer_hint = String::new();
        let mut c = shared.lock().await;
        if let Some(op) = c.node_operator {
            match c.register_node_bond(op, &peer_hint) {
                Ok(b) => info!(
                    address = %op,
                    stake = b.stake_atomic,
                    "node bond registered (or refreshed)"
                ),
                Err(e) => info!(error = %e, "node bond not registered yet (need ≥ 0.1 MESH)"),
            }
        }
    }

    let handle = run_node(
        shared.clone(),
        NodeConfig {
            listen,
            seeds,
            dial_interval_secs: 5,
            identity_path: p2p_key,
        },
    )
    .await?;
    info!(peer_id = %handle.local_peer_id, "node online");

    {
        let attest_kp = wallet.clone();
        let chain = shared.clone();
        let net = handle.clone();
        tokio::spawn(async move {
            finality_loop::run(chain, net, attest_kp).await;
        });
    }

    if http_sync::enabled() {
        let bases = http_sync::seed_rpc_bases();
        http_sync::align_once(&shared, &bases).await;
        let chain = shared.clone();
        let net = handle.clone();
        tokio::spawn(async move {
            http_sync::catchup_loop(chain, net, bases).await;
        });
    }

    if !rpc_bind.is_empty() {
        let data_dir = chain_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("data"));
        if let Some(tok) = mesh_rpc::ensure_ai_token(&data_dir) {
            info!(
                path = %data_dir.join("ai.token").display(),
                chars = tok.len(),
                "AI board token armed (MESH_AI_TOKEN_AUTO)"
            );
        }
        let rpc_cookie = mesh_rpc::ensure_rpc_token(&data_dir);
        info!(
            path = %data_dir.join("rpc.token").display(),
            chars = rpc_cookie.len(),
            "wallet/gov RPC cookie armed (fail-closed)"
        );
        let rpc_addr: std::net::SocketAddr = rpc_bind.parse()?;
        let state = RpcState::with_defaults(
            shared.clone(),
            Some(handle.clone()),
            Arc::new(RwLock::new(wallet)),
            wallet_path,
            Some(std::sync::Arc::<str>::from(rpc_cookie)),
        );
        if state.rpc_token.is_some() {
            info!("wallet/gov RPC surface requires MESH_RPC_TOKEN / rpc.token");
        }
        if state.ai_token.is_some() {
            info!("AI board mutate surface requires MESH_AI_TOKEN");
        }
        if !state.rpc_edges.is_empty() {
            info!(edges = ?state.rpc_edges, "advertising edge RPC URLs");
        }
        info!("public mine surface open (getblocktemplate/submitblock)");
        if std::env::var("MESH_EDGE_MODE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            info!("MESH_EDGE_MODE=1 — local mine RPC; AI via MESH_AI_UPSTREAM if set");
        }
        // Refresh bond with real peer id once online.
        {
            let peer = handle.local_peer_id.to_string();
            let mut c = shared.lock().await;
            if let Some(op) = c.node_operator {
                let _ = c.register_node_bond(op, &peer);
            }
        }
        tokio::spawn(async move {
            if let Err(e) = serve_rpc(state, rpc_addr).await {
                tracing::error!(error = %e, "rpc server exited");
            }
        });
    }

    if mine {
        let miner = load_or_create_key(&miner_key)?;
        let addr = miner.address();
        info!(%addr, "Auto-mine armed (waits for sync + a peer)");
        let shared_mine = shared.clone();
        let net = handle.clone();
        tokio::spawn(async move {
            let mut mined = 0u64;
            let mut last_wait = String::new();
            loop {
                if mine_blocks > 0 && mined >= mine_blocks {
                    info!("mine target reached; continuing to serve");
                    break;
                }

                let local_h = {
                    let c = shared_mine.lock().await;
                    c.height()
                };
                let peers = net.peer_count();
                let seed_h = tokio::task::spawn_blocking(http_sync::peek_seed_height)
                    .await
                    .ok()
                    .flatten();
                let wait = mine_wait_reason(local_h, peers, seed_h);
                if let Some(why) = wait {
                    if why != last_wait {
                        info!("Auto-mine waiting — {why}");
                        last_wait = why;
                    }
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    continue;
                }
                if !last_wait.is_empty() {
                    info!("Auto-mine live at height {local_h}");
                    last_wait.clear();
                }

                // Short lock for template; PoW off-thread so RPC stays responsive.
                let (mut block, light_pow) = {
                    let c = shared_mine.lock().await;
                    (c.mining_template(addr), c.light_pow)
                };
                let search = tokio::task::spawn_blocking(move || {
                    let ok = Chain::search_pow(&mut block, light_pow, 5_000_000);
                    (ok, block)
                })
                .await;

                match search {
                    Ok((true, block)) => {
                        let accepted = {
                            let mut c = shared_mine.lock().await;
                            c.accept_mined(block)
                        };
                        match accepted {
                            Ok(Some(block)) => {
                                mined += 1;
                                info!(
                                    height = block.header.height,
                                    diff = block.header.difficulty,
                                    id = %block.id(),
                                    "mined; announcing"
                                );
                                net.announce_block(block);
                            }
                            Ok(None) => {
                                tracing::debug!("stale template after PoW; retrying");
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "mine accept error");
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
                    }
                    Ok((false, _)) => {
                        tracing::warn!("no PoW solution in nonce window");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "mine task join error");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
                tokio::task::yield_now().await;
            }
        });
    }

    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

/// Keep the official public seed in the dial list.
fn expand_official_seed_dials(seeds: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let push = |out: &mut Vec<String>, raw: &str| {
        let t = raw.trim();
        if t.is_empty() {
            return;
        }
        if !out.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    };
    for s in &seeds {
        push(&mut out, s);
    }
    let dns = mesh_types::SEED_DNS;
    if !out
        .iter()
        .any(|s| s.to_ascii_lowercase().contains(dns))
    {
        push(&mut out, &mesh_types::default_seed_p2p());
    }
    out
}

fn mine_wait_reason(local_h: u64, peers: usize, seed_h: Option<u64>) -> Option<String> {
    if peers == 0 {
        return Some("no P2P peers (would fork)".into());
    }
    match seed_h {
        None => Some("seed tip unknown".into()),
        Some(h) if h > local_h.saturating_add(3) => {
            Some(format!("behind seed ({local_h} / {h})"))
        }
        Some(_) => None,
    }
}

fn resolve_node_operator(
    hot: Address,
    cli_address: Option<&str>,
    cli_vault: Option<&std::path::Path>,
) -> Address {
    // Precedence: CLI address → env address → CLI vault → env vault → hot wallet.
    // Vault load is address-only (plaintext field); never unlocks cold keys.
    if let Some(raw) = cli_address {
        let t = raw.trim();
        if !t.is_empty() {
            if let Some(a) = Address::from_hex(t) {
                return a;
            }
            tracing::warn!(%t, "--operator-address invalid — continuing resolution");
        }
    }
    if let Ok(raw) = std::env::var("MESH_OPERATOR_ADDRESS") {
        let t = raw.trim();
        if !t.is_empty() {
            if let Some(a) = Address::from_hex(t) {
                return a;
            }
            tracing::warn!(%t, "MESH_OPERATOR_ADDRESS invalid — continuing resolution");
        }
    }
    if let Some(path) = cli_vault {
        if let Some(a) = load_operator_from_vault(path) {
            return a;
        }
    }
    if let Ok(raw) = std::env::var("MESH_OPERATOR_VAULT") {
        let t = raw.trim();
        if !t.is_empty() {
            if let Some(a) = load_operator_from_vault(std::path::Path::new(t)) {
                return a;
            }
        }
    }
    hot
}

fn load_operator_from_vault(path: &std::path::Path) -> Option<Address> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "operator vault unreadable");
            return None;
        }
    };
    let vault = match VaultFile::from_json(&text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "operator vault JSON invalid");
            return None;
        }
    };
    let t = vault.address.trim();
    if t.is_empty() {
        tracing::warn!(path = %path.display(), "operator vault has empty address");
        return None;
    }
    match Address::from_hex(t) {
        Some(a) => {
            info!(path = %path.display(), address = %a, "loaded cold operator from vault (address only)");
            Some(a)
        }
        None => {
            tracing::warn!(path = %path.display(), %t, "operator vault address invalid");
            None
        }
    }
}

fn load_or_create_key(path: &PathBuf) -> Result<Keypair> {
    if path.exists() {
        let _ = mesh_crypto::restrict_secret_file(path);
        let hex = std::fs::read_to_string(path)?;
        return Ok(Keypair::from_hex(hex.trim())?);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let kp = Keypair::generate();
    mesh_crypto::write_secret_file_no_clobber(path, kp.to_hex().as_bytes())?;
    info!(path = %path.display(), "wrote owner-only key (0600)");
    Ok(kp)
}
