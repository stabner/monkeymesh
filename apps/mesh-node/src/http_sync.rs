//! Catch-up from the seed's HTTP snapshot when P2P QUIC is blocked
//! (typical on LAN without NAT hairpin). Local files still store the replica.

use std::time::Duration;

use mesh_chain::FinalityAttestation;
use mesh_p2p::{NetworkHandle, SharedChain};
use mesh_types::{Block, ProtocolEnvelopes};
use serde::Deserialize;
use tracing::{info, warn};

const BATCH: u32 = 100;
const HTTP_TIMEOUT: Duration = Duration::from_secs(45);

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
        for base in &bases {
            let tip = tokio::task::spawn_blocking({
                let base = base.clone();
                move || fetch_tip(&base)
            })
            .await
            .ok()
            .flatten();
            let Some(tip) = tip else {
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

        if seed_h < local_h {
            tokio::time::sleep(Duration::from_secs(12)).await;
            continue;
        }

        let from = if seed_h > local_h {
            local_h.saturating_add(1)
        } else {
            local_h
        };
        let batch = tokio::task::spawn_blocking({
            let base = base.clone();
            move || fetch_batch(&base, from, BATCH)
        })
        .await
        .ok()
        .flatten();

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
            let fanout = block.clone();
            let result = {
                let mut c = chain.lock().await;
                c.import_block(block)
            };
            match result {
                Ok(true) => {
                    imported += 1;
                    net.announce_block(fanout);
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
                        c.import_block(block).unwrap_or(false)
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
            tokio::time::sleep(Duration::from_secs(8)).await;
        }

        pull_finality(&chain, &net, &bases).await;
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
    let resp = ureq::get(&url).timeout(HTTP_TIMEOUT).call().ok()?;
    resp.into_json().ok()
}

fn fetch_tip(base: &str) -> Option<NodeTip> {
    let url = format!("{}/v1/getnodeinfo", base.trim_end_matches('/'));
    let resp = ureq::get(&url).timeout(HTTP_TIMEOUT).call().ok()?;
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
