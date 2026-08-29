//! MonkeyMesh Miner — all-in-one desktop GUI (CPU + NVIDIA + AMD, multi-device).

#![windows_subsystem = "windows"]

mod theme;

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ScrollArea, TextureHandle};
use mesh_miner_gpu::ai_worker::{run_ai_loop, AiCapacity};
use mesh_miner_gpu::engine::{
    ai_capacity_from_selection, enumerate_devices, format_hashrate, looks_like_pool_target,
    normalize_pool_url, run_rpc_loop, sanitize_worker_name, ComputeDevice, DeviceInfo, MinerConfig,
    MinerEvent,
};
use meshhash_cpu::{
    fusion_sequential_active, pow_fusion_sequential_height, pow_version_for_height,
};
use mesh_types::Address;
use serde::{Deserialize, Serialize};
use theme::{
    body_text, display_text, dual_rail, field_label, ghost_btn_enabled, leaf_radius,
    paint_brand_backdrop, paint_resize_grip, pointer, primary_btn, tile, CYAN, DANGER, FIELD_BG,
    INK, MUTED, OK, RULE, TEXT_BLUE, WARN,
};

const SCENE_PNG: &[u8] = include_bytes!("../assets/coin_scene.png");
const MASCOT_PNG: &[u8] = include_bytes!("../assets/mascot.png");
const COIN_PNG: &[u8] = include_bytes!("../assets/coin_mark.png");

const WIN_W: f32 = 960.0;
const WIN_H: f32 = 720.0;
const LOG_CAP: usize = 200;
const BOTTOM_H_DEFAULT: f32 = 200.0;
const BOTTOM_H_MIN: f32 = 88.0;
const BOTTOM_H_MAX: f32 = 560.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GuiConfig {
    /// Single mine target: HTTP pool URL and/or comma-separated node RPC bases.
    #[serde(default = "default_mine_target")]
    rpc: String,
    #[serde(default)]
    address: String,
    /// Optional worker / rig name (pool credits as address.worker).
    #[serde(default)]
    worker_name: String,
    #[serde(default = "default_batch")]
    batch: u32,
    #[serde(default = "default_max_nonces")]
    max_nonces: u64,
    /// Serialized device keys, e.g. "cpu", "cuda:0", "opencl:1"
    #[serde(default)]
    selected: Vec<String>,
    /// Bottom mining strip height (user-controlled).
    #[serde(default = "default_bottom_h")]
    bottom_h: f32,
    /// Pull AI research / MNIST jobs (GPU market 40%).
    #[serde(default = "default_ai_research")]
    ai_research: bool,
    /// Legacy: merged into `rpc` on load when set.
    #[serde(default, skip_serializing)]
    pool: String,
}

fn default_mine_target() -> String {
    "https://eu.hashmonkeys.cloud".into()
}
fn default_batch() -> u32 {
    // 0 = auto-scale parallel pads from GPU VRAM (recommended).
    0
}
fn default_max_nonces() -> u64 {
    5_000_000
}
fn default_bottom_h() -> f32 {
    BOTTOM_H_DEFAULT
}
fn default_ai_research() -> bool {
    true
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            rpc: default_mine_target(),
            address: String::new(),
            worker_name: String::new(),
            batch: default_batch(),
            max_nonces: default_max_nonces(),
            selected: Vec::new(),
            bottom_h: default_bottom_h(),
            ai_research: default_ai_research(),
            pool: String::new(),
        }
    }
}

struct MinerApp {
    cfg: GuiConfig,
    cfg_path: PathBuf,
    catalog: Vec<DeviceInfo>,
    selected: HashSet<ComputeDevice>,
    /// Any work running (PoW and/or AI).
    mining: bool,
    /// Stop clicked; workers have not exited yet.
    stopping: bool,
    pow_active: bool,
    ai_active: bool,
    gpu_wanted: bool,
    stop: Option<Arc<AtomicBool>>,
    event_tx: Sender<MinerEvent>,
    event_rx: Receiver<MinerEvent>,
    cpu_hashrate: f64,
    gpu_hashrate: f64,
    /// Smoothed Fusion mix (no exam-eq).
    gpu_mix_ema: f64,
    last_exam_at: Option<Instant>,
    last_hashrate_at: Option<Instant>,
    blocks_found: u64,
    ai_jobs_done: u64,
    /// Shared network brain epoch (from last verified AI job).
    brain_epoch: Option<u64>,
    last_height: Option<u64>,
    active_label: String,
    status: String,
    status_ok: bool,
    log: Vec<LogLine>,
    /// Coalesce consecutive cheap AI jobs into one feed line.
    ai_stream_kind: Option<String>,
    ai_stream_n: u32,
    batch_str: String,
    pay: Option<PaySnapshot>,
    pay_tx: Sender<Option<PaySnapshot>>,
    pay_rx: Receiver<Option<PaySnapshot>>,
    pay_inflight: Arc<AtomicBool>,
    last_pay_poll: Instant,
    logged_pay_height: Option<u64>,
    tab: MainTab,
    event_filter: EventFilter,
    node: Option<NodePulse>,
    node_tx: Sender<Option<NodePulse>>,
    node_rx: Receiver<Option<NodePulse>>,
    node_inflight: Arc<AtomicBool>,
    last_node_poll: Instant,
    last_node_height: Option<u64>,
    warned_v5: bool,
    scene: Option<TextureHandle>,
    mascot: Option<TextureHandle>,
    coin: Option<TextureHandle>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MainTab {
    Mining,
    Node,
    Events,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventFilter {
    All,
    Mine,
    Node,
    Pay,
    Ai,
    Err,
}

#[derive(Clone, Copy)]
enum EventSrc {
    Mine,
    Node,
    Pay,
    Ai,
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
    src: EventSrc,
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
    let (tag, tag_col) = match line.src {
        EventSrc::Mine => ("MINE", CYAN),
        EventSrc::Node => ("NODE", TEXT_BLUE),
        EventSrc::Pay => ("PAY", OK),
        EventSrc::Ai => ("AI", WARN),
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(body_text(&line.at, 12.0).color(MUTED).monospace());
        ui.label(body_text(tag, 11.0).color(tag_col).strong().monospace());
        ui.label(body_text(&line.msg, 13.0).color(col));
    });
}

fn main() -> eframe::Result<()> {
    let cfg_path = config_path();
    let cfg = load_config(&cfg_path);
    let catalog = enumerate_devices();
    let selected = restore_selection(&cfg.selected, &catalog);
    let (event_tx, event_rx) = mpsc::channel::<MinerEvent>();
    let (pay_tx, pay_rx) = mpsc::channel::<Option<PaySnapshot>>();
    let (node_tx, node_rx) = mpsc::channel::<Option<NodePulse>>();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WIN_W, WIN_H])
            .with_min_inner_size([800.0, 600.0])
            .with_resizable(true)
            .with_title("MonkeyMesh Miner"),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "MonkeyMesh Miner",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = MinerApp {
                batch_str: cfg.batch.to_string(),
                active_label: summarize_selection(&selected),
                cfg,
                cfg_path,
                catalog,
                selected,
                mining: false,
                stopping: false,
                pow_active: false,
                ai_active: false,
                gpu_wanted: false,
                stop: None,
                event_tx,
                event_rx,
                cpu_hashrate: 0.0,
                gpu_hashrate: 0.0,
                gpu_mix_ema: 0.0,
                last_exam_at: None,
                last_hashrate_at: None,
                blocks_found: 0,
                ai_jobs_done: 0,
                brain_epoch: None,
                last_height: None,
                status: "Ready".into(),
                status_ok: true,
                log: vec![],
                ai_stream_kind: None,
                ai_stream_n: 0,
                pay: None,
                pay_tx,
                pay_rx,
                pay_inflight: Arc::new(AtomicBool::new(false)),
                last_pay_poll: Instant::now() - Duration::from_secs(30),
                logged_pay_height: None,
                tab: MainTab::Mining,
                event_filter: EventFilter::All,
                node: None,
                node_tx,
                node_rx,
                node_inflight: Arc::new(AtomicBool::new(false)),
                last_node_poll: Instant::now() - Duration::from_secs(30),
                last_node_height: None,
                warned_v5: false,
                scene: None,
                mascot: None,
                coin: None,
            };
            app.push_log(
                "Pick CPU and/or a GPU, then press Start. GPU mixes the pad; CPU seals the Fusion digest on the live tip (45%). GPU work is the other 45%. Jobs drop the moment the tip moves — no hashing a dead height.",
                LogKind::Info,
            );
            app.scene = load_texture(&cc.egui_ctx, "coin_scene", SCENE_PNG, 1400);
            app.mascot = load_texture(&cc.egui_ctx, "mascot", MASCOT_PNG, 256);
            app.coin = load_texture(&cc.egui_ctx, "coin_mark", COIN_PNG, 192);
            Ok(Box::new(app))
        }),
    )
}

