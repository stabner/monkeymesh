//! MonkeyMesh Node — native desktop GUI (starts/stops mesh-node.exe).

#![windows_subsystem = "windows"]

mod theme;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, ScrollArea, TextureHandle};
use serde::{Deserialize, Serialize};
use theme::{
    body_text, danger_btn, display_text, dual_rail, field_label, ghost_btn, ghost_btn_enabled,
    leaf_radius,
    paint_brand_backdrop, paint_resize_grip, pointer, primary_btn, tile, CYAN, DANGER, FIELD_BG,
    INK, MUTED, OK, RULE,
    TEXT_BLUE, WARN,
};

const SCENE_PNG: &[u8] = include_bytes!("../assets/coin_scene.png");
const MASCOT_PNG: &[u8] = include_bytes!("../assets/mascot.png");
const COIN_PNG: &[u8] = include_bytes!("../assets/coin_mark.png");

const WIN_W: f32 = 960.0;
const WIN_H: f32 = 720.0;
const LOG_CAP: usize = 40;
const BOTTOM_H_DEFAULT: f32 = 200.0;
const BOTTOM_H_MIN: f32 = 88.0;
const BOTTOM_H_MAX: f32 = 560.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NodeConfig {
    #[serde(default = "default_listen")]
    listen: String,
    #[serde(default = "default_rpc")]
    rpc: String,
    #[serde(default = "default_connect")]
    connect: Vec<String>,
    #[serde(default)]
    mine: bool,
    #[serde(default)]
    mine_blocks: u64,
    #[serde(default = "default_chain")]
    chain: String,
    #[serde(default = "default_wallet")]
    wallet: String,
    #[serde(default = "default_p2p")]
    p2p_key: String,
    #[serde(default = "default_miner_key")]
    miner_key: String,
    /// Cold node-market payout address (optional; overrides vault / hot wallet)
    #[serde(default)]
    operator_address: String,
    /// Vault JSON path — plaintext address only, no unlock (optional)
    #[serde(default)]
    operator_vault: String,
    #[serde(default = "default_orch")]
    orch: String,
    #[serde(default = "default_bottom_h")]
    bottom_h: f32,
}

fn default_listen() -> String {
    // Home nodes must not bind the seed P2P port.
    "0.0.0.0:39011".into()
}
fn default_rpc() -> String {
    "0.0.0.0:18080".into()
}
fn default_orch() -> String {
    mesh_types::default_seed_rpc_url()
}
fn default_connect() -> Vec<String> {
    mesh_types::default_seed_connects()
}
fn default_chain() -> String {
    "data/chain.bin".into()
}
fn default_wallet() -> String {
    "data/wallet.key".into()
}
fn default_p2p() -> String {
    "data/p2p.key".into()
}
fn default_miner_key() -> String {
    "data/wallet.key".into()
}
fn default_bottom_h() -> f32 {
    BOTTOM_H_DEFAULT
}

