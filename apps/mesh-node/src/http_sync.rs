//! Catch-up from the seed's HTTP snapshot when P2P QUIC is blocked
//! (typical on LAN without NAT hairpin). Local files still store the replica.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use mesh_chain::{Chain, FinalityAttestation, MIN_COLD_PRUNE_KEEP};
use mesh_p2p::{NetworkHandle, SharedChain};
use mesh_types::{Address, Amount, Block, Hash, OutPoint, ProtocolEnvelopes, Utxo};
use serde::Deserialize;
use tracing::{info, warn};

const BATCH: u32 = 200;
const HTTP_TIMEOUT: Duration = Duration::from_secs(45);
/// Dead edge RPCs must not stall catch-up. Snapshot downloads keep HTTP_TIMEOUT.
const TIP_TIMEOUT: Duration = Duration::from_secs(4);
/// Fully re-hash Fusion only for this many blocks behind the seed tip.
const POW_VERIFY_TAIL: u64 = 32;
const UTXO_PAGE: usize = 2_000;

#[derive(Deserialize)]
struct SnapBatch {
    genesis: Option<String>,
    #[serde(default)]
    blocks: Vec<WireBlock>,
}

#[derive(Deserialize)]
struct WireBlock {
    block_hex: Option<String>,
}

#[derive(Deserialize)]
struct UtxoPage {
    #[serde(default)]
    utxo_count: Option<usize>,
    #[serde(default)]
    utxos: Vec<WireUtxo>,
}

#[derive(Deserialize)]
struct WireUtxo {
    txid: String,
    #[serde(default)]
    vout: u32,
    address: String,
    #[serde(default)]
    atomic: u64,
}

#[derive(Deserialize)]
struct ProposalsWire {
    active_envelopes: Option<ProtocolEnvelopes>,
}

#[derive(Deserialize)]
struct NodeTip {
    height: Option<u64>,
    genesis: Option<String>,
    #[serde(default)]
    tip: Option<String>,
}

/// Every public mesh RPC (seed + edges). Catch-up is bidirectional: a block
/// found on edge2 must be pulled by the seed, not only the other way around.
pub fn seed_rpc_bases() -> Vec<String> {
    mesh_types::default_rpc_urls()
}

pub fn enabled() -> bool {
    match std::env::var("MESH_HTTP_SYNC") {
        Ok(v) => v != "0" && !v.eq_ignore_ascii_case("false"),
        Err(_) => true,
    }
}