fn config_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("config.json");
        }
    }
    PathBuf::from("config.json")
}

fn load_config(path: &PathBuf) -> GuiConfig {
    let Ok(raw) = fs::read_to_string(path) else {
        return GuiConfig::default();
    };
    let mut cfg: GuiConfig = serde_json::from_str(&raw).unwrap_or_default();
    // Migrate legacy separate Pool field into the single mine target.
    let legacy_pool = normalize_pool_url(&cfg.pool);
    if !legacy_pool.is_empty() {
        cfg.rpc = legacy_pool;
        cfg.pool.clear();
    }
    cfg.rpc = cfg
        .rpc
        .split(',')
        .map(|p| normalize_pool_url(p))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(",");
    // Prefer public HTTPS front (WAN :12500 is not forwarded on the router).
    if cfg.rpc == "http://eu.hashmonkeys.cloud:12500"
        || cfg.rpc == "http://eu.hashmonkeys.cloud"
        || cfg.rpc == "https://eu.hashmonkeys.cloud:12500"
    {
        cfg.rpc = default_mine_target();
    }
    if cfg.rpc.is_empty() {
        cfg.rpc = default_mine_target();
    }
    cfg.worker_name = sanitize_worker_name(&cfg.worker_name);
    cfg
}

fn save_config(path: &PathBuf, cfg: &GuiConfig) {
    if let Ok(raw) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(path, raw);
    }
}

fn device_key(d: ComputeDevice) -> String {
    d.key()
}

fn parse_device_key(s: &str) -> Option<ComputeDevice> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("cpu") {
        return Some(ComputeDevice::Cpu);
    }
    if let Some(rest) = s.strip_prefix("cuda:") {
        return rest.parse().ok().map(|index| ComputeDevice::Cuda { index });
    }
    if let Some(rest) = s.strip_prefix("opencl:") {
        return rest
            .parse()
            .ok()
            .map(|index| ComputeDevice::OpenCl { index });
    }
    None
}

fn restore_selection(saved: &[String], catalog: &[DeviceInfo]) -> HashSet<ComputeDevice> {
    let available: HashSet<_> = catalog.iter().map(|d| d.id).collect();
    let mut set = HashSet::new();
    for s in saved {
        if let Some(d) = parse_device_key(s) {
            if available.contains(&d) {
                set.insert(d);
            }
        }
    }
    if set.is_empty() {
        // Prefer first NVIDIA CUDA, else first AMD/OpenCL GPU, else CPU.
        if let Some(nvidia) = catalog.iter().find(|d| d.family == "NVIDIA") {
            set.insert(nvidia.id);
        } else if let Some(amd) = catalog.iter().find(|d| d.family == "AMD") {
            set.insert(amd.id);
        } else if let Some(gpu) = catalog.iter().find(|d| !matches!(d.id, ComputeDevice::Cpu)) {
            set.insert(gpu.id);
        } else {
            set.insert(ComputeDevice::Cpu);
        }
    }
    set
}

fn summarize_selection(sel: &HashSet<ComputeDevice>) -> String {
    if sel.is_empty() {
        return "None".into();
    }
    let mut parts: Vec<String> = sel.iter().map(|d| d.short_label()).collect();
    parts.sort();
    if parts.len() <= 3 {
        parts.join(" + ")
    } else {
        format!("{} devices", parts.len())
    }
}

impl MinerApp {
    fn push_log(&mut self, msg: impl Into<String>, kind: LogKind) {
        self.push_event(EventSrc::Mine, msg, kind);
    }

    fn push_event(&mut self, src: EventSrc, msg: impl Into<String>, kind: LogKind) {
        self.log.push(LogLine {
            at: log_stamp(),
            msg: msg.into(),
            kind,
            src,
        });
        if self.log.len() > LOG_CAP {
            let n = self.log.len() - LOG_CAP;
            self.log.drain(0..n);
        }
    }