fn load_or_mint_cookie(path: &Path) -> String {
    if let Ok(s) = fs::read_to_string(path) {
        let t = s.trim().to_string();
        if !t.is_empty() {
            let _ = mesh_crypto::restrict_secret_file(path);
            return t;
        }
    }
    let token = mesh_crypto::mint_secret_hex();
    let _ = mesh_crypto::write_secret_file(path, token.as_bytes());
    token
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            rpc: default_rpc(),
            connect: default_connect(),
            mine: false,
            mine_blocks: 0,
            chain: default_chain(),
            wallet: default_wallet(),
            p2p_key: default_p2p(),
            miner_key: default_miner_key(),
            operator_address: String::new(),
            operator_vault: String::new(),
            orch: default_orch(),
            bottom_h: default_bottom_h(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct NodeInfo {
    height: u64,
    tip: String,
    #[serde(default)]
    peers: usize,
    #[serde(default)]
    address: String,
    #[serde(default)]
    operator_address: String,
    #[serde(default)]
    median_peer_rtt_ms: Option<u64>,
    #[serde(default)]
    relay_rtt_factor_milli: u64,
    #[serde(default)]
    ai_shard_id: u32,
    #[serde(default)]
    ai_shard_count: u32,
    #[serde(default)]
    finalized_height: u64,
    #[serde(default)]
    #[allow(dead_code)]
    finalized_hash: String,
    #[serde(default)]
    finality_active: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[allow(dead_code)]
struct NodeRewardsInfo {
    #[serde(default)]
    address: String,
    #[serde(default)]
    balance: String,
    #[serde(default)]
    balance_atomic: u64,
    #[serde(default)]
    pending_weight: u64,
    #[serde(default)]
    pending_total_weight: u64,
    #[serde(default)]
    estimated_share: String,
    #[serde(default)]
    peers: usize,
    #[serde(default)]
    bonded: bool,
    #[serde(default)]
    bond_eligible: bool,
    #[serde(default)]
    bond_locked_atomic: u64,
    #[serde(default)]
    bond_slashed: bool,
    #[serde(default)]
    reputation_milli: u64,
    #[serde(default)]
    relay_rtt_factor_milli: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MainTab {
    Overview,
    Earnings,
    Network,
    Settings,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct MarketsInfo {
    #[serde(default)]
    cpu_market: String,
    #[serde(default)]
    gpu_market: String,
    #[serde(default)]
    node_market: String,
    #[serde(default)]
    #[allow(dead_code)]
    gpu_exam_market: String,
    #[serde(default)]
    #[allow(dead_code)]
    gpu_fusion_market: String,
    #[serde(default)]
    #[allow(dead_code)]
    helper_floor: bool,
    #[serde(default)]
    #[allow(dead_code)]
    fair_split: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[allow(dead_code)]
struct MeshPulseInfo {
    #[serde(default)]
    gpu_vs_height_signal: f64,
    #[serde(default)]
    markets: PulseMarkets,
    #[serde(default)]
    trilemma: Option<TrilemmaInfo>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[allow(dead_code)]
struct TrilemmaInfo {
    #[serde(default)]
    sec: u8,
    #[serde(default)]
    scale: u8,
    #[serde(default)]
    decent: u8,
    #[serde(default)]
    transpar: u8,
    #[serde(default)]
    balance: u8,
    #[serde(default)]
    weakest: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[allow(dead_code)]
struct PulseMarkets {
    #[serde(default)]
    pending_gpu_weight: u64,
    #[serde(default)]
    pending_node_weight: u64,
    #[serde(default)]
    gpu_receipts: usize,
    #[serde(default)]
    avg_latency_ms: f64,
    #[serde(default)]
    research_eval_receipts: usize,
    #[serde(default)]
    research_progress: f64,
    #[serde(default)]
    research_scores: PulseResearchScores,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[allow(dead_code)]
struct PulseResearchScores {
    #[serde(default)]
    mean_primary: f64,
    #[serde(default)]
    mean_orphan_risk: f64,
    #[serde(default)]
    mean_detect_rate: f64,
    #[serde(default)]
    mean_linkability: f64,
    #[serde(default)]
    mean_backlog_ratio: f64,
    #[serde(default)]
    scenarios_touched: u32,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct SoftEnvelopes {
    #[serde(default)]
    soft_adapt_signal_threshold: f64,
    #[serde(default)]
    soft_benchmark_rounds: u32,
    #[serde(default)]
    min_verifier_weight: u64,
    #[serde(default)]
    suggested_cpu_diff_bias: i32,
    #[serde(default)]
    idle_stipend_bps_cap: u16,
    #[serde(default)]
    brain_prefer_v2: u8,
    #[serde(default)]
    brain_v2_min_workers: u32,
    #[serde(default)]
    brain_v2_vram_floor_mb: u32,
    #[serde(default)]
    leg_train_enable: u8,
    #[serde(default)]
    leg_parallel: u32,
    #[serde(default)]
    leg_harden_sec_floor: u8,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[allow(dead_code)]
struct ProposalRow {
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    created_at_height: u64,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[allow(dead_code)]
struct LatestEpochInfo {
    #[serde(default)]
    epoch: u64,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    eval_count: u64,
    #[serde(default)]
    proposal_id: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct ProposalsInfo {
    #[serde(default)]
    proposals: Vec<ProposalRow>,
    #[serde(default)]
    local_node_id: Option<String>,
    #[serde(default)]
    active_envelopes: SoftEnvelopes,
    #[serde(default)]
    last_auto_adapt_proposal_id: String,
    #[serde(default)]
    last_auto_adapt_at_height: u64,
    #[serde(default)]
    last_auto_adapt_eval_count: u64,
    #[serde(default)]
    param_epoch: u64,
    #[serde(default)]
    latest_epoch: Option<LatestEpochInfo>,
    #[serde(default)]
    epoch_history: Vec<LatestEpochInfo>,
    #[serde(default)]
    consensus_difficulty: u32,
    #[serde(default)]
    soft_diff_hint: u32,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct MiningFeedEvent {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    kind: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct MiningStatusInfo {
    #[serde(default)]
    events: Vec<MiningFeedEvent>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct PaySnapshot {
    #[serde(default)]
    rewards: String,
    #[serde(default)]
    by_lane: Vec<PayLane>,
    #[serde(default)]
    recent: Vec<PayHit>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct PayLane {
    #[serde(default)]
    title: String,
    #[serde(default)]
    paid_for: String,
    #[serde(default)]
    amount: String,
    #[serde(default)]
    count: u64,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct PayHit {
    #[serde(default)]
    height: u64,
    #[serde(default)]
    timestamp: u64,
    #[serde(default)]
    amount: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    mature: bool,
    #[serde(default)]
    confirmations: u64,
}

enum UiEvent {
    Log(String, LogKind),
    NodeInfo(NodeInfo),
    Markets(MarketsInfo),
    Pulse(MeshPulseInfo),
    Proposals(ProposalsInfo),
    Mining(MiningStatusInfo),
    Rewards(NodeRewardsInfo),
    Pay(PaySnapshot),
    PublicTip { height: u64, peers: usize },
    RpcOffline,
}

#[derive(Clone, Copy)]
enum LogKind {
    Info,
    Ok,
    Warn,
    Err,
}

struct LogLine {
    at: String,
    msg: String,
    kind: LogKind,
}

fn log_stamp() -> String {
    chrono::Local::now().format("%d %b %H:%M:%S").to_string()
}

fn format_unix_local(ts: u64) -> Option<String> {
    if ts == 0 {
        return None;
    }
    chrono::DateTime::from_timestamp(ts as i64, 0).map(|t| {
        t.with_timezone(&chrono::Local)
            .format("%d %b %H:%M:%S")
            .to_string()
    })
}

fn paint_log_line(ui: &mut egui::Ui, line: &LogLine) {
    let col = match line.kind {
        LogKind::Info => TEXT_BLUE,
        LogKind::Ok => OK,
        LogKind::Warn => WARN,
        LogKind::Err => DANGER,
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(body_text(&line.at, 12.0).color(MUTED).monospace());
        ui.label(body_text(&line.msg, 13.0).color(col));
    });
}

struct NodeApp {
    root: PathBuf,
    cfg_path: PathBuf,
    cfg: NodeConfig,
    peer_edit: String,
    status: String,
    status_ok: bool,
    running: bool,
    tab: MainTab,
    height: Option<u64>,
    tip_short: String,
    finalized_height: u64,
    finality_active: bool,
    peers: usize,
    median_peer_rtt_ms: Option<u64>,
    network_rtt_factor_milli: u64,
    ai_shard_id: u32,
    ai_shard_count: u32,
    wallet_address: String,
    markets: MarketsInfo,
    pulse: MeshPulseInfo,
    proposals: ProposalsInfo,
    rewards: NodeRewardsInfo,
    pay: PaySnapshot,
    public_height: Option<u64>,
    send_to: String,
    send_amount: String,
    send_status: String,
    backup_ack: bool,
    last_mining_event_id: u64,
    child: Option<Child>,
    child_pid: Option<u32>,
    log: Vec<LogLine>,
    event_tx: Sender<UiEvent>,
    event_rx: Receiver<UiEvent>,
    last_poll: Instant,
    scene: Option<TextureHandle>,
    mascot: Option<TextureHandle>,
    coin: Option<TextureHandle>,
}

fn main() -> eframe::Result<()> {
    let root = resolve_root();
    let cfg_path = root.join("config.json");
    let cfg = load_config(&cfg_path);
    let peer_edit = if cfg.connect.is_empty() {
        String::new()
    } else {
        cfg.connect.join(", ")
    };
    let (event_tx, event_rx) = mpsc::channel();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WIN_W, WIN_H])
            .with_min_inner_size([800.0, 600.0])
            .with_resizable(true)
            .with_title("MonkeyMesh Node"),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "MonkeyMesh Node",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = NodeApp {
                root: root.clone(),
                cfg_path,
                cfg,
                peer_edit,
                status: "Ready".into(),
                status_ok: true,
                running: false,
                tab: MainTab::Overview,
                height: None,
                tip_short: "—".into(),
                finalized_height: 0,
                finality_active: false,
                peers: 0,
                median_peer_rtt_ms: None,
                network_rtt_factor_milli: 1_000,
                ai_shard_id: 0,
                ai_shard_count: 1,
                wallet_address: String::new(),
                markets: MarketsInfo::default(),
                pulse: MeshPulseInfo::default(),
                proposals: ProposalsInfo::default(),
                rewards: NodeRewardsInfo::default(),
                pay: PaySnapshot::default(),
                public_height: None,
                send_to: String::new(),
                send_amount: String::new(),
                send_status: String::new(),
                backup_ack: false,
                last_mining_event_id: 0,
                child: None,
                child_pid: None,
                log: vec![],
                event_tx,
                event_rx,
                last_poll: Instant::now() - Duration::from_secs(5),
                scene: None,
                mascot: None,
                coin: None,
            };
            app.push_log("Set listen / RPC, then press Start.", LogKind::Info);
            app.scene = load_texture(&cc.egui_ctx, "coin_scene", SCENE_PNG, 1400);
            app.mascot = load_texture(&cc.egui_ctx, "mascot", MASCOT_PNG, 256);
            app.coin = load_texture(&cc.egui_ctx, "coin_mark", COIN_PNG, 192);
            Ok(Box::new(app))
        }),
    )
}

fn resolve_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Prefer Launchers/Node when GUI sits in that folder.
            if dir.join("config.json").exists() || dir.join("bin").exists() {
                return dir.to_path_buf();
            }
            // Fallback: repo Launchers/Node next to target
            let candidate = dir
                .join("..")
                .join("..")
                .join("Launchers")
                .join("Node");
            if candidate.join("config.json").exists() {
                return candidate;
            }
            return dir.to_path_buf();
        }
    }
    PathBuf::from(".")
}

fn load_config(path: &Path) -> NodeConfig {
    let Ok(raw) = fs::read_to_string(path) else {
        return NodeConfig::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_config(path: &Path, cfg: &NodeConfig) {
    if let Ok(raw) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(path, raw);
    }
}

fn find_node_exe(root: &Path) -> Option<PathBuf> {
    let mut candidates = vec![
        root.join("bin").join("mesh-node.exe"),
        root.join("mesh-node.exe"),
        root.join("bin").join("mesh-node"),
        PathBuf::from("target/release/mesh-node.exe"),
        PathBuf::from("target/debug/mesh-node.exe"),
    ];
    // Dev: workspace target next to Launchers/Node
    candidates.push(root.join("..").join("..").join("target").join("release").join("mesh-node.exe"));
    candidates.push(root.join("..").join("..").join("target").join("debug").join("mesh-node.exe"));
    candidates.into_iter().find(|p| p.exists())
}

/// Bind address `0.0.0.0` is not reachable as a client target — use loopback.
fn listen_rpc_for_client(rpc: &str) -> String {
    rpc.trim().replace("0.0.0.0", "127.0.0.1")
}

fn rpc_base(rpc: &str) -> String {
    let r = listen_rpc_for_client(rpc);
    let r = r.trim_end_matches('/');
    if r.starts_with("http://") || r.starts_with("https://") {
        r.to_string()
    } else {
        format!("http://{r}")
    }
}

fn rpc_is_loopback_only(rpc: &str) -> bool {
    let r = rpc.trim().to_ascii_lowercase();
    let host = r
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    host.starts_with("127.0.0.1") || host.starts_with("localhost")
}

fn short_hash(s: &str) -> String {
    if s.len() <= 12 {
        s.to_string()
    } else {
        format!("{}…", &s[..10])
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

impl NodeApp {
    fn push_log(&mut self, msg: impl Into<String>, kind: LogKind) {
        self.log.push(LogLine {
            at: log_stamp(),
            msg: msg.into(),
            kind,
        });
        if self.log.len() > LOG_CAP {
            let n = self.log.len() - LOG_CAP;
            self.log.drain(0..n);
        }
    }

    fn tip_synced(&self) -> bool {
        let local = self.height.unwrap_or(0);
        match self.public_height {
            Some(pub_h) => pub_h <= local.saturating_add(3),
            None => local > 0,
        }
    }

    fn auto_mine_ready(&self) -> bool {
        self.tip_synced() && self.peers > 0
    }

    fn apply_peer_field(&mut self) {
        let p = self.peer_edit.trim().to_string();
        if p.is_empty() {
            self.cfg.connect = default_connect();
        } else {
            self.cfg.connect = p
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    fn save_settings(&mut self) {
        self.apply_peer_field();
        self.cfg.listen = self.cfg.listen.trim().to_string();
        self.cfg.rpc = self.cfg.rpc.trim().to_string();
        save_config(&self.cfg_path, &self.cfg);
        self.status = "Saved".into();
        self.status_ok = true;
        self.push_log("Settings saved", LogKind::Ok);
    }

    fn rpc_token(&self) -> Option<String> {
        std::env::var("MESH_RPC_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let p = self.root.join("data").join("rpc.token");
                fs::read_to_string(p)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
    }

    fn apply_reward_wallet(&mut self) {
        let raw = self.cfg.operator_address.trim().to_string();
        if raw.is_empty() {
            self.status = "Enter a mesh01… reward wallet".into();
            self.status_ok = false;
            return;
        }
        if mesh_types::Address::from_hex(&raw).is_none() {
            self.status = "That is not a valid MESH address".into();
            self.status_ok = false;
            self.push_log("Reward wallet rejected — expected mesh01…", LogKind::Err);
            return;
        }
        self.cfg.operator_address = raw.clone();
        save_config(&self.cfg_path, &self.cfg);
        if !self.running {
            self.status = "Reward wallet saved — start the node to earn".into();
            self.status_ok = true;
            self.push_log(format!("Reward wallet saved: {raw}"), LogKind::Ok);
            return;
        }
        let Some(token) = self.rpc_token() else {
            self.status = "Saved locally — restart the node to apply".into();
            self.status_ok = true;
            self.push_log(
                "Reward wallet saved; no RPC token — restart the node",
                LogKind::Warn,
            );
            return;
        };
        let base = rpc_base(&self.cfg.rpc);
        let tx = self.event_tx.clone();
        self.status = "Applying reward wallet…".into();
        thread::spawn(move || {
            let body = serde_json::json!({ "address": raw });
            let mut req = ureq::post(&format!("{base}/v1/setoperator"))
                .timeout(Duration::from_secs(10));
            req = req.set("X-Mesh-Token", &token);
            match req.send_json(body) {
                Ok(resp) => {
                    let ok = resp
                        .into_json::<serde_json::Value>()
                        .ok()
                        .and_then(|v| v.get("address").and_then(|a| a.as_str()).map(|s| s.to_string()));
                    let _ = tx.send(UiEvent::Log(
                        format!(
                            "Reward wallet live: {}",
                            ok.unwrap_or_else(|| "applied".into())
                        ),
                        LogKind::Ok,
                    ));
                }
                Err(e) => {
                    let _ = tx.send(UiEvent::Log(
                        format!("Reward wallet saved; apply failed ({e}) — restart the node"),
                        LogKind::Warn,
                    ));
                }
            }
        });
    }

    fn start_node(&mut self) {
        if self.running {
            return;
        }
        self.apply_peer_field();
        save_config(&self.cfg_path, &self.cfg);

        let Some(exe) = find_node_exe(&self.root) else {
            self.status = "mesh-node.exe not found".into();
            self.status_ok = false;
            self.push_log("Could not find mesh-node.exe — build the node first.", LogKind::Err);
            return;
        };

        let _ = fs::create_dir_all(self.root.join("data"));
        let chain = self.root.join(&self.cfg.chain);
        let wallet = self.root.join(&self.cfg.wallet);
        let p2p = self.root.join(&self.cfg.p2p_key);
        let miner = self.root.join(&self.cfg.miner_key);

        let mut cmd = Command::new(&exe);
        cmd.current_dir(&self.root)
            .arg("--chain")
            .arg(&chain)
            .arg("serve")
            .arg("--listen")
            .arg(&self.cfg.listen)
            .arg("--rpc")
            .arg(&self.cfg.rpc)
            .arg("--wallet")
            .arg(&wallet)
            .arg("--p2p-key")
            .arg(&p2p)
            .arg("--miner-key")
            .arg(&miner)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if std::env::var("MESH_FORCE_RETARGET_INTERVAL").is_err() {
            // Live public testnet retargets every 15 (seed heal). Default 20 rejects block 150.
            cmd.env("MESH_FORCE_RETARGET_INTERVAL", "15");
        }

        for peer in &self.cfg.connect {
            if !peer.trim().is_empty() {
                cmd.arg("--connect").arg(peer.trim());
            }
        }
        if self.cfg.mine {
            cmd.arg("--mine");
            if self.cfg.mine_blocks > 0 {
                cmd.arg("--mine-blocks")
                    .arg(self.cfg.mine_blocks.to_string());
            }
        }

        let op_addr = self.cfg.operator_address.trim();
        if !op_addr.is_empty() {
            cmd.arg("--operator-address").arg(op_addr);
            cmd.env("MESH_OPERATOR_ADDRESS", op_addr);
        }
        let op_vault = self.cfg.operator_vault.trim();
        if !op_vault.is_empty() {
            let vault_path = self.root.join(op_vault);
            cmd.env(
                "MESH_OPERATOR_VAULT",
                vault_path.to_string_lossy().as_ref(),
            );
        }

        // Sticky cookies: data/ai.token + data/rpc.token (OS CSPRNG, owner-only).
        let data = self.root.join("data");
        let _ = fs::create_dir_all(&data);
        let auto_ai = std::env::var("MESH_AI_TOKEN_AUTO").unwrap_or_else(|_| "1".into());
        let auto_on = auto_ai == "1" || auto_ai.eq_ignore_ascii_case("true");
        let env_tok = std::env::var("MESH_AI_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(t) = env_tok {
            cmd.env("MESH_AI_TOKEN", t);
        } else if auto_on {
            cmd.env("MESH_AI_TOKEN", load_or_mint_cookie(&data.join("ai.token")));
        }
        let rpc_tok = std::env::var("MESH_RPC_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| load_or_mint_cookie(&data.join("rpc.token")));
        cmd.env("MESH_RPC_TOKEN", rpc_tok);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        match cmd.spawn() {
            Ok(mut child) => {
                let pid = child.id();
                self.child_pid = Some(pid);
                let tx = self.event_tx.clone();
                if let Some(out) = child.stdout.take() {
                    let txo = tx.clone();
                    thread::spawn(move || pipe_child_output(out, txo));
                }
                if let Some(err) = child.stderr.take() {
                    let txe = tx.clone();
                    thread::spawn(move || pipe_child_output(err, txe));
                }
                let tx_wait = self.event_tx.clone();
                // Wait for exit on a side thread using a duplicated wait via pid polling
                // after we store Child for kill. We'll poll try_wait in drain.
                self.child = Some(child);
                self.running = true;
                self.status = "Starting…".into();
                self.status_ok = true;
                self.push_log("Node started", LogKind::Ok);
                if self.cfg.mine {
                    self.push_log(
                        "Auto-mine armed — waits until this node is within 3 blocks of the seed and has a P2P peer.",
                        LogKind::Warn,
                    );
                } else {
                    self.push_log(
                        "Joining seed peers — height should climb off 0. Mining stays off (use MonkeyMesh Miner).",
                        LogKind::Info,
                    );
                }
                let _ = tx_wait; // keep pattern open for future
            }
            Err(e) => {
                self.status = "Failed to start".into();
                self.status_ok = false;
                self.push_log(format!("Could not start node: {e}"), LogKind::Err);
            }
        }
    }

    fn stop_node(&mut self) {
        if let Some(pid) = self.child_pid.take() {
            kill_process_tree(pid);
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.running = false;
        self.height = None;
        self.tip_short = "—".into();
        self.last_mining_event_id = 0;
        self.status = "Stopped".into();
        self.status_ok = true;
        self.push_log("Node stopped", LogKind::Warn);
    }

    fn poll_rpc(&mut self) {
        if !self.running {
            return;
        }
        if self.last_poll.elapsed() < Duration::from_millis(900) {
            return;
        }
        self.last_poll = Instant::now();
        let base = rpc_base(&self.cfg.rpc);
        let tx = self.event_tx.clone();
        let url = format!("{base}/v1/getnodeinfo");
        let url_m = format!("{base}/v1/markets");
        let url_p = format!("{base}/v1/meshpulse");
        let url_pr = format!("{base}/v1/proposals");
        let op = self.cfg.operator_address.trim().to_string();
        let url_rw = if op.is_empty() {
            format!("{base}/v1/noderewards")
        } else {
            format!("{base}/v1/noderewards?address={op}")
        };
        let after = self.last_mining_event_id;
        let url_mine = format!("{base}/v1/miningstatus?after={after}");
        let pay_addr = if op.is_empty() {
            String::new()
        } else {
            op.clone()
        };
        let url_pay = if pay_addr.is_empty() {
            format!("{base}/v1/getrewards")
        } else {
            format!("{base}/v1/getrewards?address={pay_addr}")
        };
        let seed_urls = vec![mesh_types::default_seed_rpc_url()];
        thread::spawn(move || {
            // Local/public seed RPC often takes 2–6s under load — keep UI from
            // flapping to "Starting…" when the node is actually healthy.
            const RPC_TIMEOUT: Duration = Duration::from_secs(12);
            match ureq::get(&url).timeout(RPC_TIMEOUT).call() {
                Ok(resp) => match resp.into_json::<NodeInfo>() {
                    Ok(info) => {
                        let _ = tx.send(UiEvent::NodeInfo(info));
                    }
                    Err(e) => {
                        let _ = tx.send(UiEvent::Log(
                            format!("RPC parse failed: {e}"),
                            LogKind::Warn,
                        ));
                        let _ = tx.send(UiEvent::RpcOffline);
                    }
                },
                Err(_) => {
                    let _ = tx.send(UiEvent::RpcOffline);
                }
            }
            if let Ok(resp) = ureq::get(&url_m).timeout(RPC_TIMEOUT).call() {
                if let Ok(m) = resp.into_json::<MarketsInfo>() {
                    let _ = tx.send(UiEvent::Markets(m));
                }
            }
            if let Ok(resp) = ureq::get(&url_p).timeout(RPC_TIMEOUT).call() {
                if let Ok(p) = resp.into_json::<MeshPulseInfo>() {
                    let _ = tx.send(UiEvent::Pulse(p));
                }
            }
            if let Ok(resp) = ureq::get(&url_pr).timeout(RPC_TIMEOUT).call() {
                if let Ok(p) = resp.into_json::<ProposalsInfo>() {
                    let _ = tx.send(UiEvent::Proposals(p));
                }
            }
            if let Ok(resp) = ureq::get(&url_rw).timeout(RPC_TIMEOUT).call() {
                if let Ok(r) = resp.into_json::<NodeRewardsInfo>() {
                    let _ = tx.send(UiEvent::Rewards(r));
                }
            }
            if let Ok(resp) = ureq::get(&url_mine).timeout(RPC_TIMEOUT).call() {
                if let Ok(m) = resp.into_json::<MiningStatusInfo>() {
                    let _ = tx.send(UiEvent::Mining(m));
                }
            }
            if let Ok(resp) = ureq::get(&url_pay).timeout(RPC_TIMEOUT).call() {
                if let Ok(p) = resp.into_json::<PaySnapshot>() {
                    let _ = tx.send(UiEvent::Pay(p));
                }
            }
            for seed in &seed_urls {
                let url = format!("{}/v1/getnodeinfo", seed.trim_end_matches('/'));
                if let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(4)).call() {
                    if let Ok(info) = resp.into_json::<NodeInfo>() {
                        let _ = tx.send(UiEvent::PublicTip {
                            height: info.height,
                            peers: info.peers,
                        });
                        break;
                    }
                }
            }
        });
    }

    fn send_to_address(&mut self) {
        let to = self.send_to.trim().to_string();
        let amount = self.send_amount.trim().to_string();
        if to.is_empty() || amount.is_empty() {
            self.send_status = "Enter address and amount".into();
            return;
        }
        if !self.running {
            self.send_status = "Node is offline".into();
            return;
        }
        let base = rpc_base(&self.cfg.rpc);
        let token = self.rpc_token();
        let tx = self.event_tx.clone();
        self.send_status = "Sending…".into();
        self.push_log(format!("Sending {amount} → {to}"), LogKind::Info);
        thread::spawn(move || {
            let body = serde_json::json!({ "address": to, "amount": amount });
            let mut req = ureq::post(&format!("{base}/v1/sendtoaddress"))
                .timeout(Duration::from_secs(15));
            if let Some(t) = token.as_deref() {
                req = req.set("X-Mesh-Token", t);
            }
            match req.send_json(body)
            {
                Ok(resp) => {
                    let status = resp.status();
                    if (200..300).contains(&status) {
                        match resp.into_json::<serde_json::Value>() {
                            Ok(v) => {
                                let txid = v
                                    .get("txid")
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("ok");
                                let _ = tx.send(UiEvent::Log(
                                    format!("Sent · {txid}"),
                                    LogKind::Ok,
                                ));
                            }
                            Err(e) => {
                                let _ = tx.send(UiEvent::Log(
                                    format!("Send ok but parse failed: {e}"),
                                    LogKind::Warn,
                                ));
                            }
                        }
                    } else {
                        let text = resp.into_string().unwrap_or_default();
                        let _ = tx.send(UiEvent::Log(
                            format!("Send failed ({status}): {}", truncate(&text, 120)),
                            LogKind::Err,
                        ));
                    }
                }
                Err(ureq::Error::Status(code, resp)) => {
                    let text = resp.into_string().unwrap_or_default();
                    let _ = tx.send(UiEvent::Log(
                        format!("Send failed ({code}): {}", truncate(&text, 120)),
                        LogKind::Err,
                    ));
                }
                Err(e) => {
                    let _ = tx.send(UiEvent::Log(format!("Send error: {e}"), LogKind::Err));
                }
            }
        });
    }

    fn drain(&mut self) {
        // Detect child exit
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let code = status.code().unwrap_or(-1);
                    self.child = None;
                    self.child_pid = None;
                    self.running = false;
                    self.push_log(format!("Node exited ({code})"), LogKind::Warn);
                    self.status = "Stopped".into();
                    self.status_ok = true;
                }
                Ok(None) => {}
                Err(_) => {}
            }
        }

        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                UiEvent::Log(msg, kind) => {
                    if msg.starts_with("Sent")
                        || msg.starts_with("Send failed")
                        || msg.starts_with("Send error")
                        || msg.starts_with("Send ok")
                    {
                        self.send_status = msg.clone();
                    }
                    self.push_log(msg, kind);
                }
                UiEvent::NodeInfo(info) => {
                    let prev = self.height;
                    self.height = Some(info.height);
                    self.tip_short = short_hash(&info.tip);
                    self.finalized_height = info.finalized_height;
                    self.finality_active = info.finality_active;
                    self.peers = info.peers;
                    self.median_peer_rtt_ms = info.median_peer_rtt_ms;
                    if info.relay_rtt_factor_milli > 0 {
                        self.network_rtt_factor_milli = info.relay_rtt_factor_milli;
                    }
                    self.ai_shard_id = info.ai_shard_id;
                    self.ai_shard_count = info.ai_shard_count.max(1);
                    if !info.address.is_empty() {
                        self.wallet_address = info.address;
                    }
                    if !info.operator_address.is_empty() && self.cfg.operator_address.trim().is_empty()
                    {
                        self.cfg.operator_address = info.operator_address;
                    }
                    self.status = "Online".into();
                    self.status_ok = true;
                    if prev != Some(info.height) {
                        self.push_log(format!("Height {}", info.height), LogKind::Ok);
                    }
                }
                UiEvent::Markets(m) => {
                    self.markets = m;
                }
                UiEvent::Pulse(p) => {
                    self.pulse = p;
                }
                UiEvent::Proposals(p) => {
                    self.proposals = p;
                }
                UiEvent::Rewards(r) => {
                    if r.peers > 0 {
                        self.peers = r.peers;
                    }
                    if !r.address.is_empty() {
                        self.wallet_address = r.address.clone();
                    }
                    self.rewards = r;
                }
                UiEvent::Pay(p) => {
                    self.pay = p;
                }
                UiEvent::PublicTip { height, peers: _ } => {
                    let prev = self.public_height;
                    self.public_height = Some(height);
                    if prev != Some(height) {
                        self.push_log(
                            format!("Public testnet height {height}"),
                            LogKind::Info,
                        );
                    }
                }
                UiEvent::Mining(m) => {
                    for ev in m.events {
                        if ev.id <= self.last_mining_event_id {
                            continue;
                        }
                        self.last_mining_event_id = self.last_mining_event_id.max(ev.id);
                        let kind = match ev.kind.as_str() {
                            "ok" => LogKind::Ok,
                            "warn" => LogKind::Warn,
                            "err" | "error" => LogKind::Err,
                            _ => LogKind::Info,
                        };
                        self.push_log(ev.msg, kind);
                    }
                }
                UiEvent::RpcOffline => {
                    if self.running {
                        // Keep last known Online briefly — RPC can be slow without being down.
                        if self.height.is_some() && self.status == "Online" {
                            // leave status as Online; only drop after sustained failure
                        } else {
                            self.status = "Starting…".into();
                            self.status_ok = true;
                        }
                    }
                }
            }
        }
    }
}

fn pipe_child_output<R: std::io::Read + Send + 'static>(reader: R, tx: Sender<UiEvent>) {
    let buf = BufReader::new(reader);
    for line in buf.lines().flatten() {
        if let Some(msg) = friendly_node_line(&line) {
            let _ = tx.send(UiEvent::Log(msg, LogKind::Info));
        }
    }
}

fn friendly_node_line(line: &str) -> Option<String> {
    let low = line.to_ascii_lowercase();
    // Quiet UX: ignore normal WARN/INFO that happen to include an `error=` field
    // (e.g. reject-block / dial noise). Only surface real ERROR / panic lines.
    if low.contains("listening") || (low.contains("rpc") && low.contains("bound")) {
        return Some("RPC is up".into());
    }
    if low.contains("mining enabled") || low.contains("auto-mine armed") {
        return Some("Auto-mine armed — waits for sync + a peer".into());
    }
    if let Some(i) = low.find("auto-mine waiting") {
        return Some(line[i..].trim().to_string());
    }
    if let Some(i) = low.find("auto-mine live") {
        return Some(line[i..].trim().to_string());
    }
    if let Some(i) = low.find("syncing from seed") {
        return Some(line[i..].trim().to_string());
    }
    if let Some(i) = low.find("caught up with seed") {
        return Some(line[i..].trim().to_string());
    }
    if let Some(i) = low.find("sync rejected block") {
        return Some(line[i..].trim().to_string());
    }
    if let Some(i) = low.find("aligned retarget") {
        return Some(line[i..].trim().to_string());
    }
    if low.contains("http catch-up from seed") {
        return Some("HTTP catch-up from seed armed".into());
    }
    if low.contains("panic") {
        return Some("Node panicked".into());
    }
    // Match tracing level token, not substring in field names like `error=…`.
    if is_tracing_error_level(&low) {
        if low.contains("mine accept") {
            return Some("Could not accept mined block".into());
        }
        if low.contains("rpc server exited") {
            return Some("RPC stopped unexpectedly".into());
        }
        return Some("Node error (see console logs)".into());
    }
    None
}

/// True only for tracing ERROR / FATAL level tokens near the start of the line.
/// Do not treat `error=…` fields on WARN lines as a node crash.
fn is_tracing_error_level(low: &str) -> bool {
    low.split_whitespace().take(5).any(|tok| {
        matches!(
            tok.trim_matches(|c: char| !c.is_ascii_alphabetic()),
            "error" | "err" | "fatal"
        )
    })
}

fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
}

impl eframe::App for NodeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        self.poll_rpc();
        ctx.request_repaint_after(Duration::from_millis(if self.running { 200 } else { 400 }));
        let time = ctx.input(|i| i.time);

        let editable = !self.running;
        let mut bottom_h = self.cfg.bottom_h.clamp(BOTTOM_H_MIN, BOTTOM_H_MAX);
        let mut save_h = false;

        egui::TopBottomPanel::bottom("node_bottom")
            .exact_height(bottom_h)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(12, 9, 7, 248))
                    .inner_margin(egui::Margin::symmetric(18, 10))
                    .stroke(egui::Stroke::new(1.0, RULE)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                let handle = ui.allocate_response(
                    egui::vec2(ui.available_width(), 12.0),
                    egui::Sense::drag(),
                );
                {
                    let r = handle.rect;
                    let painter = ui.painter();
                    paint_resize_grip(painter, r);
                }
                if handle.dragged() {
                    bottom_h =
                        (bottom_h - handle.drag_delta().y).clamp(BOTTOM_H_MIN, BOTTOM_H_MAX);
                }
                if handle.drag_stopped() {
                    save_h = true;
                }
                handle.on_hover_cursor(egui::CursorIcon::ResizeVertical);

                ui.horizontal(|ui| {
                    if self.running {
                        if danger_btn(ui, "Stop").clicked() {
                            self.stop_node();
                        }
                    } else if primary_btn(ui, "Start node", true).clicked() {
                        self.start_node();
                    }
                    ui.add_space(10.0);
                    if ghost_btn(ui, "Explorer").clicked() {
                        let _ = open::that(format!("{}/", rpc_base(&self.cfg.rpc)));
                    }
                    ui.add_space(12.0);
                    ui.label(
                        body_text(&self.status, 14.0)
                            .color(if self.status_ok { TEXT_BLUE } else { DANGER }),
                    );
                });
                ui.add_space(6.0);
                field_label(ui, "Recent");
                let log_h = ui.available_height().max(36.0);
                egui::Frame::new()
                    .fill(FIELD_BG)
                    .corner_radius(leaf_radius())
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ScrollArea::vertical()
                            .id_salt("node_log")
                            .max_height(log_h)
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                if self.log.is_empty() {
                                    ui.label(body_text("Nothing yet.", 13.0).color(MUTED));
                                }
                                for line in &self.log {
                                    paint_log_line(ui, line);
                                }
                            });
                    });
            });
        self.cfg.bottom_h = bottom_h;
        if save_h {
            save_config(&self.cfg_path, &self.cfg);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::ZERO))
            .show(ctx, |ui| {
                paint_brand_backdrop(ui, self.scene.as_ref(), time);
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(tex) = &self.mascot {
                                ui.add(
                                    egui::Image::new(tex)
                                        .fit_to_exact_size(egui::vec2(52.0, 52.0))
                                        .maintain_aspect_ratio(true),
                                );
                                ui.add_space(12.0);
                            }
                            ui.vertical(|ui| {
                                ui.label(display_text("MonkeyMesh", 20.0).color(INK).strong());
                                ui.label(display_text("Node", 13.0).color(CYAN));
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if let Some(tex) = &self.coin {
                                        ui.add(
                                            egui::Image::new(tex)
                                                .fit_to_exact_size(egui::vec2(40.0, 40.0))
                                                .maintain_aspect_ratio(true),
                                        );
                                    }
                                },
                            );
                        });

                        ui.add_space(10.0);
                        dual_rail(ui);
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            tab_btn(ui, &mut self.tab, MainTab::Overview, "Overview");
                            tab_btn(ui, &mut self.tab, MainTab::Earnings, "Earnings");
                            tab_btn(ui, &mut self.tab, MainTab::Network, "Network");
                            tab_btn(ui, &mut self.tab, MainTab::Settings, "Settings");
                        });

                        ui.add_space(10.0);

                        ScrollArea::vertical()
                            .id_salt("main")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let tab = self.tab;
                                match tab {
                                    MainTab::Overview => self.ui_overview(ui),
                                    MainTab::Earnings => self.ui_earnings(ui),
                                    MainTab::Network => self.ui_network(ui, editable),
                                    MainTab::Settings => self.ui_settings(ui, editable),
                                }
                            });
                    });
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_node();
    }
}

