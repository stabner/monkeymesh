//! Track external miners that pull templates / submit blocks (for node UI feed).

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde::Serialize;

const MAX_EVENTS: usize = 64;
const HEARTBEAT_SECS: u64 = 30;
const IDLE_SECS: u64 = 45;

#[derive(Clone, Debug, Serialize)]
pub struct MiningFeedEvent {
    pub id: u64,
    pub msg: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActiveMiner {
    pub address: String,
    pub short: String,
    pub height: u64,
    pub templates: u64,
    pub blocks_found: u64,
    pub last_seen_secs: u64,
    pub mining: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MiningStatus {
    pub active_miners: Vec<ActiveMiner>,
    pub events: Vec<MiningFeedEvent>,
}

struct MinerSession {
    last_template: Instant,
    last_heartbeat_log: Instant,
    last_height: u64,
    prev_height: u64,
    template_count: u64,
    blocks_found: u64,
    announced_active: bool,
}

pub struct MiningActivity {
    next_id: u64,
    events: VecDeque<MiningFeedEvent>,
    sessions: HashMap<String, MinerSession>,
}

impl Default for MiningActivity {
    fn default() -> Self {
        Self {
            next_id: 1,
            events: VecDeque::new(),
            sessions: HashMap::new(),
        }
    }
}

fn short_addr(address: &str) -> String {
    let a = address.trim();
    if a.len() <= 16 {
        return a.to_string();
    }
    format!("{}…{}", &a[..8], &a[a.len().saturating_sub(6)..])
}

impl MiningActivity {
    fn push(&mut self, msg: impl Into<String>, kind: &str) {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.events.push_back(MiningFeedEvent {
            id,
            msg: msg.into(),
            kind: kind.to_string(),
        });
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }

    /// True if this payout address pulled a template for `height` (exam gate).
    /// Accepts the current template height or the previous one (5s blocks race the exam POST).
    pub fn has_template_for(&self, address: &str, height: u64) -> bool {
        self.sessions
            .get(address.trim())
            .map(|s| {
                s.announced_active && (s.last_height == height || s.prev_height == height)
            })
            .unwrap_or(false)
    }

    pub fn note_template(&mut self, address: &str, height: u64) {
        let address = address.trim();
        if address.is_empty() {
            return;
        }
        let now = Instant::now();
        let short = short_addr(address);
        let entry = self.sessions.get_mut(address);
        match entry {
            None => {
                self.sessions.insert(
                    address.to_string(),
                    MinerSession {
                        last_template: now,
                        last_heartbeat_log: now,
                        last_height: height,
                        prev_height: 0,
                        template_count: 1,
                        blocks_found: 0,
                        announced_active: true,
                    },
                );
                self.push(
                    format!("Miner connected · working on height {height} · {short}"),
                    "ok",
                );
            }
            Some(sess) => {
                let was_idle = !sess.announced_active
                    || sess.last_template.elapsed() >= Duration::from_secs(IDLE_SECS);
                sess.last_template = now;
                if sess.last_height != height {
                    sess.prev_height = sess.last_height;
                    sess.last_height = height;
                }
                sess.template_count = sess.template_count.saturating_add(1);
                if was_idle {
                    sess.announced_active = true;
                    sess.last_heartbeat_log = now;
                    self.push(
                        format!("Miner connected · working on height {height} · {short}"),
                        "ok",
                    );
                } else if sess.last_heartbeat_log.elapsed() >= Duration::from_secs(HEARTBEAT_SECS)
                {
                    sess.last_heartbeat_log = now;
                    self.push(
                        format!("Miner hashing · height {height} · {short}"),
                        "ok",
                    );
                }
            }
        }
    }

    pub fn note_block_found(&mut self, address: &str, height: u64) {
        let short = if address.is_empty() {
            "unknown".to_string()
        } else {
            short_addr(address)
        };
        if let Some(sess) = self.sessions.get_mut(address) {
            sess.blocks_found = sess.blocks_found.saturating_add(1);
            sess.last_height = height;
            sess.last_template = Instant::now();
            sess.announced_active = true;
        }
        self.push(
            format!("Miner found block #{height} · CPU reward · {short}"),
            "ok",
        );
    }

    pub fn note_stale_submit(&mut self, address: &str, height: u64) {
        let short = if address.is_empty() {
            "unknown".to_string()
        } else {
            short_addr(address)
        };
        if let Some(sess) = self.sessions.get_mut(address) {
            if sess.last_heartbeat_log.elapsed() >= Duration::from_secs(HEARTBEAT_SECS) {
                sess.last_heartbeat_log = Instant::now();
                self.push(
                    format!("Miner submitted stale block #{height} (tip moved) · {short}"),
                    "warn",
                );
            }
        }
    }

    fn sweep_idle(&mut self) {
        let idle = Duration::from_secs(IDLE_SECS);
        let mut went_quiet = Vec::new();
        for (addr, sess) in self.sessions.iter_mut() {
            if sess.announced_active && sess.last_template.elapsed() >= idle {
                sess.announced_active = false;
                went_quiet.push((short_addr(addr), sess.last_height));
            }
        }
        for (short, height) in went_quiet {
            self.push(
                format!("Miner went quiet · last height {height} · {short}"),
                "warn",
            );
        }
    }

    /// Snapshot for RPC. `after_id`: only return events with id > after_id.
    pub fn status_since(&mut self, after_id: u64) -> MiningStatus {
        self.sweep_idle();
        let now = Instant::now();
        let mut active: Vec<ActiveMiner> = self
            .sessions
            .iter()
            .map(|(addr, sess)| ActiveMiner {
                address: addr.clone(),
                short: short_addr(addr),
                height: sess.last_height,
                templates: sess.template_count,
                blocks_found: sess.blocks_found,
                last_seen_secs: now.saturating_duration_since(sess.last_template).as_secs(),
                mining: sess.announced_active
                    && sess.last_template.elapsed() < Duration::from_secs(IDLE_SECS),
            })
            .collect();
        active.sort_by(|a, b| b.mining.cmp(&a.mining).then(a.short.cmp(&b.short)));
        let events: Vec<_> = self
            .events
            .iter()
            .filter(|e| e.id > after_id)
            .cloned()
            .collect();
        MiningStatus {
            active_miners: active,
            events,
        }
    }
}

/// Tip + mempool fingerprint keyed cache (Build/27 N7).
fn template_ttl_ms() -> u64 {
    std::env::var("MESH_TEMPLATE_TTL_MS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(5_000)
        .max(100)
}

fn template_cache_cap() -> usize {
    std::env::var("MESH_TEMPLATE_CACHE_CAP")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(4_000)
        .clamp(64, 50_000)
}

#[derive(Clone)]
pub struct CachedTemplate {
    pub height: u64,
    pub difficulty: u32,
    pub soft_diff_hint: u32,
    pub light_pow: bool,
    pub pow_version: u8,
    pub pow_recipe: String,
    pub assigned_role: String,
    pub mesh_strength: u64,
    pub address: String,
    pub block_hex: String,
    pub exam_root: String,
    pub exam_scenario: String,
    pub exam_title: String,
    pub exam_payload_hex: String,
    pub exam_job_id: String,
    pub fair_split: bool,
    pub cpu_bps: u16,
    pub gpu_bps: u16,
    pub node_bps: u16,
}

struct CachedEntry {
    built: Instant,
    tmpl: CachedTemplate,
}

#[derive(Default)]
pub struct TemplateCache {
    tip: String,
    mempool_fp: u64,
    soft_diff: u32,
    scores_epoch: u64,
    entries: HashMap<String, CachedEntry>,
    pub hits: u64,
    pub misses: u64,
}

impl TemplateCache {
    pub fn clear(&mut self) {
        self.tip.clear();
        self.mempool_fp = 0;
        self.soft_diff = 0;
        self.scores_epoch = 0;
        self.entries.clear();
    }

    fn key_match(&self, tip: &str, mempool_fp: u64, soft_diff: u32, scores_epoch: u64) -> bool {
        self.tip == tip
            && self.mempool_fp == mempool_fp
            && self.soft_diff == soft_diff
            && self.scores_epoch == scores_epoch
    }

    fn evict_oldest(&mut self) {
        let cap = template_cache_cap();
        while self.entries.len() >= cap {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.built)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                self.entries.remove(&k);
            } else {
                break;
            }
        }
    }

    pub fn get(
        &mut self,
        tip: &str,
        mempool_fp: u64,
        soft_diff: u32,
        scores_epoch: u64,
        address: &str,
    ) -> Option<CachedTemplate> {
        if !self.key_match(tip, mempool_fp, soft_diff, scores_epoch) {
            self.tip = tip.to_string();
            self.mempool_fp = mempool_fp;
            self.soft_diff = soft_diff;
            self.scores_epoch = scores_epoch;
            self.entries.clear();
            self.misses = self.misses.saturating_add(1);
            return None;
        }
        let e = match self.entries.get(address) {
            Some(e) => e,
            None => {
                self.misses = self.misses.saturating_add(1);
                return None;
            }
        };
        if e.built.elapsed() > Duration::from_millis(template_ttl_ms()) {
            self.misses = self.misses.saturating_add(1);
            return None;
        }
        self.hits = self.hits.saturating_add(1);
        Some(e.tmpl.clone())
    }

    pub fn put(
        &mut self,
        tip: &str,
        mempool_fp: u64,
        soft_diff: u32,
        scores_epoch: u64,
        tmpl: CachedTemplate,
    ) {
        if !self.key_match(tip, mempool_fp, soft_diff, scores_epoch) {
            self.tip = tip.to_string();
            self.mempool_fp = mempool_fp;
            self.soft_diff = soft_diff;
            self.scores_epoch = scores_epoch;
            self.entries.clear();
        }
        self.evict_oldest();
        self.entries.insert(
            tmpl.address.clone(),
            CachedEntry {
                built: Instant::now(),
                tmpl,
            },
        );
    }
}