    fn note_ai_job(&mut self, kind: &str, brain_epoch: Option<u64>) {
        let (base, backend) = match kind.split_once('/') {
            Some((b, be)) => (b, be),
            None => (kind, ""),
        };
        let cheap = matches!(base, "protocol_eval" | "benchmark");
        if cheap {
            let noun = if base == "protocol_eval" {
                "protocol sims"
            } else {
                "benchmarks"
            };
            if self.ai_stream_kind.as_deref() == Some(base) {
                self.ai_stream_n = self.ai_stream_n.saturating_add(1);
                if let Some(last) = self.log.last_mut() {
                    last.at = log_stamp();
                    last.msg = format!(
                        "Research ×{} {noun} — no extra MESH · GPU Fusion still hashing",
                        self.ai_stream_n
                    );
                    last.kind = LogKind::Ok;
                    last.src = EventSrc::Ai;
                }
                return;
            }
            self.ai_stream_kind = Some(base.to_string());
            self.ai_stream_n = 1;
            self.push_event(
                EventSrc::Ai,
                format!("Research ×1 {noun} — no extra MESH · GPU Fusion still hashing"),
                LogKind::Ok,
            );
            return;
        }
        self.ai_stream_kind = None;
        self.ai_stream_n = 0;
        let line = match base {
            "ml_train" | "ml_train_shared" | "ml_train_shared_v2" => {
                let epoch = brain_epoch
                    .map(|e| format!(" · epoch {e}"))
                    .unwrap_or_default();
                format!("Network brain step{epoch} — research only, no extra MESH")
            }
            "leg_train" | "quantum_train" => {
                "Guardian train — research only, no extra MESH".into()
            }
            "exam" => {
                self.last_exam_at = Some(Instant::now());
                "GPU exam matched — this is the 45% GPU ticket (same weight as Fusion). Does not find the block.".into()
            }
            other => format!("{other} — research only, no extra MESH"),
        };
        let _ = backend;
        self.push_event(EventSrc::Ai, line, LogKind::Ok);
    }

    fn persist_selection(&mut self) {
        let mut keys: Vec<String> = self.selected.iter().map(|d| device_key(*d)).collect();
        keys.sort();
        self.cfg.selected = keys;
        self.cfg.batch = self.batch_str.trim().parse().unwrap_or(0);
        self.active_label = summarize_selection(&self.selected);
    }

    fn apply_fields(&mut self) -> Result<(), String> {
        self.cfg.rpc = self
            .cfg
            .rpc
            .split(',')
            .map(|p| normalize_pool_url(p))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        self.cfg.pool.clear();
        self.cfg.worker_name = sanitize_worker_name(&self.cfg.worker_name);
        self.cfg.address = self.cfg.address.trim().to_string();
        if self.cfg.rpc.is_empty() {
            return Err("Enter a mine target (pool or node URL)".into());
        }
        if self.cfg.address.is_empty() {
            return Err("Enter your wallet address".into());
        }
        if Address::from_hex(&self.cfg.address).is_none() {
            return Err("That address does not look right".into());
        }
        if self.selected.is_empty() {
            return Err("Pick CPU and/or a GPU to mine".into());
        }
        self.persist_selection();
        Ok(())
    }

    fn start_mining(&mut self) {
        if self.mining || self.stopping {
            return;
        }
        if let Err(e) = self.apply_fields() {
            self.status = e.clone();
            self.status_ok = false;
            self.push_log(e, LogKind::Err);
            return;
        }
        save_config(&self.cfg_path, &self.cfg);

        let Some(addr) = Address::from_hex(&self.cfg.address) else {
            self.status = "bad payout address".into();
            self.status_ok = false;
            return;
        };

        let cpu_selected = self
            .selected
            .iter()
            .any(|d| matches!(d, ComputeDevice::Cpu));
        let gpu_selected = self
            .selected
            .iter()
            .any(|d| !matches!(d, ComputeDevice::Cpu));
        // CPU and/or GPU search Fusion. Exam sidecar rides every template (fair 45/45).
        let do_pow = cpu_selected || gpu_selected;
        let do_ai = gpu_selected && self.cfg.ai_research;
        if !do_pow && !do_ai {
            self.status = "Pick CPU and/or a GPU".into();
            self.status_ok = false;
            return;
        }

        let stop = Arc::new(AtomicBool::new(false));
        self.stop = Some(stop.clone());
        self.mining = true;
        self.stopping = false;
        self.gpu_wanted = gpu_selected;
        self.pow_active = do_pow;
        self.ai_active = do_ai;
        self.cpu_hashrate = 0.0;
        self.gpu_hashrate = 0.0;
        self.gpu_mix_ema = 0.0;
        self.last_exam_at = None;
        self.last_hashrate_at = None;
        self.blocks_found = 0;
        self.ai_jobs_done = 0;
        self.brain_epoch = None;
        self.ai_stream_kind = None;
        self.ai_stream_n = 0;
        self.active_label = summarize_selection(&self.selected);
        self.status = match (cpu_selected, gpu_selected) {
            (true, true) => "GPU work + CPU Fusion seal...".into(),
            (true, false) => "CPU Fusion seal + exam...".into(),
            (false, true) => "GPU work + CPU Fusion seal...".into(),
            (false, false) => "Idle".into(),
        };
        self.status_ok = true;
        if cpu_selected {
            self.push_log(
                "CPU started — fills the pad and seals Fusion on the live tip (45%)",
                LogKind::Ok,
            );
        }
        if gpu_selected {
            self.push_log(
                "GPU started — mixes the pad (GPU work 45%). CPU seals the winning nonce.",
                LogKind::Ok,
            );
            self.push_log(
                "Split is 45% Fusion seal / 45% GPU work / 10% nodes. Pads stay on the box.",
                LogKind::Info,
            );
        }

        let tx = self.event_tx.clone();
        if do_pow {
            let devices: Vec<ComputeDevice> = self.selected.iter().copied().collect();
            let cfg = MinerConfig::with_devices(
                self.cfg.rpc.clone(),
                addr,
                self.cfg.batch,
                self.cfg.max_nonces,
                devices,
            )
            .with_worker_name(Some(if self.cfg.worker_name.is_empty() {
                "default".into()
            } else {
                self.cfg.worker_name.clone()
            }));
            let stop_pow = stop.clone();
            let tx_pow = tx.clone();
            thread::spawn(move || run_rpc_loop(cfg, stop_pow, tx_pow));
        }
        if do_ai {
            // AI board lives on seed/edges — never point GPU jobs at the HTTP pool.
            let orch = if looks_like_pool_target(&self.cfg.rpc) {
                mesh_types::default_rpc_urls().join(",")
            } else {
                self.cfg.rpc.clone()
            };
            let address = self.cfg.address.clone();
            let stop_ai = stop.clone();
            let (gpu_name, vram_bytes) =
                ai_capacity_from_selection(&self.catalog, &self.selected);
            let capacity = AiCapacity::from_vram(gpu_name, vram_bytes);
            self.push_event(
                EventSrc::Ai,
                format!(
                    "Research sidecar: {} · {} MB · {} puller(s) — GPU trains v2 when Fusion is free; CPU exams only; seed rematches",
                    capacity.gpu_name,
                    capacity.vram_mb,
                    capacity.local_pullers(),
                ),
                LogKind::Info,
            );
            thread::spawn(move || run_ai_loop(orch, address, capacity, stop_ai, tx));
        }
    }