impl NodeApp {
    fn ui_overview(&self, ui: &mut egui::Ui) {
        let height = self
            .height
            .map(|h| h.to_string())
            .unwrap_or_else(|| "—".into());
        let online = if self.running && self.status == "Online" {
            "Online"
        } else if self.running {
            "Starting"
        } else {
            "Offline"
        };
        let finality = if self.finality_active {
            format!("#{}", self.finalized_height)
        } else {
            "off".into()
        };
        ui.columns(5, |cols| {
            stat_col(&mut cols[0], "Height", &height, CYAN);
            stat_col(
                &mut cols[1],
                "Status",
                online,
                if online == "Online" { OK } else { MUTED },
            );
            stat_col(&mut cols[2], "Tip", &self.tip_short, TEXT_BLUE);
            stat_col(
                &mut cols[3],
                "Peers",
                &self.peers.to_string(),
                if self.peers > 0 { OK } else { MUTED },
            );
            stat_col(&mut cols[4], "Finality", &finality, MUTED);
        });

        if self.running {
            ui.add_space(10.0);
            let local_h = self.height.unwrap_or(0);
            if let Some(pub_h) = self.public_height {
                if pub_h > local_h {
                    ui.label(
                        body_text(
                            format!(
                                "Syncing from seednode.hashmonkeys.cloud — height {local_h} → {pub_h}. Local data is your copy of the public chain, not a private network."
                            ),
                            12.5,
                        )
                        .color(CYAN),
                    );
                } else if self.peers == 0 {
                    ui.label(
                        body_text(
                            "Synced with the seed. Open UDP 39001 in Windows Firewall if you want live P2P peers.",
                            12.5,
                        )
                        .color(OK),
                    );
                } else {
                    ui.label(
                        body_text("Synced with the public chain.", 12.5).color(OK),
                    );
                }
            } else if self.peers == 0 {
                ui.label(
                    body_text(
                        "No peers yet. Connect is seednode.hashmonkeys.cloud:39001. Windows Firewall must allow UDP.",
                        12.5,
                    )
                    .color(WARN),
                );
            }
            if self.cfg.mine && !self.auto_mine_ready() {
                ui.label(
                    body_text(
                        "Auto-mine is armed but blocked until this node is within 3 blocks of the seed and Peers > 0.",
                        12.5,
                    )
                    .color(WARN),
                );
            }
        }

        ui.add_space(10.0);
        field_label(ui, "This block pays");
        let cpu = if self.markets.cpu_market.is_empty() {
            "—"
        } else {
            self.markets.cpu_market.as_str()
        };
        let gpu = if self.markets.gpu_market.is_empty() {
            "—"
        } else {
            self.markets.gpu_market.as_str()
        };
        let node_m = if self.markets.node_market.is_empty() {
            "—"
        } else {
            self.markets.node_market.as_str()
        };
        ui.columns(3, |cols| {
            stat_col(&mut cols[0], "Fusion seal", cpu, TEXT_BLUE);
            stat_col(&mut cols[1], "GPU work", gpu, CYAN);
            stat_col(&mut cols[2], "Node work", node_m, OK);
        });
        ui.label(
            body_text(
                "45% Fusion seal · 45% GPU work · 10% nodes. Node 10% is useful work only — not this Overview total to you.",
                11.5,
            )
            .color(MUTED),
        );

        ui.add_space(10.0);
        field_label(ui, "Network");
        let gpu_w = self.pulse.markets.pending_gpu_weight;
        let research = self.pulse.markets.research_eval_receipts;
        let progress = self.pulse.markets.research_progress;
        let (gpu_label, gpu_color) = if gpu_w == 0 {
            ("Waiting", MUTED)
        } else {
            ("Working", OK)
        };
        let (research_label, research_color) = if research == 0 && progress <= 0.0 {
            ("—".to_string(), MUTED)
        } else if progress > 0.0 && progress < 1.0 {
            (format!("{:.0}%", progress * 100.0), CYAN)
        } else if research > 0 {
            (format!("{research}"), OK)
        } else {
            ("Idle".to_string(), MUTED)
        };
        let health = if self.running && self.status == "Online" {
            ("Healthy", OK)
        } else if self.running {
            ("Starting", TEXT_BLUE)
        } else {
            ("Offline", MUTED)
        };
        ui.columns(3, |cols| {
            stat_col(&mut cols[0], "GPU", gpu_label, gpu_color);
            stat_col(&mut cols[1], "Findings", &research_label, research_color);
            stat_col(&mut cols[2], "Node", health.0, health.1);
        });

        if let Some(t) = &self.pulse.trilemma {
            ui.add_space(10.0);
            field_label(ui, "Trilemma");
            ui.columns(4, |cols| {
                stat_col(&mut cols[0], "Sec", &t.sec.to_string(), OK);
                stat_col(&mut cols[1], "Scale", &t.scale.to_string(), CYAN);
                stat_col(&mut cols[2], "Decent", &t.decent.to_string(), TEXT_BLUE);
                stat_col(&mut cols[3], "Transp", &t.transpar.to_string(), MUTED);
            });
            ui.add_space(6.0);
            ui.columns(2, |cols| {
                let bal_color = if t.balance >= 70 {
                    OK
                } else if t.balance >= 50 {
                    CYAN
                } else {
                    Color32::from_rgb(220, 120, 80)
                };
                stat_col(
                    &mut cols[0],
                    "Balance",
                    &format!("{}", t.balance),
                    bal_color,
                );
                let weak = if t.weakest.is_empty() {
                    "—"
                } else {
                    t.weakest.as_str()
                };
                stat_col(&mut cols[1], "Weakest", weak, TEXT_BLUE);
            });
        }

        ui.add_space(10.0);
        field_label(ui, "Soft settings");
        let cons = self.proposals.consensus_difficulty;
        let soft = if self.proposals.soft_diff_hint == 0 {
            cons
        } else {
            self.proposals.soft_diff_hint
        };
        ui.columns(3, |cols| {
            stat_col(
                &mut cols[0],
                "Epoch",
                &self.proposals.param_epoch.to_string(),
                CYAN,
            );
            stat_col(
                &mut cols[1],
                "Difficulty",
                &cons.to_string(),
                TEXT_BLUE,
            );
            let soft_txt = if soft == cons {
                "—".to_string()
            } else {
                soft.to_string()
            };
            stat_col(&mut cols[2], "Soft hint", &soft_txt, MUTED);
        });
        let env = &self.proposals.active_envelopes;
        ui.add_space(4.0);
        ui.columns(3, |cols| {
            stat_col(
                &mut cols[0],
                "Adapt thr",
                &format!("{:.2}", env.soft_adapt_signal_threshold),
                MUTED,
            );
            stat_col(
                &mut cols[1],
                "Bench rounds",
                &env.soft_benchmark_rounds.to_string(),
                MUTED,
            );
            stat_col(
                &mut cols[2],
                "Min verify",
                &env.min_verifier_weight.to_string(),
                MUTED,
            );
        });
        ui.columns(3, |cols| {
            stat_col(
                &mut cols[0],
                "CPU bias",
                &env.suggested_cpu_diff_bias.to_string(),
                MUTED,
            );
            stat_col(
                &mut cols[1],
                "Stipend",
                &env.idle_stipend_bps_cap.to_string(),
                MUTED,
            );
            let v2 = if env.brain_prefer_v2 != 0 { "on" } else { "off" };
            stat_col(&mut cols[2], "Brain v2", v2, MUTED);
        });
        if let Some(ep) = &self.proposals.latest_epoch {
            let line = if !ep.rationale.is_empty() {
                truncate(&ep.rationale, 90)
            } else {
                format!("block {} · v{}", ep.height, ep.epoch)
            };
            ui.label(body_text(line, 12.0).color(OK));
        } else if !self.proposals.last_auto_adapt_proposal_id.is_empty() {
            ui.label(
                body_text(
                    format!("Updated @ block {}", self.proposals.last_auto_adapt_at_height),
                    12.0,
                )
                .color(MUTED),
            );
        }

        let hist: Vec<_> = self
            .proposals
            .epoch_history
            .iter()
            .rev()
            .take(5)
            .cloned()
            .collect();
        if !hist.is_empty() {
            ui.add_space(8.0);
            field_label(ui, "Param epoch history");
            for ep in hist {
                let title = if ep.rationale.is_empty() {
                    format!("v{} @ block {}", ep.epoch, ep.height)
                } else {
                    format!(
                        "v{} @ {} — {}",
                        ep.epoch,
                        ep.height,
                        truncate(&ep.rationale, 64)
                    )
                };
                ui.label(body_text(title, 12.0).color(TEXT_BLUE));
            }
        }

        let recent: Vec<_> = self
            .proposals
            .proposals
            .iter()
            .filter(|p| {
                matches!(p.status.as_str(), "accepted" | "applied")
                    || p.rationale.contains("auto-adapt")
            })
            .rev()
            .take(2)
            .cloned()
            .collect();
        if !recent.is_empty() {
            ui.add_space(8.0);
            field_label(ui, "Recent updates");
            for p in recent {
                let title = if p.rationale.is_empty() {
                    format!("block {}", p.created_at_height)
                } else {
                    truncate(&p.rationale, 80)
                };
                ui.label(body_text(title, 13.0).color(TEXT_BLUE));
            }
        }
    }