/// All-in-One sidecar: never mint a private genesis — join the official seed's block 0.
pub fn join_official_enabled() -> bool {
    match std::env::var("MESH_JOIN_OFFICIAL") {
        Ok(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        Err(_) => false,
    }
}

/// Fetch official genesis over HTTP. If the local store is empty or a height-0
/// private genesis, replace it. Refuses to run if a taller private chain exists.
#[allow(dead_code)]
pub fn prepare_official_genesis(chain_path: &Path) -> Result<Block> {
    let (seed_genesis, base, genesis) = fetch_official_genesis()
        .context("cannot fetch official genesis from seed RPC — stay on the public seed until it answers")?;

    let local = Chain::open(chain_path).ok();
    if let Some(chain) = local {
        if !chain.store().is_empty() {
            let local_g = chain.genesis_hash().to_string();
            let h = chain.height();
            if local_g.eq_ignore_ascii_case(&seed_genesis) {
                info!(
                    genesis = %local_g,
                    height = h,
                    "local store already on official genesis"
                );
                return Ok(genesis);
            }
            if h > 0 {
                bail!(
                    "local chain genesis {local_g} != official {seed_genesis} (height {h}). Move the data dir aside — refusing to wipe a non-empty private chain"
                );
            }
            drop(chain);
            info!(
                local = %local_g,
                official = %seed_genesis,
                "discarding height-0 private genesis; joining official chain from {base}"
            );
            Chain::wipe_store_files(chain_path);
        }
    }
    Ok(genesis)
}

fn fetch_official_genesis() -> Result<(String, String, Block)> {
    let mut last_err = String::from("no seed RPC answered");
    for base in seed_rpc_bases() {
        match fetch_official_genesis_from(&base) {
            Ok(pair) => return Ok((pair.0, base, pair.1)),
            Err(e) => last_err = format!("{base}: {e}"),
        }
    }
    bail!("{last_err}")
}

fn fetch_official_genesis_from(base: &str) -> Result<(String, Block)> {
    let tip = fetch_tip(base).context("getnodeinfo")?;
    let seed_g = tip
        .genesis
        .filter(|s| !s.is_empty())
        .context("seed getnodeinfo missing genesis")?;
    let batch = fetch_batch(base, 0, 1).context("snapshot/download from=0")?;
    if let Some(g) = batch.genesis.as_deref() {
        if !g.is_empty() && !g.eq_ignore_ascii_case(&seed_g) {
            bail!("snapshot genesis {g} != getnodeinfo {seed_g}");
        }
    }
    let wire = batch
        .blocks
        .first()
        .and_then(|w| w.block_hex.as_deref())
        .context("snapshot has no genesis block")?;
    let raw = hex::decode(wire).context("genesis block hex")?;
    let block: Block = bincode::deserialize(&raw).context("genesis block decode")?;
    if block.header.height != 0 {
        bail!("snapshot first block is height {}, not genesis", block.header.height);
    }
    let id = block.id().to_string();
    if !id.eq_ignore_ascii_case(&seed_g) {
        bail!("decoded genesis {id} != seed {seed_g}");
    }
    Ok((seed_g, block))
}

/// Official sidecar: UTXO snapshot + hot tail when far behind, else genesis-only join.
pub fn bootstrap_official_replica(chain_path: &Path) -> Result<Chain> {
    let (seed_genesis, base, genesis_block) = fetch_official_genesis()
        .context("cannot fetch official genesis from seed RPC — stay on the public seed until it answers")?;

    let local = Chain::open(chain_path).ok();
    let local_state = local.as_ref().and_then(|c| {
        if c.store().is_empty() {
            None
        } else {
            Some((c.height(), c.genesis_hash().to_string()))
        }
    });
    drop(local);

    if let Some((h, g)) = &local_state {
        if !g.eq_ignore_ascii_case(&seed_genesis) {
            if *h > 0 {
                bail!(
                    "local chain genesis {g} != official {seed_genesis} (height {h}). Move the data dir aside — refusing to wipe a non-empty private chain"
                );
            }
            info!(
                local = %g,
                official = %seed_genesis,
                "discarding height-0 private genesis; joining official chain from {base}"
            );
            Chain::wipe_store_files(chain_path);
        }
    }

    let tip = fetch_tip(&base).context("getnodeinfo for fast-sync")?;
    let seed_h = tip.height.unwrap_or(0);
    let seed_tip = tip.tip.clone().unwrap_or_default();
    let behind = match &local_state {
        Some((h, g)) if g.eq_ignore_ascii_case(&seed_genesis) => seed_h.saturating_sub(*h),
        _ => seed_h,
    };

    if seed_h >= MIN_COLD_PRUNE_KEEP && behind > MIN_COLD_PRUNE_KEEP.saturating_mul(2) {
        match install_utxo_snapshot(chain_path, &base, &seed_genesis, seed_h, &seed_tip) {
            Ok(()) => {
                info!(
                    height = seed_h,
                    behind,
                    "official UTXO snapshot installed — skipped historical Fusion replay"
                );
                return Chain::open(chain_path).map_err(|e| anyhow!("{e}"));
            }
            Err(e) => warn!(error = %e, "UTXO snapshot failed — linear HTTP catch-up"),
        }
    }

    let mut c = Chain::open(chain_path).map_err(|e| anyhow!("{e}"))?;
    if c.store().is_empty() {
        if !c.import_block(genesis_block).map_err(|e| anyhow!("{e}"))? {
            bail!("failed to import official genesis");
        }
        info!(
            id = %c.genesis_hash(),
            "joined official genesis from seed HTTP (no local mint)"
        );
    }
    Ok(c)
}

fn install_utxo_snapshot(
    chain_path: &Path,
    base: &str,
    seed_genesis: &str,
    seed_h: u64,
    seed_tip: &str,
) -> Result<()> {
    let utxos = fetch_all_utxos(base)?;
    let from = seed_h.saturating_sub(MIN_COLD_PRUNE_KEEP.saturating_sub(1));
    let hot = fetch_blocks_range(base, from, seed_h)?;
    let last = hot.last().context("official snapshot hot tail empty")?;
    if last.header.height != seed_h {
        bail!(
            "hot tail height {} != seed {seed_h}",
            last.header.height
        );
    }
    if !seed_tip.is_empty() && !last.id().to_string().eq_ignore_ascii_case(seed_tip) {
        bail!(
            "hot tail tip {} != seed {seed_tip} (tip moved — retry linear)",
            last.id()
        );
    }
    let genesis = Hash::from_hex(seed_genesis).map_err(|e| anyhow!("genesis hex: {e}"))?;
    info!(
        utxos = utxos.len(),
        hot_from = from,
        hot_blocks = hot.len(),
        "downloading official snapshot"
    );
    Chain::wipe_store_files(chain_path);
    let mut c = Chain::open(chain_path).map_err(|e| anyhow!("{e}"))?;
    c.install_official_prune(genesis, utxos, hot)
        .map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

fn fetch_all_utxos(base: &str) -> Result<std::collections::HashMap<OutPoint, Utxo>> {
    let mut out = std::collections::HashMap::new();
    let mut offset = 0usize;
    loop {
        let page = fetch_utxo_page(base, offset, UTXO_PAGE)
            .with_context(|| format!("snapshot/utxos offset={offset}"))?;
        let n = page.utxos.len();
        for row in page.utxos {
            let txid = Hash::from_hex(&row.txid).map_err(|e| anyhow!("utxo txid: {e}"))?;
            let address = Address::from_hex(&row.address)
                .ok_or_else(|| anyhow!("utxo address {}", row.address))?;
            out.insert(
                OutPoint::new(txid, row.vout),
                Utxo {
                    address,
                    amount: Amount::from_atomic(row.atomic),
                },
            );
        }
        offset += n;
        if n < UTXO_PAGE || offset >= page.utxo_count.unwrap_or(offset) {
            break;
        }
    }
    if out.is_empty() {
        bail!("seed UTXO snapshot empty");
    }
    Ok(out)
}

fn fetch_blocks_range(base: &str, from: u64, through: u64) -> Result<Vec<Block>> {
    let mut blocks = Vec::new();
    let mut h = from;
    while h <= through {
        let batch = fetch_batch(base, h, BATCH).with_context(|| format!("snapshot from={h}"))?;
        let got = decode_batch_blocks(&batch)?;
        if got.is_empty() {
            bail!("empty snapshot batch at {h}");
        }
        let next = got.last().map(|b| b.header.height.saturating_add(1)).unwrap_or(h);
        blocks.extend(got);
        if next <= h {
            bail!("snapshot did not advance from {h}");
        }
        h = next;
    }
    blocks.retain(|b| b.header.height >= from && b.header.height <= through);
    Ok(blocks)
}

fn decode_batch_blocks(batch: &SnapBatch) -> Result<Vec<Block>> {
    let mut out = Vec::new();
    for wire in &batch.blocks {
        let Some(hex) = wire.block_hex.as_deref() else {
            continue;
        };
        let raw = hex::decode(hex).context("block hex")?;
        let block: Block = bincode::deserialize(&raw).context("block decode")?;
        out.push(block);
    }
    Ok(out)
}

pub async fn catchup_loop(chain: SharedChain, net: NetworkHandle, bases: Vec<String>) {
    info!(peers = bases.len(), "HTTP mesh catch-up armed");
    loop {
        let (local_h, local_genesis, local_tip) = {
            let c = chain.lock().await;
            (
                c.height(),
                c.genesis_hash().to_string(),
                c.tip_hash().to_string(),
            )
        };

        let mut best: Option<(u64, String)> = None;
        let mut fork_probe: Option<(u64, String)> = None;
        let tip_joins: Vec<_> = bases
            .iter()
            .map(|base| {
                let base = base.clone();
                tokio::task::spawn_blocking(move || (base.clone(), fetch_tip(&base)))
            })
            .collect();
        for join in tip_joins {
            let Ok((base, Some(tip))) = join.await else {
                continue;
            };
            if let Some(g) = tip.genesis.as_deref() {
                if !g.is_empty() && g != local_genesis {
                    warn!(
                        peer = %base,
                        seed = %g,
                        local = %local_genesis,
                        "peer genesis does not match local chain — skip"
                    );
                    continue;
                }
            }
            let h = tip.height.unwrap_or(0);
            if h > local_h && best.as_ref().map(|(bh, _)| h > *bh).unwrap_or(true) {
                best = Some((h, base.clone()));
            } else if h == local_h {
                if let Some(t) = tip.tip.as_deref() {
                    if !t.is_empty() && t != local_tip {
                        fork_probe = Some((h, base.clone()));
                    }
                }
            }
        }

        let (seed_h, base) = match best.or(fork_probe) {
            Some(pair) => pair,
            None => {
                let _ = net.peer_count();
                tokio::time::sleep(Duration::from_secs(12)).await;
                continue;
            }
        };

        align_retarget_from_seed(&chain, &base).await;

        let behind = seed_h.saturating_sub(local_h);
        {
            let mut c = chain.lock().await;
            c.set_bulk_import(behind > 8);
        }

        if seed_h < local_h {
            tokio::time::sleep(Duration::from_secs(12)).await;
            continue;
        }

        let from = if seed_h > local_h {
            local_h.saturating_add(1)
        } else {
            local_h
        };
        let prefetch = tokio::task::spawn_blocking({
            let base = base.clone();
            move || fetch_batch(&base, from, BATCH)
        });
        let batch = prefetch.await.ok().flatten();

        let Some(batch) = batch else {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        };

        if let Some(g) = batch.genesis.as_deref() {
            if !g.is_empty() && g != local_genesis {
                warn!(seed = %g, local = %local_genesis, "snapshot genesis mismatch");
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
        }

        info!(
            from,
            count = batch.blocks.len(),
            seed_h,
            %base,
            "HTTP catch-up pulling official blocks"
        );

        let mut imported = 0u32;
        let mut need_parent_probe = false;
        for wire in &batch.blocks {
            let Some(hex) = wire.block_hex.as_deref() else {
                continue;
            };
            let Ok(raw) = hex::decode(hex) else {
                warn!("bad snapshot block hex");
                break;
            };
            let Ok(block) = bincode::deserialize::<Block>(&raw) else {
                warn!("bad snapshot block blob");
                break;
            };
            let height = block.header.height;
            let verify_pow = seed_h.saturating_sub(height) <= POW_VERIFY_TAIL;
            let fanout = block.clone();
            let result = {
                let mut c = chain.lock().await;
                c.import_official_snapshot_block(block, verify_pow)
            };
            match result {
                Ok(true) => {
                    imported += 1;
                    if behind <= 8 {
                        net.announce_block(fanout);
                    }
                }
                Ok(false) => {
                    need_parent_probe = true;
                }
                Err(e) => {
                    let expected = {
                        let c = chain.lock().await;
                        c.next_difficulty()
                    };
                    warn!(
                        height,
                        expected,
                        reason = %e,
                        "HTTP catch-up rejected block"
                    );
                    info!("Sync rejected block {height} ({e})");
                    align_retarget_from_seed(&chain, &base).await;
                    need_parent_probe = true;
                    break;
                }
            }
        }

        if imported == 0 && need_parent_probe && local_h > 0 {
            let parent_from = local_h;
            if let Some(parent_batch) = tokio::task::spawn_blocking({
                let base = base.clone();
                move || fetch_batch(&base, parent_from, 1)
            })
            .await
            .ok()
            .flatten()
            {
                for wire in &parent_batch.blocks {
                    let Some(hex) = wire.block_hex.as_deref() else {
                        continue;
                    };
                    let Ok(raw) = hex::decode(hex) else {
                        continue;
                    };
                    let Ok(block) = bincode::deserialize::<Block>(&raw) else {
                        continue;
                    };
                    let fanout = block.clone();
                    let ok = {
                        let mut c = chain.lock().await;
                        c.import_official_snapshot_block(block, false).unwrap_or(false)
                    };
                    if ok {
                        imported += 1;
                        net.announce_block(fanout);
                    }
                }
            }
        }

        let now = {
            let c = chain.lock().await;
            c.height()
        };
        if imported > 0 {
            info!("Syncing from mesh: height {now} / {seed_h} via {base}");
        } else if batch.blocks.is_empty() {
            tokio::time::sleep(Duration::from_secs(8)).await;
        }

        if now >= seed_h {
            info!("Caught up with mesh peer at height {now}");
            {
                let mut c = chain.lock().await;
                c.set_bulk_import(false);
            }
            tokio::time::sleep(Duration::from_secs(8)).await;
            pull_finality(&chain, &net, &bases).await;
        }

        tokio::task::yield_now().await;
    }
}

/// Align envelopes from the first seed RPC that answers (startup + reject heal).
pub async fn align_once(chain: &SharedChain, bases: &[String]) {
    for base in bases {
        align_retarget_from_seed(chain, base).await;
        let interval = {
            let c = chain.lock().await;
            c.active_envelopes().retarget_interval
        };
        if interval == 15 {
            return;
        }
    }
}

pub fn peek_seed_height() -> Option<u64> {
    let mut best = None;
    for base in seed_rpc_bases() {
        if let Some(t) = fetch_tip(&base) {
            if let Some(h) = t.height {
                best = Some(best.map_or(h, |b: u64| b.max(h)));
            }
        }
    }
    best
}

#[derive(Deserialize)]
struct FinalityWire {
    #[serde(default)]
    pending: Vec<FinalityAttestation>,
}

async fn pull_finality(chain: &SharedChain, net: &NetworkHandle, bases: &[String]) {
    for base in bases {
        let wire = tokio::task::spawn_blocking({
            let base = base.clone();
            move || fetch_finality(&base)
        })
        .await
        .ok()
        .flatten();
        let Some(wire) = wire else {
            continue;
        };
        for att in wire.pending {
            let fanout = att.clone();
            let result = {
                let mut c = chain.lock().await;
                c.record_finality_attestation(att)
            };
            if let Ok(ing) = result {
                if ing.new_vote {
                    net.announce_finality_attest(fanout);
                }
                if let Some(addr) = ing.slashed {
                    net.announce_slash_mark(addr.to_string(), String::new(), 0, 0, String::new());
                }
            }
        }
    }
}

fn fetch_finality(base: &str) -> Option<FinalityWire> {
    let url = format!("{}/v1/finality", base.trim_end_matches('/'));
    let resp = ureq::get(&url).timeout(TIP_TIMEOUT).call().ok()?;
    resp.into_json().ok()
}

fn fetch_tip(base: &str) -> Option<NodeTip> {
    let url = format!("{}/v1/getnodeinfo", base.trim_end_matches('/'));
    let resp = ureq::get(&url).timeout(TIP_TIMEOUT).call().ok()?;
    resp.into_json().ok()
}

fn fetch_proposals(base: &str) -> Option<ProposalsWire> {
    let url = format!("{}/v1/proposals", base.trim_end_matches('/'));
    let resp = ureq::get(&url).timeout(HTTP_TIMEOUT).call().ok()?;
    resp.into_json().ok()
}

async fn align_retarget_from_seed(chain: &SharedChain, base: &str) {
    let Some(p) = tokio::task::spawn_blocking({
        let base = base.to_string();
        move || fetch_proposals(&base)
    })
    .await
    .ok()
    .flatten()
    else {
        return;
    };
    let Some(seed_env) = p.active_envelopes else {
        return;
    };
    let interval = seed_env.retarget_interval.clamp(10, 40);
    let mut c = chain.lock().await;
    let mut env = c.active_envelopes();
    if env.retarget_interval == interval
        && env.retarget_step == seed_env.retarget_step
        && env.min_difficulty_floor == seed_env.min_difficulty_floor
    {
        return;
    }
    env.retarget_interval = interval;
    env.retarget_step = seed_env.retarget_step.clamp(1, 2);
    env.min_difficulty_floor = seed_env.min_difficulty_floor;
    if c.set_active_envelopes(env).is_ok() {
        info!("Aligned retarget with seed (interval {interval})");
    }
}

fn fetch_batch(base: &str, from: u64, limit: u32) -> Option<SnapBatch> {
    let url = format!(
        "{}/v1/snapshot/download?from={from}&limit={limit}",
        base.trim_end_matches('/')
    );
    let resp = ureq::get(&url).timeout(HTTP_TIMEOUT).call().ok()?;
    resp.into_json().ok()
}

fn fetch_utxo_page(base: &str, offset: usize, limit: usize) -> Option<UtxoPage> {
    let url = format!(
        "{}/v1/snapshot/utxos?offset={offset}&limit={limit}",
        base.trim_end_matches('/')
    );
    let resp = ureq::get(&url).timeout(HTTP_TIMEOUT).call().ok()?;
    resp.into_json().ok()
}