    fn stop_mining(&mut self) {
        if self.stopping {
            return;
        }
        if let Some(s) = &self.stop {
            s.store(true, Ordering::SeqCst);
        }
        self.stopping = true;
        self.status = "Stopping…".into();
        self.status_ok = true;
        self.push_log("Stop requested", LogKind::Warn);
        // Workers check the flag between Fusion mix slices; keep the UI live.
    }

    fn finish_if_idle(&mut self) {
        if !self.pow_active && !self.ai_active {
            self.mining = false;
            self.stopping = false;
            self.stop = None;
            self.cpu_hashrate = 0.0;
            self.gpu_hashrate = 0.0;
            self.gpu_mix_ema = 0.0;
            self.last_exam_at = None;
            self.last_hashrate_at = None;
            self.status = "Stopped".into();
            self.status_ok = true;
        }
    }

    fn drain(&mut self) {
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                MinerEvent::Status(s) => {
                    if !s.is_empty() {
                        self.push_log(s, LogKind::Info);
                    }
                }
                MinerEvent::Hashrate { cpu_hs, gpu_hs } => {
                    if self.stopping {
                        continue;
                    }
                    self.last_hashrate_at = Some(Instant::now());
                    if cpu_hs > 0.0 {
                        self.cpu_hashrate = cpu_hs;
                    }
                    if gpu_hs > 0.0 {
                        self.gpu_mix_ema = gpu_hs;
                        self.gpu_hashrate = gpu_hs;
                    }
                    if cpu_hs == 0.0 && gpu_hs == 0.0 {
                        self.cpu_hashrate = 0.0;
                        self.gpu_hashrate = 0.0;
                        self.gpu_mix_ema = 0.0;
                    }
                }
                MinerEvent::BlockFound { height, id: _ } => {
                    if self.stopping {
                        continue;
                    }
                    self.blocks_found += 1;
                    self.last_height = Some(height);
                    self.status = format!("Fusion sealed #{height}");
                    self.status_ok = true;
                    self.push_log(format!("Fusion sealed block #{height}"), LogKind::Ok);
                    self.last_pay_poll = Instant::now() - Duration::from_secs(30);
                }
                MinerEvent::AiJobDone {
                    job_id: _,
                    kind,
                    brain_epoch,
                } => {
                    self.ai_jobs_done += 1;
                    if brain_epoch.is_some() {
                        self.brain_epoch = brain_epoch;
                    }
                    self.note_ai_job(&kind, brain_epoch);
                    if !self.pow_active {
                        self.status = match self.brain_epoch {
                            Some(e) => format!("Research · brain epoch {e}"),
                            None => "Research sidecar running".into(),
                        };
                        self.status_ok = true;
                    }
                }
                MinerEvent::Error(e) => {
                    if is_race_log(&e) {
                        continue;
                    }
                    if e.starts_with("AI advertise") || e.starts_with("AI job:") {
                        self.status = e.clone();
                        self.status_ok = false;
                        self.push_event(EventSrc::Ai, e, LogKind::Err);
                    } else {
                        let (status, hint) =
                            format_miner_error(&e, looks_like_pool_target(&self.cfg.rpc));
                        self.status = status;
                        self.status_ok = false;
                        self.push_log(hint, LogKind::Err);
                    }
                }
                MinerEvent::Stopped => {
                    self.pow_active = false;
                    self.cpu_hashrate = 0.0;
                    self.gpu_hashrate = 0.0;
                    self.gpu_mix_ema = 0.0;
                    self.finish_if_idle();
                }
                MinerEvent::AiStopped => {
                    self.ai_active = false;
                    self.finish_if_idle();
                }
            }
        }
    }

    fn drain_pay(&mut self) {
        while let Ok(snap) = self.pay_rx.try_recv() {
            self.pay_inflight.store(false, Ordering::SeqCst);
            if let Some(snap) = snap {
                if let Some(hit) = snap.recent.first() {
                    if self.logged_pay_height != Some(hit.height) {
                        self.logged_pay_height = Some(hit.height);
                        let same_block: Vec<_> = snap
                            .recent
                            .iter()
                            .filter(|h| h.height == hit.height)
                            .collect();
                        for h in same_block {
                            self.push_event(
                                EventSrc::Pay,
                                format!("Paid {} — {}", h.amount, h.title),
                                LogKind::Ok,
                            );
                        }
                    }
                }
                self.pay = Some(snap);
            }
        }
    }

    fn maybe_poll_pay(&mut self) {
        if self.cfg.address.trim().is_empty() {
            return;
        }
        if self.pay_inflight.load(Ordering::SeqCst) {
            return;
        }
        if self.last_pay_poll.elapsed() < Duration::from_secs(12) {
            return;
        }
        self.last_pay_poll = Instant::now();
        self.pay_inflight.store(true, Ordering::SeqCst);
        let rpc = self.cfg.rpc.clone();
        let addr = self.cfg.address.clone();
        let tx = self.pay_tx.clone();
        thread::spawn(move || {
            let _ = tx.send(fetch_pay_snapshot(&rpc, &addr));
        });
    }

    fn drain_node(&mut self) {
        while let Ok(pulse) = self.node_rx.try_recv() {
            self.node_inflight.store(false, Ordering::SeqCst);
            let Some(pulse) = pulse else {
                continue;
            };
            if self.last_node_height != Some(pulse.height) {
                let prev = self.last_node_height;
                self.last_node_height = Some(pulse.height);
                self.last_height = Some(pulse.height);
                if prev.is_some() {
                    self.push_event(
                        EventSrc::Node,
                        format!(
                            "Height {} · tip {} · diff {} · {} peer(s)",
                            pulse.height,
                            short_hash(&pulse.tip),
                            pulse.next_difficulty,
                            pulse.peers
                        ),
                        LogKind::Info,
                    );
                }
            }
            let v5_at = pow_fusion_sequential_height();
            if !self.warned_v5 && !fusion_sequential_active(pulse.height) {
                let left = v5_at.saturating_sub(pulse.height);
                if left <= 2_000 {
                    self.warned_v5 = true;
                    self.push_event(
                        EventSrc::Node,
                        format!(
                            "Sequential Fusion (v5) in {left} blocks — hop this miner before #{v5_at}. Official CPU-only refuses v5; GPU required."
                        ),
                        LogKind::Warn,
                    );
                }
            }
            self.node = Some(pulse);
        }
    }

    fn maybe_poll_node(&mut self) {
        if self.node_inflight.load(Ordering::SeqCst) {
            return;
        }
        if self.last_node_poll.elapsed() < Duration::from_secs(8) {
            return;
        }
        self.last_node_poll = Instant::now();
        self.node_inflight.store(true, Ordering::SeqCst);
        let rpc = self.cfg.rpc.clone();
        let tx = self.node_tx.clone();
        thread::spawn(move || {
            let _ = tx.send(fetch_node_pulse(&rpc));
        });
    }

    fn toggle_family_cpu(&mut self, on: bool) {
        if on {
            self.selected.insert(ComputeDevice::Cpu);
        } else {
            self.selected.remove(&ComputeDevice::Cpu);
        }
        self.persist_selection();
    }

    fn select_all_gpus(&mut self, on: bool) {
        for d in &self.catalog {
            if matches!(d.id, ComputeDevice::Cpu) {
                continue;
            }
            if on {
                self.selected.insert(d.id);
            } else {
                self.selected.remove(&d.id);
            }
        }
        self.persist_selection();
    }

    fn ui_mining_session(&self, ui: &mut egui::Ui) {
        let height = self
            .last_height
            .or_else(|| self.node.as_ref().map(|n| n.height));
        let v5_at = pow_fusion_sequential_height();
        let pow = match height {
            Some(h) => {
                let ver = pow_version_for_height(h);
                if fusion_sequential_active(h) {
                    format!("v{ver} sequential")
                } else {
                    let left = v5_at.saturating_sub(h);
                    format!("v{ver} · v5 in {left}")
                }
            }
            None => "—".into(),
        };
        let tip = self
            .node
            .as_ref()
            .map(|n| short_hash(&n.tip))
            .unwrap_or_else(|| "—".into());
        let devices = if self.active_label.is_empty() {
            "none".into()
        } else {
            self.active_label.clone()
        };
        ui.columns(4, |cols| {
            stat_col(
                &mut cols[0],
                "Height",
                &height.map(|h| h.to_string()).unwrap_or_else(|| "—".into()),
                CYAN,
            );
            stat_col(&mut cols[1], "PoW", &pow, WARN);
            stat_col(&mut cols[2], "Tip", &tip, TEXT_BLUE);
            stat_col(&mut cols[3], "Devices", &devices, INK);
        });
        if let Some(h) = height {
            if !fusion_sequential_active(h) {
                ui.add_space(6.0);
                ui.label(
                    body_text(
                        format!(
                            "Live tip is still Fusion v4. Sequential v5 (GPU wave → CPU seal) starts at #{v5_at}. Official CPU-only miners refuse v5 — keep this GPU app hopped."
                        ),
                        12.0,
                    )
                    .color(TEXT_BLUE),
                );
            }
        }
    }

    fn ui_node(&self, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .id_salt("miner_node")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                field_label(ui, "Connected node");
                ui.label(
                    body_text(
                        if self.cfg.rpc.trim().is_empty() {
                            "No mine target set."
                        } else {
                            self.cfg.rpc.trim()
                        },
                        13.0,
                    )
                    .color(TEXT_BLUE),
                );
                ui.add_space(10.0);

                let Some(n) = &self.node else {
                    ui.label(
                        body_text(
                            "Waiting for /v1/getnodeinfo on the mine target (pool or node). Check Mine target on the Mining tab.",
                            13.0,
                        )
                        .color(MUTED),
                    );
                    return;
                };

                let v5_at = pow_fusion_sequential_height();
                let pow = pow_version_for_height(n.height);
                let era = if fusion_sequential_active(n.height) {
                    format!("v{pow} sequential live")
                } else {
                    format!(
                        "v{pow} · {} blocks to v5 (#{v5_at})",
                        v5_at.saturating_sub(n.height)
                    )
                };
                let finality = if n.finality_active {
                    format!("#{}", n.finalized_height)
                } else {
                    "off".into()
                };
                ui.columns(4, |cols| {
                    stat_col(&mut cols[0], "Height", &n.height.to_string(), CYAN);
                    stat_col(&mut cols[1], "Difficulty", &n.next_difficulty.to_string(), TEXT_BLUE);
                    stat_col(&mut cols[2], "Peers", &n.peers.to_string(), INK);
                    stat_col(&mut cols[3], "Finality", &finality, if n.finality_active { OK } else { MUTED });
                });
                ui.add_space(8.0);
                ui.columns(4, |cols| {
                    stat_col(&mut cols[0], "PoW era", &era, WARN);
                    stat_col(&mut cols[1], "Tip", &short_hash(&n.tip), TEXT_BLUE);
                    stat_col(&mut cols[2], "Genesis", &short_hash(&n.genesis), MUTED);
                    stat_col(
                        &mut cols[3],
                        "Maturity",
                        &format!("{} blocks", n.coinbase_maturity),
                        INK,
                    );
                });
                ui.add_space(10.0);
                field_label(ui, "Tip hash");
                ui.label(body_text(&n.tip, 12.0).color(TEXT_BLUE).monospace());
                ui.add_space(8.0);
                field_label(ui, "Supply");
                ui.label(
                    body_text(
                        format!(
                            "Cap {} MESH · emitted {}",
                            n.supply_cap_mesh,
                            format_emitted(&n.emitted_atomic)
                        ),
                        13.0,
                    )
                    .color(TEXT_BLUE),
                );
                ui.add_space(10.0);
                ui.label(
                    body_text(
                        "This pulse is the same public getnodeinfo the explorer uses. Finality stays off until MESH_FINALITY_HEIGHT is armed on the seeds.",
                        12.0,
                    )
                    .color(MUTED),
                );
            });
    }

    fn ui_events(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            filter_btn(ui, &mut self.event_filter, EventFilter::All, "All");
            filter_btn(ui, &mut self.event_filter, EventFilter::Mine, "Mine");
            filter_btn(ui, &mut self.event_filter, EventFilter::Node, "Node");
            filter_btn(ui, &mut self.event_filter, EventFilter::Pay, "Pay");
            filter_btn(ui, &mut self.event_filter, EventFilter::Ai, "AI");
            filter_btn(ui, &mut self.event_filter, EventFilter::Err, "Errors");
            ui.add_space(8.0);
            if ghost_btn_enabled(ui, "Clear", !self.log.is_empty()).clicked() {
                self.log.clear();
            }
        });
        ui.add_space(8.0);
        let filter = self.event_filter;
        let shown: Vec<&LogLine> = self
            .log
            .iter()
            .filter(|line| match filter {
                EventFilter::All => true,
                EventFilter::Mine => matches!(line.src, EventSrc::Mine),
                EventFilter::Node => matches!(line.src, EventSrc::Node),
                EventFilter::Pay => matches!(line.src, EventSrc::Pay),
                EventFilter::Ai => matches!(line.src, EventSrc::Ai),
                EventFilter::Err => matches!(line.kind, LogKind::Err),
            })
            .collect();
        field_label(ui, &format!("Event list · {} / {}", shown.len(), self.log.len()));
        let log_h = ui.available_height().max(80.0);
        egui::Frame::new()
            .fill(FIELD_BG)
            .corner_radius(leaf_radius())
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ScrollArea::vertical()
                    .id_salt("event_list")
                    .max_height(log_h)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if shown.is_empty() {
                            ui.label(
                                body_text("Nothing in this filter yet.", 13.0).color(MUTED),
                            );
                        }
                        for line in shown {
                            paint_log_line(ui, line);
                        }
                    });
            });
    }
}