    fn ui_earnings(&mut self, ui: &mut egui::Ui) {
        ui.label(
            body_text(
                "This node is paid only for useful work: relaying blocks and txs, routing AI jobs, and serving snapshots or archive. Sitting idle earns nothing. The 10% node pot goes to the deferred vault when nobody has attested work.",
                12.0,
            )
            .color(TEXT_BLUE),
        );
        ui.add_space(12.0);
        field_label(ui, "Reward wallet");
        dark_edit(
            ui,
            &mut self.cfg.operator_address,
            "mesh01… (where node rewards land)",
            true,
        );
        ui.add_space(6.0);
        if primary_btn(ui, "Use this wallet", true).clicked() {
            self.apply_reward_wallet();
        }
        ui.label(
            body_text(
                "Needs ≥ 0.1 MESH on this address to bond. Pending credits already earned stay on the previous address.",
                11.0,
            )
            .color(MUTED),
        );

        ui.add_space(14.0);
        let bal = if self.rewards.balance.is_empty() {
            "—"
        } else {
            self.rewards.balance.as_str()
        };
        ui.label(display_text("Balance", 12.0).color(MUTED));
        ui.label(display_text(bal, 24.0).color(CYAN).strong());

        ui.add_space(10.0);
        ui.columns(2, |cols| {
            stat_col(
                &mut cols[0],
                "Pending weight",
                &self.rewards.pending_weight.to_string(),
                TEXT_BLUE,
            );
            let share = if self.rewards.estimated_share.is_empty() {
                "—"
            } else {
                self.rewards.estimated_share.as_str()
            };
            stat_col(&mut cols[1], "Est. next node share", share, OK);
        });

        ui.add_space(14.0);
        field_label(ui, "What this wallet was paid for");
        if self.pay.by_lane.is_empty() {
            ui.label(
                body_text(
                    "No coinbase on this reward wallet yet. After the node is synced, finds / exam MATCH / Fusion / node work show up here as separate lines.",
                    12.0,
                )
                .color(MUTED),
            );
        } else {
            if !self.pay.rewards.is_empty() {
                ui.label(body_text(format!("Lifetime coinbase {}", self.pay.rewards), 13.0).color(CYAN));
            }
            for lane in &self.pay.by_lane {
                ui.horizontal(|ui| {
                    ui.label(body_text(&lane.title, 13.0).color(INK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            body_text(
                                format!("{} · {}", lane.amount, lane.count),
                                13.0,
                            )
                            .color(CYAN)
                            .strong(),
                        );
                    });
                });
                if !lane.paid_for.is_empty() {
                    ui.label(body_text(&lane.paid_for, 11.5).color(MUTED));
                }
            }
            if !self.pay.recent.is_empty() {
                ui.add_space(8.0);
                field_label(ui, "Recent pays");
                for hit in self.pay.recent.iter().take(10) {
                    let mat = if hit.mature {
                        "spendable".to_string()
                    } else {
                        format!("immature {}/20", hit.confirmations.min(20))
                    };
                    ui.label(
                        body_text(
                            format!(
                                "#{}{}  {}  {}  {}",
                                hit.height,
                                format_unix_local(hit.timestamp)
                                    .map(|t| format!("  {t}"))
                                    .unwrap_or_default(),
                                hit.amount,
                                hit.title,
                                mat
                            ),
                            12.0,
                        )
                        .color(TEXT_BLUE),
                    );
                }
            }
        }

        ui.add_space(10.0);
        field_label(ui, "Soft node score");
        let rep = if self.rewards.reputation_milli == 0 {
            1_000
        } else {
            self.rewards.reputation_milli
        };
        let rtt_f = if self.rewards.relay_rtt_factor_milli == 0 {
            self.network_rtt_factor_milli.max(1)
        } else {
            self.rewards.relay_rtt_factor_milli
        };
        let soft = rep.saturating_mul(rtt_f).saturating_div(1_000);
        ui.columns(3, |cols| {
            stat_col(
                &mut cols[0],
                "Diversity",
                &format!("{rep}‰"),
                TEXT_BLUE,
            );
            stat_col(&mut cols[1], "RTT factor", &format!("{rtt_f}‰"), CYAN);
            stat_col(
                &mut cols[2],
                "Combined",
                &format!("{soft}‰"),
                if soft >= 900 { OK } else if soft >= 700 { WARN } else { MUTED },
            );
        });
        ui.add_space(6.0);
        let bond_txt = if self.rewards.bond_slashed {
            "Slashed".to_string()
        } else if self.rewards.bonded {
            format!(
                "Bonded ({:.4} MESH)",
                self.rewards.bond_locked_atomic as f64 / 100_000_000.0
            )
        } else if self.rewards.bond_eligible {
            "Eligible — register bond".to_string()
        } else {
            "Unbonded (need ≥ 0.1 MESH)".to_string()
        };
        ui.label(
            body_text(format!("Bond: {bond_txt}"), 12.0)
                .color(if self.rewards.bonded { OK } else { MUTED }),
        );