impl eframe::App for MinerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        if self.mining
            && !self.stopping
            && self
                .last_hashrate_at
                .is_some_and(|t| t.elapsed() > Duration::from_secs(45))
        {
            // Only clear after a long gap (stale-wave / exam / GBT). 10s zeroes a live tile.
            self.cpu_hashrate = 0.0;
            self.gpu_hashrate = 0.0;
            self.gpu_mix_ema = 0.0;
        }
        self.drain_pay();
        self.drain_node();
        self.maybe_poll_pay();
        self.maybe_poll_node();
        ctx.request_repaint_after(Duration::from_millis(
            if self.mining || self.stopping { 80 } else { 400 },
        ));
        let time = ctx.input(|i| i.time);

        let editable = !self.mining;

        // User-owned height: drag the cyan handle. No egui snap-back.
        let mut bottom_h = self.cfg.bottom_h.clamp(BOTTOM_H_MIN, BOTTOM_H_MAX);
        let mut save_h = false;
        egui::TopBottomPanel::bottom("miner_bottom")
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
                    if self.mining {
                        let label = if self.stopping { "Stopping…" } else { "Stop" };
                        let stop = ui.add_sized(
                            [108.0, 36.0],
                            egui::Button::new(
                                egui::RichText::new(label)
                                    .color(INK)
                                    .size(14.0)
                                    .strong(),
                            )
                            .fill(if self.stopping {
                                Color32::from_rgb(72, 32, 38)
                            } else {
                                Color32::from_rgb(150, 32, 48)
                            })
                            .stroke(egui::Stroke::new(1.0, DANGER))
                            .sense(egui::Sense::click()),
                        );
                        if stop.clicked() && !self.stopping {
                            self.stop_mining();
                        }
                    } else if primary_btn(ui, "Start", true).clicked() {
                        self.start_mining();
                    }
                    ui.add_space(14.0);
                    ui.label(
                        body_text(&self.status, 14.0)
                            .color(if self.status_ok { TEXT_BLUE } else { DANGER }),
                    );
                });
                ui.add_space(6.0);
                field_label(ui, "Recent · Events tab for the full list");
                let log_h = ui.available_height().max(36.0);
                egui::Frame::new()
                    .fill(FIELD_BG)
                    .corner_radius(leaf_radius())
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ScrollArea::vertical()
                            .id_salt("activity_log")
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
                                ui.label(display_text("Miner", 13.0).color(CYAN));
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
                            tab_btn(ui, &mut self.tab, MainTab::Mining, "Mining");
                            tab_btn(ui, &mut self.tab, MainTab::Node, "Node");
                            tab_btn(ui, &mut self.tab, MainTab::Events, "Events");
                        });

                        ui.add_space(10.0);

                        match self.tab {
                            MainTab::Node => self.ui_node(ui),
                            MainTab::Events => self.ui_events(ui),
                            MainTab::Mining => {
                        ScrollArea::vertical()
                            .id_salt("miner_main")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                        let fusion_hs = meshhash_cpu::hashrate_fusion(
                            self.cpu_hashrate,
                            self.gpu_mix_ema.max(self.gpu_hashrate),
                        );
                        let fusion_hr = if self.mining || fusion_hs > 0.0 {
                            format_hashrate(fusion_hs)
                        } else {
                            "—".into()
                        };
                        let cpu_hr = if self.mining || self.cpu_hashrate > 0.0 {
                            format_hashrate(self.cpu_hashrate)
                        } else {
                            "—".into()
                        };
                        let gpu_hr = if self.mining
                            && self.gpu_wanted
                            && self.gpu_mix_ema <= 0.0
                            && self.gpu_hashrate <= 0.0
                            && !self.stopping
                        {
                            "warming…".into()
                        } else if self.mining || self.gpu_hashrate > 0.0 {
                            format_hashrate(self.gpu_mix_ema.max(self.gpu_hashrate))
                        } else {
                            "—".into()
                        };
                        let exam_stat = if !self.gpu_wanted && !self.ai_active {
                            "—".to_string()
                        } else if self
                            .last_exam_at
                            .is_some_and(|t| t.elapsed() < Duration::from_secs(20))
                        {
                            "MATCH".to_string()
                        } else if self.mining && self.gpu_wanted {
                            "needed".to_string()
                        } else {
                            "—".to_string()
                        };
                        ui.columns(4, |cols| {
                            stat_col(&mut cols[0], "Fusion", &fusion_hr, CYAN);
                            stat_col(&mut cols[1], "CPU", &cpu_hr, TEXT_BLUE);
                            stat_col(&mut cols[2], "GPU", &gpu_hr, INK);
                            stat_col(
                                &mut cols[3],
                                "Blocks",
                                &self.blocks_found.to_string(),
                                OK,
                            );
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(body_text("GPU exam", 11.0).color(MUTED));
                            ui.label(body_text(&exam_stat, 13.0).color(WARN).strong());
                            ui.add_space(12.0);
                            ui.label(
                                body_text(
                                    "Fusion H/s is finished digests (one nonce). GPU path fills on CPU and seals — CPU/GPU tiles match; do not add them.",
                                    11.0,
                                )
                                .color(MUTED),
                            );
                        });
                        ui.add_space(8.0);
                        self.ui_mining_session(ui);
                        if let Some(pay) = &self.pay {
                            ui.add_space(10.0);
                            ui.label(display_text("PAID FOR", 10.5).color(MUTED));
                            ui.add_space(4.0);
                            if pay.by_lane.is_empty() {
                                ui.label(
                                    body_text(
                                        if pay.rewards.trim().is_empty()
                                            || pay.rewards.starts_with('0')
                                        {
                                            "No coinbase on this address yet. After a Fusion seal or exam MATCH, this lists Fusion seal / GPU work / nodes separately.".into()
                                        } else {
                                            format!(
                                                "Lifetime coinbase {} — lane labels need a current node (GET /v1/getrewards).",
                                                pay.rewards
                                            )
                                        },
                                        12.0,
                                    )
                                    .color(TEXT_BLUE),
                                );
                            } else {
                                ui.label(
                                    body_text(
                                        format!("Lifetime coinbase {}", pay.rewards),
                                        12.0,
                                    )
                                    .color(CYAN),
                                );
                                for lane in &pay.by_lane {
                                    ui.horizontal(|ui| {
                                        ui.label(body_text(&lane.title, 12.0).color(INK));
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    body_text(&lane.amount, 12.0)
                                                        .color(CYAN)
                                                        .strong(),
                                                );
                                            },
                                        );
                                    });
                                    ui.label(body_text(&lane.paid_for, 11.0).color(TEXT_BLUE));
                                }
                                if let Some(hit) = pay.recent.first() {
                                    ui.add_space(4.0);
                                    ui.label(
                                        body_text(
                                            format!(
                                                "Last: {} at #{}{} — {}",
                                                hit.amount,
                                                hit.height,
                                                format_unix_local(hit.timestamp)
                                                    .map(|t| format!(" · {t}"))
                                                    .unwrap_or_default(),
                                                hit.title
                                            ),
                                            12.0,
                                        )
                                        .color(OK),
                                    );
                                }
                            }
                        }
                        if self.mining && (self.cpu_hashrate > 0.0 || self.gpu_wanted) {
                            ui.add_space(8.0);
                            ui.label(
                                body_text(
                                    "GPU mixes the pad. CPU seals Fusion on the live tip. Both H/s tiles are finished Fusion hashes. Jobs abort when the tip moves. Research jobs do not pay extra MESH.",
                                    12.0,
                                )
                                .color(TEXT_BLUE),
                            );
                        }

                        ui.add_space(14.0);
                        dual_rail(ui);
                        ui.add_space(12.0);

                        let catalog = self.catalog.clone();
                        let only_cpu = catalog
                            .iter()
                            .all(|d| matches!(d.id, ComputeDevice::Cpu));

                        ScrollArea::vertical()
                            .id_salt("miner_config")
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                field_label(ui, "Mine target");
                                dark_edit(
                                    ui,
                                    &mut self.cfg.rpc,
                                    "https://eu.hashmonkeys.cloud",
                                    editable,
                                );
                                ui.add_space(10.0);
                                field_label(ui, "Worker name");
                                dark_edit(ui, &mut self.cfg.worker_name, "rig1", editable);
                                ui.add_space(10.0);

                                field_label(ui, "Your address");
                                dark_edit(ui, &mut self.cfg.address, "mesh01…", editable);
                                ui.add_space(10.0);

                                if ghost_btn_enabled(ui, "Save", editable).clicked() {
                                    match self.apply_fields() {
                                        Ok(()) => {
                                            save_config(&self.cfg_path, &self.cfg);
                                            self.status = "Saved".into();
                                            self.status_ok = true;
                                            self.push_log("Settings saved", LogKind::Ok);
                                        }
                                        Err(e) => {
                                            self.status = e.clone();
                                            self.status_ok = false;
                                            self.push_log(e, LogKind::Err);
                                        }
                                    }
                                }

                                ui.add_space(14.0);
                                field_label(ui, "Hardware roles");
                                ui.label(
                                    body_text(
                                        "Tick a GPU: one Fusion path (GPU wave + CPU seal). Research is on by default — exam MATCH is required to submit a block from height 39000, and brain jobs pay from the GPU helper floor.",
                                        12.0,
                                    )
                                    .color(TEXT_BLUE),
                                );
                                ui.add_enabled_ui(editable, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        let mut cpu = self.selected.contains(&ComputeDevice::Cpu);
                                        if pointer(ui.checkbox(&mut cpu, "CPU")).changed() {
                                            self.toggle_family_cpu(cpu);
                                        }
                                        let mut research = self.cfg.ai_research;
                                        if pointer(ui.checkbox(&mut research, "Research (pays MESH)")).changed()
                                        {
                                            self.cfg.ai_research = research;
                                            self.persist_selection();
                                            save_config(&self.cfg_path, &self.cfg);
                                        }
                                        let gpu_ids: Vec<ComputeDevice> = catalog
                                            .iter()
                                            .filter(|d| !matches!(d.id, ComputeDevice::Cpu))
                                            .map(|d| d.id)
                                            .collect();
                                        let all_gpus = !gpu_ids.is_empty()
                                            && gpu_ids.iter().all(|id| self.selected.contains(id));
                                        let mut all = all_gpus;
                                        if pointer(ui.checkbox(&mut all, "All GPUs")).changed() {
                                            self.select_all_gpus(all);
                                        }
                                    });
                                });

                                ui.add_space(6.0);
                                let list_w = field_width(ui);
                                egui::Frame::new()
                                    .fill(FIELD_BG)
                                    .inner_margin(egui::Margin::symmetric(10, 8))
                                    .corner_radius(4.0)
                                    .show(ui, |ui| {
                                        ui.set_width(list_w);
                                        ScrollArea::vertical()
                                            .id_salt("device_list")
                                            .max_height(96.0)
                                            .auto_shrink([false, true])
                                            .show(ui, |ui| {
                                                ui.add_enabled_ui(editable, |ui| {
                                                    for d in &catalog {
                                                        if matches!(d.id, ComputeDevice::Cpu) {
                                                            continue;
                                                        }
                                                        let mut on = self.selected.contains(&d.id);
                                                        let label = match d.id {
                                                            ComputeDevice::Cuda { index } => {
                                                                format!("NVIDIA GPU {index}")
                                                            }
                                                            ComputeDevice::OpenCl { .. } => {
                                                                format!(
                                                                    "{} · {}",
                                                                    d.family,
                                                                    short_device_name(&d.name)
                                                                )
                                                            }
                                                            ComputeDevice::Cpu => {
                                                                "CPU".into()
                                                            }
                                                        };
                                                        if pointer(
                                                            ui.checkbox(
                                                                &mut on,
                                                                body_text(label, 13.5).color(TEXT_BLUE),
                                                            ),
                                                        )
                                                        .changed()
                                                        {
                                                            if on {
                                                                self.selected.insert(d.id);
                                                            } else {
                                                                self.selected.remove(&d.id);
                                                            }
                                                            self.persist_selection();
                                                        }
                                                    }
                                                    if only_cpu {
                                                        ui.label(
                                                            body_text(
                                                                "No GPU found — CPU mines blocks only. Add a GPU for AI (GPU market).",
                                                                13.0,
                                                            )
                                                            .color(TEXT_BLUE),
                                                        );
                                                    }
                                                });
                                            });
                                    });
                            });
                    });
                            }
                        }
                    });
            });
    }
}

fn field_width(ui: &egui::Ui) -> f32 {
    // About half the panel — leave the right side for brand art.
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

fn short_device_name(name: &str) -> String {
    name.split('(').next().unwrap_or(name).trim().to_string()
}

fn format_emitted(atomic: &str) -> String {
    match atomic.parse::<u64>() {
        Ok(n) => mesh_types::Amount::from_atomic(n).to_string(),
        Err(_) => {
            if atomic.is_empty() {
                "—".into()
            } else {
                atomic.to_string()
            }
        }
    }
}

fn short_hash(h: &str) -> String {
    let t = h.trim();
    if t.len() <= 16 {
        return t.to_string();
    }
    format!("{}…{}", &t[..8], &t[t.len() - 6..])
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
                display_text(label, 13.0)
                    .color(if selected { INK } else { MUTED })
                    .strong(),
            );
        })
        .response
        .interact(egui::Sense::click());
    if pointer(resp).clicked() {
        *current = tab;
    }
}

fn filter_btn(ui: &mut egui::Ui, current: &mut EventFilter, filter: EventFilter, label: &str) {
    let selected = *current == filter;
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
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(
                display_text(label, 12.0)
                    .color(if selected { INK } else { MUTED })
                    .strong(),
            );
        })
        .response
        .interact(egui::Sense::click());
    if pointer(resp).clicked() {
        *current = filter;
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PaySnapshot {
    #[serde(default)]
    rewards: String,
    #[serde(default)]
    by_lane: Vec<PayLane>,
    #[serde(default)]
    recent: Vec<PayHit>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PayLane {
    #[serde(default)]
    title: String,
    #[serde(default)]
    paid_for: String,
    #[serde(default)]
    amount: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PayHit {
    #[serde(default)]
    height: u64,
    #[serde(default)]
    timestamp: u64,
    #[serde(default)]
    amount: String,
    #[serde(default)]
    title: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct NodePulse {
    #[serde(default)]
    height: u64,
    #[serde(default)]
    tip: String,
    #[serde(default)]
    genesis: String,
    #[serde(default)]
    next_difficulty: u32,
    #[serde(default)]
    peers: usize,
    #[serde(default)]
    finalized_height: u64,
    #[serde(default)]
    finality_active: bool,
    #[serde(default)]
    supply_cap_mesh: u64,
    #[serde(default)]
    emitted_atomic: String,
    #[serde(default)]
    coinbase_maturity: u64,
}

fn fetch_node_pulse(mine_rpc: &str) -> Option<NodePulse> {
    let mut urls = Vec::new();
    let mine = normalize_pool_url(mine_rpc);
    if !mine.is_empty() {
        urls.push(mine);
    }
    for u in mesh_types::default_rpc_urls() {
        if !urls.iter().any(|x| x == &u) {
            urls.push(u);
        }
    }
    for base in urls {
        let url = format!("{}/v1/getnodeinfo", base.trim_end_matches('/'));
        let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(6)).call() else {
            continue;
        };
        if !(200..300).contains(&resp.status()) {
            continue;
        }
        if let Ok(pulse) = resp.into_json::<NodePulse>() {
            if pulse.height > 0 || !pulse.tip.is_empty() {
                return Some(pulse);
            }
        }
    }
    None
}

fn fetch_pay_snapshot(mine_rpc: &str, address: &str) -> Option<PaySnapshot> {
    let addr = address.trim();
    if addr.is_empty() {
        return None;
    }
    let mut urls = Vec::new();
    let mine = normalize_pool_url(mine_rpc);
    if !mine.is_empty() {
        urls.push(mine);
    }
    for u in mesh_types::default_rpc_urls() {
        if !urls.iter().any(|x| x == &u) {
            urls.push(u);
        }
    }
    for base in urls {
        let url = format!(
            "{}/v1/getrewards?address={}",
            base.trim_end_matches('/'),
            addr
        );
        let Ok(resp) = ureq::get(&url)
            .timeout(Duration::from_secs(8))
            .call()
        else {
            continue;
        };
        if !(200..300).contains(&resp.status()) {
            continue;
        }
        if let Ok(snap) = resp.into_json::<PaySnapshot>() {
            return Some(snap);
        }
    }
    None
}

fn stat_col(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    tile().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(display_text(label.to_uppercase(), 10.5).color(MUTED));
        ui.add(
            egui::Label::new(display_text(value, 16.0).color(color).strong()).truncate(),
        );
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

fn is_race_log(msg: &str) -> bool {
    let low = msg.to_ascii_lowercase();
    (low.contains("submitblock") && low.contains("400"))
        || low.contains("stale block")
        || low.contains("tip moved")
        || low.contains("result http 409")
        || low.contains("stale shared brain")
}

/// Status line + log line. Only call a target "unreachable" for real transport failures.
fn format_miner_error(e: &str, pool: bool) -> (String, String) {
    let low = e.to_ascii_lowercase();
    if low.contains("nonce failed difficulty") {
        return (
            "GPU nonce failed CPU verify — skipped.".into(),
            format!("GPU candidate did not rematch on CPU ({e})"),
        );
    }
    let transport = low.contains("connection")
        || low.contains("timed out")
        || low.contains("timeout")
        || low.contains("dns")
        || low.contains("refused")
        || low.contains("unreachable")
        || low.contains("failed to lookup")
        || low.contains("no such host")
        || low.contains("http 502")
        || low.contains("http 503")
        || low.contains("http 504")
        || (low.contains("get ") && low.contains("->"));
    if transport && pool {
        return (
            "Pool unreachable — check Mine target (HTTPS preferred).".into(),
            format!(
                "Pool unreachable — use https://eu.hashmonkeys.cloud (HTTP/S, not stratum+tcp). ({e})"
            ),
        );
    }
    if transport {
        return (
            "Could not reach the node. Is it running?".into(),
            format!("Could not reach the node. Is it running? ({e})"),
        );
    }
    let status: String = e.chars().take(88).collect();
    (status, e.to_string())
}