        ui.add_space(12.0);
        field_label(ui, "Address");
        let addr = if !self.rewards.address.is_empty() {
            self.rewards.address.clone()
        } else if !self.wallet_address.is_empty() {
            self.wallet_address.clone()
        } else {
            "—".into()
        };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&addr)
                    .size(12.0)
                    .family(egui::FontFamily::Monospace)
                    .color(TEXT_BLUE),
            );
            if addr != "—" && ghost_btn(ui, "Copy").clicked() {
                ui.ctx().copy_text(addr.clone());
                self.push_log("Address copied", LogKind::Ok);
            }
        });

        ui.add_space(14.0);
        field_label(ui, "Send");
        dark_edit(ui, &mut self.send_to, "destination address", true);
        ui.add_space(6.0);
        dark_edit(ui, &mut self.send_amount, "amount (MESH)", true);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if primary_btn(ui, "Send", self.running).clicked() {
                self.send_to_address();
            }
            if !self.send_status.is_empty() {
                ui.label(body_text(&self.send_status, 12.0).color(MUTED));
            }
        });

        if !self.backup_ack {
            ui.add_space(16.0);
            ui.label(
                body_text(
                    "Back up your wallet key file before moving funds. Losing it means losing coins.",
                    12.0,
                )
                .color(WARN),
            );
            if ghost_btn(ui, "Acknowledge").clicked() {
                self.backup_ack = true;
            }
        }
    }

    fn ui_network(&mut self, ui: &mut egui::Ui, editable: bool) {
        ui.columns(3, |cols| {
            stat_col(
                &mut cols[0],
                "Peers",
                &self.peers.to_string(),
                if self.peers > 0 { OK } else { MUTED },
            );
            let rtt = self
                .median_peer_rtt_ms
                .map(|ms| format!("{ms} ms"))
                .unwrap_or_else(|| "—".into());
            stat_col(&mut cols[1], "Median RTT", &rtt, TEXT_BLUE);
            let shard = if self.ai_shard_count > 1 {
                format!("{}/{}", self.ai_shard_id, self.ai_shard_count)
            } else {
                "—".into()
            };
            stat_col(&mut cols[2], "AI shard", &shard, CYAN);
        });
        ui.add_space(12.0);

        field_label(ui, "Connect seeds (comma-separated)");
        dark_edit(
            ui,
            &mut self.peer_edit,
            "seednode.hashmonkeys.cloud:39001",
            editable,
        );
        ui.add_space(10.0);

        field_label(ui, "P2P listen");
        dark_edit(ui, &mut self.cfg.listen, "0.0.0.0:39011", editable);

        if rpc_is_loopback_only(&self.cfg.rpc) {
            ui.add_space(8.0);
            ui.label(
                body_text("LAN peers can't reach this RPC", 12.0).color(MUTED),
            );
        }

        ui.add_space(12.0);
        if ghost_btn_enabled(ui, "Save", editable).clicked() {
            self.save_settings();
        }
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui, editable: bool) {
        field_label(ui, "RPC");
        dark_edit(ui, &mut self.cfg.rpc, "0.0.0.0:18080", editable);
        ui.add_space(10.0);

        field_label(ui, "Research coordinator / orch");
        dark_edit(
            ui,
            &mut self.cfg.orch,
            "http://seednode.hashmonkeys.cloud:18080",
            editable,
        );
        ui.add_space(10.0);

        ui.add_enabled_ui(editable, |ui| {
            let mut mine = self.cfg.mine;
            if pointer(ui.checkbox(
                &mut mine,
                body_text("Auto-mine when synced (solo/dev — needs a peer)", 13.5).color(TEXT_BLUE),
            ))
            .changed()
            {
                self.cfg.mine = mine;
            }
        });
        if self.cfg.mine {
            let note = if self.auto_mine_ready() {
                "Ready — still forks if you mine without the public miner. Prefer MonkeyMesh Miner."
            } else {
                "Blocked until height is within 3 of the seed and Peers > 0."
            };
            ui.label(body_text(note, 12.0).color(if self.auto_mine_ready() { MUTED } else { WARN }));
        }
        ui.add_space(12.0);

        field_label(ui, "Chain path");
        dark_edit(ui, &mut self.cfg.chain, "data/chain.bin", editable);
        ui.add_space(10.0);

        field_label(ui, "Wallet key");
        dark_edit(ui, &mut self.cfg.wallet, "data/wallet.key", editable);
        ui.add_space(10.0);

        field_label(ui, "P2P key");
        dark_edit(ui, &mut self.cfg.p2p_key, "data/p2p.key", editable);
        ui.add_space(10.0);

        field_label(ui, "Miner key");
        dark_edit(ui, &mut self.cfg.miner_key, "data/wallet.key", editable);
        ui.add_space(10.0);

        field_label(ui, "Reward wallet (node useful-work payout)");
        dark_edit(
            ui,
            &mut self.cfg.operator_address,
            "mesh01… (Earnings tab can set this too)",
            editable,
        );
        ui.add_space(10.0);

        field_label(ui, "Cold operator vault (optional)");
        dark_edit(
            ui,
            &mut self.cfg.operator_vault,
            "data/cold.vault.json",
            editable,
        );
        ui.label(
            body_text("Address-only — vault is never unlocked by the node.", 11.0).color(MUTED),
        );
        ui.add_space(12.0);

        if ghost_btn_enabled(ui, "Save", editable).clicked() {
            self.save_settings();
        }
    }
}

fn tab_btn(ui: &mut egui::Ui, current: &mut MainTab, tab: MainTab, label: &str) {
    let selected = *current == tab;
    let fill = if selected {
        Color32::from_rgba_unmultiplied(46, 220, 240, 28)
    } else {
        Color32::from_rgba_unmultiplied(14, 22, 32, 200)
    };
    let stroke = if selected {
        egui::Stroke::new(1.0, CYAN)
    } else {
        egui::Stroke::new(1.0, RULE)
    };
    let resp = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(leaf_radius())
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.label(
                display_text(label, 13.0).color(if selected { INK } else { MUTED }).strong(),
            );
        })
        .response
        .interact(egui::Sense::click());
    if pointer(resp).clicked() {
        *current = tab;
    }
}

fn field_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() * 0.48).clamp(280.0, 420.0)
}

fn dark_edit(ui: &mut egui::Ui, text: &mut String, hint: &str, editable: bool) {
    let w = field_width(ui);
    egui::Frame::new()
        .fill(FIELD_BG)
        .stroke(egui::Stroke::new(1.0, RULE))
        .corner_radius(leaf_radius())
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(w);
            ui.visuals_mut().extreme_bg_color = FIELD_BG;
            ui.visuals_mut().override_text_color = Some(TEXT_BLUE);
            ui.add_enabled(
                editable,
                egui::TextEdit::singleline(text)
                    .desired_width(w - 20.0)
                    .clip_text(true)
                    .hint_text(body_text(hint, 13.0).color(MUTED))
                    .text_color(TEXT_BLUE)
                    .frame(false),
            );
        });
}

fn stat_col(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    tile().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(display_text(label.to_uppercase(), 10.5).color(MUTED));
        ui.add(egui::Label::new(display_text(value, 16.0).color(color).strong()).truncate());
    });
}

fn load_texture(
    ctx: &egui::Context,
    name: &str,
    bytes: &[u8],
    max: u32,
) -> Option<TextureHandle> {
    let img = image::load_from_memory(bytes).ok()?;
    let w = img.width();
    let h = img.height();
    let img = if w <= max && h <= max {
        img
    } else if w >= h {
        img.resize(
            max,
            ((h as f32) * (max as f32) / (w as f32)).max(1.0) as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img.resize(
            ((w as f32) * (max as f32) / (h as f32)).max(1.0) as u32,
            max,
            image::imageops::FilterType::Triangle,
        )
    };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture(
        name,
        color,
        egui::TextureOptions {
            magnification: egui::TextureFilter::Linear,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        },
    ))
}
