//! MonkeyMesh desktop — wallet, Fusion mine, optional local node (Windows all-in-one).

#![windows_subsystem = "windows"]

mod address_book;
mod chrome;
mod icons;
mod rpc;
mod seed_gate;
mod theme;
mod wallet_store;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};
use mesh_crypto::Keypair;
use qrcode::QrCode;

use address_book::{load_book, save_book, AddressBook};
use chrome::{hairline, paint_backdrop, status_dot};
use icons::{icon_btn, Icon};
use std::collections::HashSet;

use meshhash_cpu::{
    fusion_sequential_active, pow_fusion_sequential_height, pow_version_for_height,
};
use mesh_miner_gpu::{
    devices_status, enumerate_devices, format_hashrate, run_rpc_loop, ComputeDevice, DeviceInfo,
    MinerConfig, MinerEvent,
};
use mesh_types::Address;
use rpc::{NodeInfo, RewardsView, RpcClient, TxRow};
use seed_gate::SeedGate;
use theme::{
    ghost_btn, label_upper, panel, pointer, primary_btn, CYAN, CYAN_DIM, DANGER, INK, MUTED, OK,
    WARN,
};
use wallet_store::{
    reveal_mnemonic, resolve_rpc_candidates, resolve_vault_path, LoadedWallet, WalletKind,
};

const MASCOT_PNG: &[u8] = include_bytes!("../assets/mascot.png");
const COIN_PNG: &[u8] = include_bytes!("../assets/coin.png");
const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
const BACK_PNG: &[u8] = include_bytes!("../assets/wallet_back.png");

const WIN_W: f32 = 1100.0;
const WIN_H: f32 = 700.0;
const SIDE_W: f32 = 196.0;
const OFFICIAL_POOL: &str = "https://eu.hashmonkeys.cloud";
const LOCAL_NODE_RPC: &str = "http://127.0.0.1:18082";
const LOCAL_NODE_RPC_BIND: &str = "127.0.0.1:18082";
const LOCAL_NODE_P2P: &str = "0.0.0.0:39012";
const EVENT_CAP: usize = 200;

fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn find_local_node_exe() -> Option<PathBuf> {
    let dir = app_dir();
    let names = if cfg!(windows) {
        ["mesh-node.exe", "mesh-node"]
    } else {
        ["mesh-node", "mesh-node.exe"]
    };
    for name in names {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    let nested = dir.join("bin").join(if cfg!(windows) {
        "mesh-node.exe"
    } else {
        "mesh-node"
    });
    nested.is_file().then_some(nested)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Send,
    Receive,
    History,
    Mine,
    Network,
    Security,
}

#[derive(Clone, Copy)]
enum EventSrc {
    Mine,
    Node,
    Pay,
}

#[derive(Clone, Copy)]
enum EventKind {
    Info,
    Ok,
    Warn,
    Err,
}

struct EventLine {
    at: String,
    src: EventSrc,
    kind: EventKind,
    msg: String,
}

enum Job {
    Refresh,
    Send {
        to: String,
        amount: String,
        memo: String,
    },
}

enum JobResult {
    RefreshOk {
        info: NodeInfo,
        balance: String,
        spendable: Option<String>,
        txs: Vec<TxRow>,
        rewards: RewardsView,
    },
    RefreshErr(String),
    SendOk(String),
    SendErr(String),
}

struct MeshWalletApp {
    gate: Option<SeedGate>,
    page: Page,
    rpc_url: String,
    key: Option<Keypair>,
    mnemonic: Option<String>,
    address_book: AddressBook,
    wallet_kind: Option<WalletKind>,
    vault_path: String,
    address: String,
    balance: String,
    spendable: Option<String>,
    node: Option<NodeInfo>,
    txs: Vec<TxRow>,
    rewards: RewardsView,
    status: String,
    status_ok: bool,
    busy: bool,
    // Mining (in-wallet CPU + multi-GPU)
    mining: bool,
    mine_stop: Option<Arc<AtomicBool>>,
    mine_found: u64,
    mine_hashrate: f64,
    mine_cpu_hs: f64,
    mine_gpu_hs: f64,
    events: Vec<EventLine>,
    last_node_height: Option<u64>,
    logged_pay_height: Option<u64>,
    warned_v5: bool,
    mine_catalog: Vec<DeviceInfo>,
    mine_selected: HashSet<ComputeDevice>,
    mine_server: String,
    mine_batch: u32,
    mine_batch_str: String,
    mine_active_label: String,
    mine_tx: Sender<MinerEvent>,
    mine_rx: Receiver<MinerEvent>,
    local_node: Option<Child>,
    local_node_status: String,
    last_poll: Instant,
    to: String,
    amount: String,
    memo: String,
    rpc_edit: String,
    backup_password: String,
    revealed_seed: String,
    seed_ack: bool,
    pending_fresh_seed: Option<String>,
    mascot: Option<TextureHandle>,
    coin: Option<TextureHandle>,
    qr: Option<TextureHandle>,
    hero: Option<TextureHandle>,
    job_tx: Sender<(Job, String, Keypair)>,
    result_tx: Sender<JobResult>,
    result_rx: Receiver<JobResult>,
}

fn main() -> eframe::Result<()> {
    let rpc_url = RpcClient::pick_live(&resolve_rpc_candidates());

    let (job_tx, job_rx) = mpsc::channel::<(Job, String, Keypair)>();
    let (result_tx, result_rx) = mpsc::channel::<JobResult>();
    let (mine_tx, mine_rx) = mpsc::channel::<MinerEvent>();
    let worker_tx = result_tx.clone();
    thread::spawn(move || worker_loop(job_rx, worker_tx));

    let icon = load_icon_data(ICON_PNG);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WIN_W, WIN_H])
            .with_min_inner_size([1000.0, 640.0])
            .with_resizable(true)
            .with_title("MonkeyMesh")
            .with_icon(icon),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "MonkeyMesh",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let mine_catalog = enumerate_devices();
            let mut mine_selected = HashSet::new();
            if let Some(gpu) = mine_catalog
                .iter()
                .find(|d| !matches!(d.id, ComputeDevice::Cpu))
            {
                mine_selected.insert(gpu.id);
            } else {
                mine_selected.insert(ComputeDevice::Cpu);
            }
            let mut app = MeshWalletApp {
                gate: Some(SeedGate::new()),
                page: Page::Overview,
                rpc_url: rpc_url.clone(),
                key: None,
                mnemonic: None,
                address_book: AddressBook::default(),
                wallet_kind: None,
                vault_path: resolve_vault_path().display().to_string(),
                address: String::new(),
                balance: "—".into(),
                spendable: None,
                node: None,
                txs: vec![],
                rewards: RewardsView::default(),
                status: "Locked".into(),
                status_ok: false,
                busy: false,
                mining: false,
                mine_stop: None,
                mine_found: 0,
                mine_hashrate: 0.0,
                mine_cpu_hs: 0.0,
                mine_gpu_hs: 0.0,
                events: vec![],
                last_node_height: None,
                logged_pay_height: None,
                warned_v5: false,
                mine_catalog,
                mine_selected,
                mine_server: OFFICIAL_POOL.into(),
                local_node: None,
                local_node_status: String::new(),
                mine_batch: 256,
                mine_batch_str: "256".into(),
                mine_active_label: "Ready".into(),
                mine_tx,
                mine_rx,
                last_poll: Instant::now() - Duration::from_secs(10),
                to: String::new(),
                amount: String::new(),
                memo: String::new(),
                rpc_edit: rpc_url,
                backup_password: String::new(),
                revealed_seed: String::new(),
                seed_ack: false,
                pending_fresh_seed: None,
                mascot: None,
                coin: None,
                qr: None,
                hero: None,
                job_tx,
                result_tx,
                result_rx,
            };
            app.load_textures(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

fn worker_loop(rx: Receiver<(Job, String, Keypair)>, tx: Sender<JobResult>) {
    while let Ok((job, rpc, key)) = rx.recv() {
        let client = RpcClient::new(&rpc);
        let res = match job {
            Job::Refresh => match client.refresh(&key.address().to_string()) {
                Ok((info, balance, spendable, txs, rewards)) => JobResult::RefreshOk {
                    info,
                    balance,
                    spendable,
                    txs,
                    rewards,
                },
                Err(e) => JobResult::RefreshErr(e.to_string()),
            },
            Job::Send { to, amount, memo } => match client.send(&key, &to, &amount, &memo) {
                Ok(txid) => JobResult::SendOk(txid),
                Err(e) => JobResult::SendErr(e.to_string()),
            },
        };
        let _ = tx.send(res);
    }
}

impl MeshWalletApp {
    fn load_textures(&mut self, ctx: &egui::Context) {
        self.mascot = load_texture(ctx, "mascot", MASCOT_PNG, 64);
        self.coin = load_texture(ctx, "coin", COIN_PNG, 96);
        self.hero = load_texture(ctx, "hero", BACK_PNG, 1280);
        if !self.address.is_empty() {
            self.qr = make_qr_texture(ctx, &self.address);
        }
    }

    fn unlock_with(&mut self, loaded: LoadedWallet, ctx: &egui::Context) {
        self.pending_fresh_seed = loaded.fresh_mnemonic.clone();
        self.wallet_kind = Some(loaded.kind);
        self.vault_path = loaded.vault_path.display().to_string();
        self.mnemonic = loaded.mnemonic.clone();
        self.address = loaded.key.address().to_string();
        self.key = Some(loaded.key);
        self.qr = make_qr_texture(ctx, &self.address);

        let vault = PathBuf::from(&self.vault_path);
        match load_book(&vault) {
            Ok(mut book) => {
                book.ensure_index0(&self.address);
                let _ = save_book(&vault, &book);
                self.address_book = book;
                if let Some(active) = self.address_book.active().cloned() {
                    let _ = self.activate_address_index(active.index, ctx);
                }
            }
            Err(_) => {
                self.address_book = AddressBook::default();
                self.address_book.ensure_index0(&self.address);
            }
        }

        self.gate = None;
        self.status = "Connecting…".into();
        if self.pending_fresh_seed.is_some() {
            self.page = Page::Security;
            self.revealed_seed = self.pending_fresh_seed.clone().unwrap_or_default();
            self.seed_ack = false;
        }
        self.queue_refresh();
    }

    fn activate_address_index(&mut self, index: u32, ctx: &egui::Context) -> anyhow::Result<()> {
        let Some(phrase) = self.mnemonic.clone() else {
            // Legacy / no mnemonic — only index 0
            if index != 0 {
                anyhow::bail!("seed required to derive more addresses");
            }
            return Ok(());
        };
        let m = Keypair::mnemonic_from_phrase(&phrase)?;
        let key = Keypair::from_mnemonic_index(&m, "", index)?;
        self.address = key.address().to_string();
        self.key = Some(key);
        self.address_book.active_index = index;
        if let Some(e) = self
            .address_book
            .entries
            .iter_mut()
            .find(|e| e.index == index)
        {
            e.address = self.address.clone();
        }
        let _ = save_book(PathBuf::from(&self.vault_path).as_path(), &self.address_book);
        self.qr = make_qr_texture(ctx, &self.address);
        self.queue_refresh();
        Ok(())
    }

    fn generate_new_address(&mut self, ctx: &egui::Context) {
        let Some(phrase) = self.mnemonic.clone() else {
            self.status = "Seed vault required for new addresses".into();
            self.status_ok = false;
            return;
        };
        let index = self.address_book.next_index();
        match Keypair::mnemonic_from_phrase(&phrase)
            .and_then(|m| Keypair::from_mnemonic_index(&m, "", index))
        {
            Ok(key) => {
                let addr = key.address().to_string();
                self.address_book
                    .push(index, addr.clone(), format!("Address {index}"));
                let _ = save_book(PathBuf::from(&self.vault_path).as_path(), &self.address_book);
                self.key = Some(key);
                self.address = addr;
                self.qr = make_qr_texture(ctx, &self.address);
                self.status = format!("Generated address #{index}");
                self.status_ok = true;
                self.queue_refresh();
            }
            Err(e) => {
                self.status = e.to_string();
                self.status_ok = false;
            }
        }
    }

    fn queue_refresh(&mut self) {
        let Some(key) = self.key.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        self.busy = true;
        let _ = self
            .job_tx
            .send((Job::Refresh, self.rpc_url.clone(), key));
    }

    fn push_event(&mut self, src: EventSrc, kind: EventKind, msg: impl Into<String>) {
        self.events.push(EventLine {
            at: chrono::Local::now().format("%d %b %H:%M:%S").to_string(),
            src,
            kind,
            msg: msg.into(),
        });
        if self.events.len() > EVENT_CAP {
            let n = self.events.len() - EVENT_CAP;
            self.events.drain(0..n);
        }
    }

    fn note_node_pulse(&mut self, info: &NodeInfo) {
        if self.last_node_height != Some(info.height) {
            let prev = self.last_node_height;
            self.last_node_height = Some(info.height);
            if prev.is_some() {
                self.push_event(
                    EventSrc::Node,
                    EventKind::Info,
                    format!(
                        "Height {} · tip {} · diff {} · {} peer(s)",
                        info.height,
                        short(&info.tip, 8, 6),
                        info.next_difficulty,
                        info.peers
                    ),
                );
            }
        }
        let v5_at = pow_fusion_sequential_height();
        if !self.warned_v5 && !fusion_sequential_active(info.height) {
            let left = v5_at.saturating_sub(info.height);
            if left <= 2_000 {
                self.warned_v5 = true;
                self.push_event(
                    EventSrc::Node,
                    EventKind::Warn,
                    format!(
                        "Sequential Fusion (v5) in {left} blocks — hop this app before #{v5_at}. GPU required; official CPU-only refuses v5."
                    ),
                );
            }
        }
    }

    fn note_rewards(&mut self, rewards: &RewardsView) {
        if let Some(hit) = rewards.recent.first() {
            if self.logged_pay_height != Some(hit.height) {
                self.logged_pay_height = Some(hit.height);
                let title = if hit.title.is_empty() {
                    "coinbase"
                } else {
                    hit.title.as_str()
                };
                self.push_event(
                    EventSrc::Pay,
                    EventKind::Ok,
                    format!("Paid {} — {title} at #{}", hit.amount, hit.height),
                );
            }
        }
    }

    fn start_mining(&mut self) {
        if self.mining {
            return;
        }
        if self.address.is_empty() {
            self.status = "Unlock wallet first".into();
            self.status_ok = false;
            return;
        }

        self.mine_batch = self.mine_batch_str.trim().parse().unwrap_or(256).max(1);
        let server = self.mine_server.trim().trim_end_matches('/').to_string();
        if server.is_empty() {
            self.status = "Set mining server URL".into();
            self.status_ok = false;
            return;
        }
        let low = server.to_ascii_lowercase();
        if low.starts_with("stratum+tcp://") || low.starts_with("stratum://") {
            self.status = "Not stratum — use the official HTTPS pool or a node RPC".into();
            self.status_ok = false;
            return;
        }
        if !(low.starts_with("http://") || low.starts_with("https://")) {
            self.status = "Use https://eu.hashmonkeys.cloud or http://… node RPC".into();
            self.status_ok = false;
            return;
        }
        if self.mine_selected.is_empty() {
            self.status = "Select CPU and/or at least one GPU".into();
            self.status_ok = false;
            return;
        }
        let Some(addr) = Address::from_hex(&self.address) else {
            self.status = "bad payout address".into();
            self.status_ok = false;
            return;
        };

        let devices: Vec<ComputeDevice> = self.mine_selected.iter().copied().collect();
        let stop = Arc::new(AtomicBool::new(false));
        self.mine_stop = Some(stop.clone());
        self.mining = true;
        self.mine_found = 0;
        self.mine_hashrate = 0.0;
        self.mine_cpu_hs = 0.0;
        self.mine_gpu_hs = 0.0;
        self.mine_active_label = devices_status(&devices);
        self.status = format!("Mining… ({})", self.mine_active_label);
        self.status_ok = true;
        self.push_event(
            EventSrc::Mine,
            EventKind::Ok,
            format!("Started Fusion mine · {} · {server}", self.mine_active_label),
        );

        let cfg = MinerConfig::with_devices(
            server,
            addr,
            self.mine_batch,
            5_000_000,
            devices,
        );
        let tx = self.mine_tx.clone();
        thread::spawn(move || run_rpc_loop(cfg, stop, tx));
    }

    fn stop_mining(&mut self) {
        if let Some(stop) = &self.mine_stop {
            stop.store(true, Ordering::SeqCst);
        }
        self.mining = false;
        self.mine_hashrate = 0.0;
        self.mine_cpu_hs = 0.0;
        self.mine_gpu_hs = 0.0;
        self.status = "Mining stopped".into();
        self.push_event(EventSrc::Mine, EventKind::Warn, "Stop requested");
    }

    fn local_node_alive(&self) -> bool {
        self.local_node.is_some()
    }

    fn poll_local_node(&mut self) {
        let dead = match &mut self.local_node {
            Some(child) => match child.try_wait() {
                Ok(Some(st)) => Some(format!("Local node exited ({st})")),
                Ok(None) => None,
                Err(e) => Some(format!("Local node error: {e}")),
            },
            None => None,
        };
        if let Some(msg) = dead {
            self.local_node = None;
            self.local_node_status = msg;
        }
    }

    fn start_local_node(&mut self) {
        if self.local_node_alive() {
            return;
        }
        let Some(exe) = find_local_node_exe() else {
            self.local_node_status =
                "mesh-node.exe not found next to this app (re-stage the MonkeyMesh pack)".into();
            return;
        };
        let data = app_dir().join("data").join("local-node");
        if let Err(e) = std::fs::create_dir_all(&data) {
            self.local_node_status = format!("Cannot create node data dir: {e}");
            return;
        }
        let mut cmd = Command::new(&exe);
        cmd.arg("--chain")
            .arg(data.join("chain.bin"))
            .arg("serve")
            .arg("--listen")
            .arg(LOCAL_NODE_P2P)
            .arg("--rpc")
            .arg(LOCAL_NODE_RPC_BIND)
            .arg("--wallet")
            .arg(data.join("wallet.key"))
            .arg("--p2p-key")
            .arg(data.join("p2p.key"))
            .arg("--miner-key")
            .arg(data.join("miner.key"))
            .arg("--connect")
            .arg(mesh_types::default_seed_p2p())
            .current_dir(app_dir())
            .env("MESH_FORCE_RETARGET_INTERVAL", "15")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        match cmd.spawn() {
            Ok(child) => {
                self.local_node = Some(child);
                self.local_node_status = format!("Running · RPC {LOCAL_NODE_RPC}");
            }
            Err(e) => {
                self.local_node_status = format!("Failed to start node: {e}");
            }
        }
    }

    fn stop_local_node(&mut self) {
        if let Some(mut child) = self.local_node.take() {
            let _ = child.kill();
            let _ = child.wait();
            self.local_node_status = "Local node stopped".into();
        }
    }

    fn use_wallet_rpc(&mut self, url: String) {
        let url = url.trim().trim_end_matches('/').to_string();
        if url.is_empty() {
            return;
        }
        self.rpc_url = url.clone();
        self.rpc_edit = url;
        self.queue_refresh();
    }

    fn drain_mine_events(&mut self) {
        while let Ok(ev) = self.mine_rx.try_recv() {
            match ev {
                MinerEvent::Status(s) => {
                    if self.mining {
                        self.status = truncate(&s, 56);
                        self.status_ok = true;
                    }
                    if !s.is_empty() {
                        self.push_event(EventSrc::Mine, EventKind::Info, s);
                    }
                }
                MinerEvent::Hashrate { cpu_hs, gpu_hs } => {
                    if cpu_hs > 0.0 {
                        self.mine_cpu_hs = cpu_hs;
                    }
                    if gpu_hs > 0.0 {
                        self.mine_gpu_hs = gpu_hs;
                    }
                    if cpu_hs == 0.0 && gpu_hs == 0.0 {
                        self.mine_cpu_hs = 0.0;
                        self.mine_gpu_hs = 0.0;
                    }
                    self.mine_hashrate =
                        mesh_miner_gpu::MinerEvent::hashrate_fusion(self.mine_cpu_hs, self.mine_gpu_hs);
                }
                MinerEvent::BlockFound { height, id: _ } => {
                    self.mine_found += 1;
                    self.status = format!("Mined #{height} ({} found)", self.mine_found);
                    self.status_ok = true;
                    self.last_poll = Instant::now() - Duration::from_secs(3);
                    self.push_event(
                        EventSrc::Mine,
                        EventKind::Ok,
                        format!("Fusion sealed block #{height}"),
                    );
                }
                MinerEvent::AiJobDone { .. } | MinerEvent::AiStopped => {}
                MinerEvent::Error(e) => {
                    let low = e.to_ascii_lowercase();
                    let race = (low.contains("submitblock") && low.contains("400"))
                        || low.contains("stale")
                        || low.contains("tip moved");
                    if race {
                        // Normal multi-device race — keep status green / quiet.
                        if self.mining {
                            self.status_ok = true;
                        }
                    } else if self.mining {
                        self.status = truncate(&format!("Mine: {e}"), 48);
                        self.status_ok = false;
                        self.push_event(EventSrc::Mine, EventKind::Err, e);
                    }
                }
                MinerEvent::Stopped => {
                    self.mining = false;
                    self.mine_stop = None;
                    self.mine_hashrate = 0.0;
                    self.mine_cpu_hs = 0.0;
                    self.mine_gpu_hs = 0.0;
                    self.push_event(EventSrc::Mine, EventKind::Warn, "Miner stopped");
                }
            }
        }
    }

    fn drain(&mut self, ctx: &egui::Context) {
        self.drain_mine_events();
        while let Ok(res) = self.result_rx.try_recv() {
            match res {
                JobResult::RefreshOk {
                    info,
                    balance,
                    spendable,
                    txs,
                    rewards,
                } => {
                    self.busy = false;
                    self.note_node_pulse(&info);
                    self.note_rewards(&rewards);
                    self.node = Some(info);
                    self.balance = balance;
                    self.spendable = spendable;
                    self.txs = txs;
                    self.rewards = rewards;
                    if !self.mining {
                        self.status = "Online".into();
                    }
                    self.status_ok = true;
                    self.last_poll = Instant::now();
                }
                JobResult::RefreshErr(e) => {
                    self.busy = false;
                    if !self.mining {
                        self.status = truncate(&e, 42);
                        self.status_ok = false;
                    }
                    self.last_poll = Instant::now();
                }
                JobResult::SendOk(txid) => {
                    self.busy = false;
                    self.status = format!("Sent {}", short(&txid, 8, 4));
                    self.status_ok = true;
                    self.to.clear();
                    self.amount.clear();
                    self.memo.clear();
                    self.queue_refresh();
                }
                JobResult::SendErr(e) => {
                    self.busy = false;
                    self.status = truncate(&e, 42);
                    self.status_ok = false;
                }
            }
            ctx.request_repaint();
        }
    }
}

impl eframe::App for MeshWalletApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.gate.is_some() {
            let unlocked = {
                let gate = self.gate.as_mut().unwrap();
                gate.show(ctx, self.hero.as_ref(), self.mascot.as_ref())
            };
            if let Some(w) = unlocked {
                self.unlock_with(w, ctx);
            }
            return;
        }

        self.drain(ctx);
        self.poll_local_node();
        if !self.busy && self.last_poll.elapsed() > Duration::from_secs(2) {
            self.queue_refresh();
        }
        // Idle: poll cadence. Mining/busy: smoother UI. Avoid permanent 30fps burn.
        let repaint = if self.mining || self.busy {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(400)
        };
        ctx.request_repaint_after(repaint);
        let time = ctx.input(|i| i.time);

        egui::SidePanel::left("rail")
            .exact_width(SIDE_W)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(10, 16, 22, 250))
                    .stroke(egui::Stroke::new(1.0, theme::RULE))
                    .inner_margin(egui::Margin::symmetric(10, 12)),
            )
            .show(ctx, |ui| {
                self.draw_rail(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::BG0))
            .show(ctx, |ui| {
                paint_backdrop(ui, self.hero.as_ref(), time);
                egui::Frame::new()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(18, 14))
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        self.draw_header(ui);
                        ui.add_space(8.0);
                        hairline(ui);
                        ui.add_space(12.0);

                        let avail = ui.available_size();
                        ui.allocate_ui_with_layout(
                            avail,
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| match self.page {
                                Page::Overview => self.page_overview(ui),
                                Page::Send => self.page_send(ui),
                                Page::Receive => self.page_receive(ui),
                                Page::History => self.page_history(ui),
                                Page::Mine => self.page_mine(ui),
                                Page::Network => self.page_network(ui),
                                Page::Security => self.page_security(ui),
                            },
                        );
                    });
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_mining();
        self.stop_local_node();
    }
}

impl Drop for MeshWalletApp {
    fn drop(&mut self) {
        self.stop_local_node();
    }
}

impl MeshWalletApp {
    fn draw_rail(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(tex) = &self.mascot {
                ui.add(egui::Image::new(tex).fit_to_exact_size(Vec2::splat(40.0)));
            }
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("MonkeyMesh")
                        .color(CYAN)
                        .size(15.0)
                        .family(theme::ui_family())
                        .strong(),
                );
                ui.label(RichText::new("All-in-one").color(MUTED).size(11.5));
            });
        });

        ui.add_space(14.0);
        hairline(ui);
        ui.add_space(12.0);

        let items = [
            (Page::Overview, Icon::Overview, "Overview"),
            (Page::Send, Icon::Send, "Send"),
            (Page::Receive, Icon::Receive, "Receive"),
            (Page::History, Icon::History, "History"),
            (Page::Mine, Icon::Mine, "Mine"),
            (Page::Network, Icon::Network, "Network"),
            (Page::Security, Icon::Network, "Security"),
        ];
        for (page, ic, label) in items {
            if icon_btn(ui, ic, label, self.page == page).clicked() {
                self.page = page;
            }
            ui.add_space(2.0);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, self.status_ok, self.busy || self.mining);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    let (label, col) = if self.mining {
                        ("Mining", CYAN)
                    } else if self.status_ok {
                        ("Connected", OK)
                    } else {
                        ("Offline", DANGER)
                    };
                    ui.label(RichText::new(label).color(col).size(11.0).strong());
                    ui.label(RichText::new(&self.status).color(MUTED).size(10.5));
                });
            });
        });
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        let title = match self.page {
            Page::Overview => "Overview",
            Page::Send => "Send",
            Page::Receive => "Receive",
            Page::History => "History",
            Page::Mine => "Mine",
            Page::Network => "Network",
            Page::Security => "Security",
        };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(title)
                    .color(INK)
                    .size(20.0)
                    .family(theme::ui_family())
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_btn(ui, "Refresh", !self.busy).clicked() {
                    self.queue_refresh();
                }
                if let Some(n) = &self.node {
                    ui.label(
                        RichText::new(format!("Height {}", n.height))
                            .color(CYAN)
                            .size(12.5),
                    );
                }
            });
        });
    }

    fn page_overview(&mut self, ui: &mut egui::Ui) {
        panel().show(ui, |ui| {
            ui.set_min_height(88.0);
            ui.horizontal(|ui| {
                if let Some(tex) = &self.coin {
                    ui.add(egui::Image::new(tex).fit_to_exact_size(Vec2::splat(48.0)));
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    label_upper(ui, "Available balance");
                    ui.horizontal(|ui| {
                        let shown = self
                            .spendable
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .unwrap_or(self.balance.as_str());
                        ui.label(
                            RichText::new(shown)
                                .color(CYAN)
                                .size(26.0)
                                .family(theme::ui_family())
                                .strong(),
                        );
                        ui.label(RichText::new("MESH").color(CYAN_DIM).size(13.0));
                    });
                    if let Some(sp) = &self.spendable {
                        if *sp != self.balance && self.balance != "—" {
                            ui.label(
                                RichText::new(format!("Total {} (coinbase matures in 20 blocks)", self.balance))
                                    .color(MUTED)
                                    .size(11.0),
                            );
                        }
                    }
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(short(&self.address, 18, 10))
                            .monospace()
                            .color(MUTED)
                            .size(11.0),
                    );
                });
            });
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if primary_btn(ui, "Send", true).clicked() {
                self.page = Page::Send;
            }
            if ghost_btn(ui, "Receive").clicked() {
                self.page = Page::Receive;
            }
            if ghost_btn(ui, if self.mining { "Stop mine" } else { "Mine" }).clicked()
            {
                if self.mining {
                    self.stop_mining();
                } else {
                    self.page = Page::Mine;
                }
            }
            if ghost_btn(ui, "Copy").clicked() {
                ui.ctx().copy_text(self.address.clone());
                self.status = "Address copied".into();
                self.status_ok = true;
            }
        });

        ui.add_space(12.0);
        label_upper(ui, "Node pulse");
        ui.add_space(4.0);
        if let Some(n) = &self.node {
            ui.horizontal(|ui| {
                metric(ui, "Height", &n.height.to_string());
                metric(ui, "Diff", &n.next_difficulty.to_string());
                metric(ui, "Peers", &n.peers.to_string());
                metric(ui, "PoW", &pow_era_label(n.height));
                metric(
                    ui,
                    "Finality",
                    &if n.finality_active {
                        format!("#{}", n.finalized_height)
                    } else {
                        "off".into()
                    },
                );
            });
        } else {
            ui.label(
                RichText::new("Start Node launcher, then Refresh.")
                    .color(DANGER)
                    .size(12.5),
            );
        }

        ui.add_space(12.0);
        label_upper(ui, "Recent activity");
        ui.add_space(4.0);
        let h = ui.available_height().max(80.0);
        egui::ScrollArea::vertical()
            .max_height(h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.tx_rows(ui, 12);
            });
    }

    fn page_send(&mut self, ui: &mut egui::Ui) {
        panel().show(ui, |ui| {
            ui.set_width(ui.available_width().min(520.0));
            label_upper(ui, "From");
            ui.label(
                RichText::new(short(&self.address, 16, 10))
                    .monospace()
                    .color(CYAN)
                    .size(12.0),
            );
            ui.add_space(10.0);
            label_upper(ui, "To");
            field(ui, &mut self.to, "mesh01…", true);
            ui.add_space(10.0);
            label_upper(ui, "Amount");
            field(ui, &mut self.amount, "1.0", false);
            ui.add_space(10.0);
            label_upper(ui, "Memo");
            field(ui, &mut self.memo, "optional", true);
            ui.add_space(16.0);
            if primary_btn(ui, "Broadcast", !self.busy).clicked() {
                if let Some(key) = self.key.clone() {
                    self.busy = true;
                    let _ = self.job_tx.send((
                        Job::Send {
                            to: self.to.clone(),
                            amount: self.amount.clone(),
                            memo: self.memo.clone(),
                        },
                        self.rpc_url.clone(),
                        key,
                    ));
                }
            }
        });
    }

    fn page_receive(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.page_receive_inner(ui, &ctx);
            });
    }

    fn page_receive_inner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        panel().show(ui, |ui| {
            ui.horizontal(|ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(14, 22, 32))
                    .stroke(egui::Stroke::new(1.0, theme::RULE))
                    .corner_radius(8.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        if let Some(tex) = &self.qr {
                            ui.add(egui::Image::new(tex).fit_to_exact_size(Vec2::splat(180.0)));
                        } else {
                            ui.allocate_exact_size(Vec2::splat(180.0), egui::Sense::hover());
                            ui.label(RichText::new("QR unavailable").color(MUTED));
                        }
                    });
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    label_upper(ui, "Active address");
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(short(&self.address, 20, 12))
                            .monospace()
                            .color(CYAN)
                            .size(12.0),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if primary_btn(ui, "Copy address", true).clicked() {
                            ui.ctx().copy_text(self.address.clone());
                            self.status = "Address copied".into();
                            self.status_ok = true;
                        }
                        if self.mnemonic.is_some() {
                            if ghost_btn(ui, "Generate new address").clicked() {
                                self.generate_new_address(&ctx);
                            }
                        }
                    });
                    if self.mnemonic.is_none() {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("HD addresses require a BIP39 vault wallet.")
                                .color(MUTED)
                                .size(11.5),
                        );
                    }
                });
            });
        });

        ui.add_space(12.0);
        self.page_receive_pay(ui);

        if !self.address_book.entries.is_empty() {
            ui.add_space(12.0);
            label_upper(ui, "Your addresses");
            ui.add_space(4.0);
            let entries = self.address_book.entries.clone();
            let active = self.address_book.active_index;
            for e in entries {
                let selected = e.index == active;
                egui::Frame::new()
                    .fill(if selected {
                        Color32::from_rgba_unmultiplied(0, 245, 255, 28)
                    } else {
                        Color32::from_rgb(12, 20, 28)
                    })
                    .stroke(egui::Stroke::new(
                        1.0,
                        if selected {
                            CYAN_DIM
                        } else {
                            Color32::from_rgb(26, 42, 52)
                        },
                    ))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("#{}", e.index))
                                    .color(CYAN)
                                    .size(12.0)
                                    .strong(),
                            );
                            ui.label(RichText::new(&e.label).color(INK).size(12.5));
                            ui.label(
                                RichText::new(short(&e.address, 12, 8))
                                    .monospace()
                                    .color(MUTED)
                                    .size(11.5),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if !selected
                                    && ghost_btn(ui, "Use").clicked()
                                {
                                    if let Err(err) = self.activate_address_index(e.index, &ctx) {
                                        self.status = err.to_string();
                                        self.status_ok = false;
                                    }
                                }
                            });
                        });
                    });
                ui.add_space(4.0);
            }
        }
    }

    fn page_receive_pay(&self, ui: &mut egui::Ui) {
        label_upper(ui, "What you were paid for");
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Coinbase is 45% Fusion seal (CPU) / 45% GPU work / 10% nodes. Helpers share the GPU lane via exam MATCH. Immature outputs need 20 confirms.",
            )
            .color(MUTED)
            .size(11.5),
        );
        ui.add_space(8.0);
        if self.rewards.by_lane.is_empty() && self.rewards.recent.is_empty() {
            panel().show(ui, |ui| {
                ui.label(
                    RichText::new("No mining coinbase on this address yet. Mine to this receive address, then Refresh.")
                        .color(MUTED)
                        .size(12.5),
                );
            });
            return;
        }
        panel().show(ui, |ui| {
            if !self.rewards.rewards.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Lifetime coinbase").color(MUTED).size(11.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(&self.rewards.rewards)
                                .color(CYAN)
                                .size(16.0)
                                .strong(),
                        );
                    });
                });
                ui.add_space(6.0);
            }
            for lane in &self.rewards.by_lane {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(&lane.title)
                                .color(INK)
                                .size(12.5)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(&lane.paid_for)
                                .color(MUTED)
                                .size(11.0),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(&lane.amount).color(CYAN).size(13.0));
                    });
                });
                ui.add_space(6.0);
            }
        });
        if !self.rewards.recent.is_empty() {
            ui.add_space(10.0);
            label_upper(ui, "Recent pays");
            ui.add_space(4.0);
            for hit in self.rewards.recent.iter().take(12) {
                egui::Frame::new()
                    .fill(Color32::from_rgb(12, 20, 28))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(26, 42, 52)))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("#{}", hit.height))
                                    .color(MUTED)
                                    .monospace()
                                    .size(12.0),
                            );
                            if let Some(at) = format_unix_local(hit.timestamp) {
                                ui.label(
                                    RichText::new(at)
                                        .color(MUTED)
                                        .monospace()
                                        .size(11.5),
                                );
                            }
                            ui.label(
                                RichText::new(&hit.title)
                                    .color(CYAN)
                                    .size(12.5)
                                    .strong(),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(&hit.amount).color(INK).size(13.0));
                            });
                        });
                        ui.label(
                            RichText::new(&hit.paid_for)
                                .color(MUTED)
                                .size(11.0),
                        );
                        let mat = if hit.mature {
                            "spendable".to_string()
                        } else {
                            format!("immature · {}/20 confirms", hit.confirmations.min(20))
                        };
                        ui.label(RichText::new(mat).color(if hit.mature { OK } else { MUTED }).size(11.0));
                    });
                ui.add_space(4.0);
            }
        }
    }

    fn page_history(&mut self, ui: &mut egui::Ui) {
        let h = ui.available_height();
        egui::ScrollArea::vertical()
            .max_height(h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.tx_rows(ui, 100);
            });
    }

    fn page_mine(&mut self, ui: &mut egui::Ui) {
        let editable = !self.mining;
        let h = ui.available_height();
        egui::ScrollArea::vertical()
            .id_salt("wallet_mine")
            .max_height(h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
        panel().show(ui, |ui| {
            ui.set_width(ui.available_width().min(720.0));
            ui.label(
                RichText::new(
                    "Fusion mine: GPU mix + CPU seal on the live tip. Pay goes to your unlocked address.",
                )
                .color(MUTED)
                .size(12.0),
            );
            ui.add_space(8.0);

            {
                let fusion = if self.mining || self.mine_hashrate > 0.0 {
                    format_hashrate(self.mine_hashrate)
                } else {
                    "—".into()
                };
                ui.vertical(|ui| {
                    label_upper(ui, "Fusion");
                    ui.label(
                        RichText::new(fusion)
                            .color(CYAN)
                            .size(22.0)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(
                            "Finished Fusion hashes / s. GPU path: CPU and GPU tiles match — do not add them.",
                        )
                        .color(MUTED)
                        .size(11.0),
                    );
                });
            }
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    label_upper(ui, "CPU");
                    ui.label(
                        RichText::new(if self.mining || self.mine_cpu_hs > 0.0 {
                            format_hashrate(self.mine_cpu_hs)
                        } else {
                            "—".into()
                        })
                        .color(CYAN)
                        .size(20.0)
                        .strong(),
                    );
                });
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    label_upper(ui, "GPU");
                    ui.label(
                        RichText::new(if self.mining || self.mine_gpu_hs > 0.0 {
                            format_hashrate(self.mine_gpu_hs)
                        } else {
                            "—".into()
                        })
                        .color(INK)
                        .size(20.0)
                        .strong(),
                    );
                });
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    label_upper(ui, "Blocks found");
                    ui.label(
                        RichText::new(self.mine_found.to_string())
                            .color(OK)
                            .size(20.0)
                            .strong(),
                    );
                });
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    label_upper(ui, "Active");
                    ui.label(
                        RichText::new(&self.mine_active_label)
                            .color(CYAN)
                            .size(12.0)
                            .strong(),
                    );
                });
            });

            ui.add_space(8.0);
            self.ui_mine_session(ui);

            ui.add_space(10.0);
            label_upper(ui, "Mine with");
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal(|ui| {
                    let mut cpu = self.mine_selected.contains(&ComputeDevice::Cpu);
                    if pointer(ui.checkbox(&mut cpu, "CPU")).changed() {
                        if cpu {
                            self.mine_selected.insert(ComputeDevice::Cpu);
                        } else {
                            self.mine_selected.remove(&ComputeDevice::Cpu);
                        }
                    }
                    ui.add_space(10.0);
                    let gpu_ids: Vec<_> = self
                        .mine_catalog
                        .iter()
                        .filter(|d| !matches!(d.id, ComputeDevice::Cpu))
                        .map(|d| d.id)
                        .collect();
                    let all_gpus =
                        !gpu_ids.is_empty() && gpu_ids.iter().all(|id| self.mine_selected.contains(id));
                    let mut all = all_gpus;
                    if pointer(ui.checkbox(&mut all, "All GPUs")).changed() {
                        for id in &gpu_ids {
                            if all {
                                self.mine_selected.insert(*id);
                            } else {
                                self.mine_selected.remove(id);
                            }
                        }
                    }
                });
            });

            ui.add_space(4.0);
            let catalog = self.mine_catalog.clone();
            let only_cpu = catalog
                .iter()
                .all(|d| matches!(d.id, ComputeDevice::Cpu));
            egui::Frame::new()
                .fill(Color32::from_rgb(8, 16, 22))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(28, 50, 60)))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .max_height(110.0)
                        .show(ui, |ui| {
                            ui.add_enabled_ui(editable, |ui| {
                                for d in &catalog {
                                    if matches!(d.id, ComputeDevice::Cpu) {
                                        continue;
                                    }
                                    let mut on = self.mine_selected.contains(&d.id);
                                    let label = format!("[{}] {}", d.family, d.name);
                                    if pointer(ui.checkbox(&mut on, label)).changed() {
                                        if on {
                                            self.mine_selected.insert(d.id);
                                        } else {
                                            self.mine_selected.remove(&d.id);
                                        }
                                    }
                                }
                                if only_cpu {
                                    ui.label(
                                        RichText::new("No GPU detected — CPU only.")
                                            .color(MUTED)
                                            .size(12.0),
                                    );
                                }
                            });
                        });
                });

            ui.add_space(8.0);
            label_upper(ui, "Mine target");
            ui.label(
                RichText::new(
                    "Official pool is HTTPS (not stratum). Wallet RPC is only for balances and send.",
                )
                .color(MUTED)
                .size(11.0),
            );
            ui.add_enabled(
                editable,
                egui::TextEdit::singleline(&mut self.mine_server)
                    .desired_width(ui.available_width())
                    .hint_text(OFFICIAL_POOL),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if theme::ghost_btn_enabled(ui, "Official pool", editable).clicked() {
                    self.mine_server = OFFICIAL_POOL.into();
                }
                if theme::ghost_btn_enabled(ui, "Use wallet RPC", editable).clicked() {
                    self.mine_server = self.rpc_url.clone();
                }
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    label_upper(ui, "Batch");
                    ui.add_enabled(
                        editable,
                        egui::TextEdit::singleline(&mut self.mine_batch_str).desired_width(80.0),
                    );
                });
            });

            ui.add_space(8.0);
            label_upper(ui, "Payout (active wallet)");
            ui.label(
                RichText::new(&self.address)
                    .monospace()
                    .color(CYAN)
                    .size(12.0),
            );

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if self.mining {
                    if primary_btn(ui, "Stop mining", true).clicked() {
                        self.stop_mining();
                    }
                } else if primary_btn(ui, "Start mining", true).clicked() {
                    self.start_mining();
                }
            });

            ui.add_space(12.0);
            self.ui_event_list(ui, 180.0);
        });
            });
    }

    fn ui_mine_session(&self, ui: &mut egui::Ui) {
        let height = self.node.as_ref().map(|n| n.height);
        let tip = self
            .node
            .as_ref()
            .map(|n| short(&n.tip, 8, 6))
            .unwrap_or_else(|| "—".into());
        ui.horizontal(|ui| {
            metric(
                ui,
                "Height",
                &height.map(|h| h.to_string()).unwrap_or_else(|| "—".into()),
            );
            metric(
                ui,
                "PoW",
                &height
                    .map(pow_era_label)
                    .unwrap_or_else(|| "—".into()),
            );
            metric(ui, "Tip", &tip);
        });
        if let Some(h) = height {
            if !fusion_sequential_active(h) {
                let v5_at = pow_fusion_sequential_height();
                ui.label(
                    RichText::new(format!(
                        "Live tip is Fusion v4. Sequential v5 starts at #{v5_at}. Official CPU-only miners refuse v5 — this app needs a GPU from that height."
                    ))
                    .color(MUTED)
                    .size(11.0),
                );
            }
        }
    }

    fn ui_event_list(&mut self, ui: &mut egui::Ui, max_h: f32) {
        ui.horizontal(|ui| {
            label_upper(ui, "Event list");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::ghost_btn_enabled(ui, "Clear", !self.events.is_empty()).clicked() {
                    self.events.clear();
                }
            });
        });
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(8, 16, 22))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(28, 50, 60)))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("wallet_events")
                    .max_height(max_h)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if self.events.is_empty() {
                            ui.label(
                                RichText::new("Nothing yet — start mining or wait for a node refresh.")
                                    .color(MUTED)
                                    .size(12.0),
                            );
                        }
                        for line in &self.events {
                            paint_event_line(ui, line);
                        }
                    });
            });
    }

    fn page_network(&mut self, ui: &mut egui::Ui) {
        let h = ui.available_height();
        egui::ScrollArea::vertical()
            .id_salt("wallet_network")
            .max_height(h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
        panel().show(ui, |ui| {
            ui.set_width(ui.available_width().min(640.0));
            label_upper(ui, "Wallet RPC");
            ui.label(
                RichText::new(
                    "Balances and send use this URL. Mining uses the Mine tab target (official pool by default).",
                )
                .color(MUTED)
                .size(11.0),
            );
            field(ui, &mut self.rpc_edit, "http://seednode.hashmonkeys.cloud:18080", true);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if primary_btn(ui, "Save", !self.busy).clicked() {
                    self.use_wallet_rpc(self.rpc_edit.clone());
                }
                if ghost_btn(ui, "Official seed").clicked() {
                    self.use_wallet_rpc(mesh_types::default_seed_rpc_url());
                }
                if ghost_btn(ui, "Official edge").clicked() {
                    self.use_wallet_rpc(mesh_types::default_edge_rpc_url());
                }
                if ghost_btn(ui, "Explorer").clicked() {
                    let _ = open::that(format!("{}/", self.rpc_url));
                }
            });
            ui.add_space(12.0);
            label_upper(ui, "Connected node");
            ui.label(
                RichText::new(&self.rpc_url)
                    .monospace()
                    .color(CYAN)
                    .size(12.0),
            );
            ui.add_space(6.0);
            if let Some(n) = &self.node {
                ui.horizontal(|ui| {
                    metric(ui, "Height", &n.height.to_string());
                    metric(ui, "Difficulty", &n.next_difficulty.to_string());
                    metric(ui, "Peers", &n.peers.to_string());
                    metric(
                        ui,
                        "Finality",
                        &if n.finality_active {
                            format!("#{}", n.finalized_height)
                        } else {
                            "off".into()
                        },
                    );
                });
                ui.horizontal(|ui| {
                    metric(ui, "PoW", &pow_era_label(n.height));
                    metric(ui, "Tip", &short(&n.tip, 8, 6));
                    if !n.genesis.is_empty() {
                        metric(ui, "Genesis", &short(&n.genesis, 8, 6));
                    }
                    if n.coinbase_maturity > 0 {
                        metric(ui, "Maturity", &format!("{} blocks", n.coinbase_maturity));
                    }
                });
                if n.supply_cap_mesh > 0 {
                    ui.label(
                        RichText::new(format!(
                            "Cap {} MESH · emitted {}",
                            n.supply_cap_mesh,
                            format_emitted(&n.emitted_atomic)
                        ))
                        .color(MUTED)
                        .size(12.0),
                    );
                }
                if !n.tip.is_empty() {
                    ui.label(
                        RichText::new(&n.tip)
                            .monospace()
                            .color(MUTED)
                            .size(11.0),
                    );
                }
            } else {
                ui.label(
                    RichText::new("No pulse yet — unlock and wait for Refresh, or Save a reachable seed/edge.")
                        .color(MUTED)
                        .size(12.0),
                );
            }
            ui.add_space(16.0);
            label_upper(ui, "Local node");
            ui.label(
                RichText::new(
                    "Optional replica beside this app. Uses ports 18082 / 39012 so it does not collide with the standalone Node pack.",
                )
                .color(MUTED)
                .size(11.0),
            );
            if let Some(local_h) = local_node_snap_height() {
                let public_h = self.node.as_ref().map(|n| n.height).unwrap_or(0);
                if public_h == 0 || public_h.saturating_sub(local_h) > 64 {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "Local replica data is height {local_h}{}. Do not Use local RPC until this sidecar catches up — wallet and mine stay on the seed / official pool.",
                            if public_h > 0 {
                                format!("; wallet node is #{public_h}")
                            } else {
                                String::new()
                            }
                        ))
                        .color(WARN)
                        .size(12.0),
                    );
                }
            }
            if find_local_node_exe().is_none() {
                ui.label(
                    RichText::new("mesh-node.exe is not next to this app — use the MonkeyMesh Windows pack.")
                        .color(MUTED)
                        .size(12.0),
                );
            } else {
                let running = self.local_node_alive();
                ui.label(
                    RichText::new(if self.local_node_status.is_empty() {
                        if running {
                            "Running"
                        } else {
                            "Stopped"
                        }
                    } else {
                        self.local_node_status.as_str()
                    })
                    .color(if running { OK } else { MUTED })
                    .size(12.0),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if running {
                        if primary_btn(ui, "Stop local node", true).clicked() {
                            self.stop_local_node();
                        }
                    } else if primary_btn(ui, "Start local node", true).clicked() {
                        self.start_local_node();
                    }
                    if ghost_btn(ui, "Use local RPC").clicked() {
                        self.use_wallet_rpc(LOCAL_NODE_RPC.into());
                    }
                });
            }
            ui.add_space(16.0);
            label_upper(ui, "Encrypted vault");
            ui.label(
                RichText::new(&self.vault_path)
                    .monospace()
                    .color(MUTED)
                    .size(11.5),
            );
            let kind = match self.wallet_kind {
                Some(WalletKind::Vault) => "BIP39 vault (encrypted)",
                Some(WalletKind::LegacyHex) => "LEGACY plaintext key — migrate in Security",
                None => "locked",
            };
            ui.label(RichText::new(kind).color(CYAN).size(12.0));
            ui.add_space(8.0);
            if ghost_btn(ui, "Lock wallet").clicked() {
                self.stop_mining();
                self.key = None;
                self.mnemonic = None;
                self.address.clear();
                self.address_book = AddressBook::default();
                self.revealed_seed.clear();
                self.pending_fresh_seed = None;
                self.gate = Some(SeedGate::new());
                self.status = "Locked".into();
                self.status_ok = false;
            }
            ui.add_space(14.0);
            self.ui_event_list(ui, 160.0);
        });
            });
    }

    fn page_security(&mut self, ui: &mut egui::Ui) {
        panel().show(ui, |ui| {
            ui.set_width(ui.available_width().min(640.0));
            ui.label(
                RichText::new("Seed phrase backup")
                    .color(CYAN)
                    .size(16.0)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Your 24-word BIP39 seed is the only way to recover this wallet. Write it on paper, store offline, never screenshot or share it.",
                )
                .color(MUTED)
                .size(12.5),
            );
            ui.add_space(12.0);

            if let Some(seed) = &self.pending_fresh_seed {
                if self.revealed_seed.is_empty() {
                    self.revealed_seed = seed.clone();
                }
                ui.label(
                    RichText::new("NEW WALLET — back up these words now")
                        .color(DANGER)
                        .size(12.0)
                        .strong(),
                );
            }

            if !self.revealed_seed.is_empty() {
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(6, 12, 16))
                    .stroke(egui::Stroke::new(1.0, CYAN_DIM))
                    .corner_radius(6.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(&self.revealed_seed)
                                .monospace()
                                .color(INK)
                                .size(13.0),
                        );
                    });
                ui.add_space(8.0);
                ui.checkbox(&mut self.seed_ack, "I wrote these words down offline");
                if self.seed_ack {
                    if primary_btn(ui, "Hide seed & continue", true).clicked() {
                        self.revealed_seed.clear();
                        self.pending_fresh_seed = None;
                        self.seed_ack = false;
                        self.page = Page::Overview;
                    }
                }
            } else {
                label_upper(ui, "Vault password to reveal seed");
                ui.add(
                    egui::TextEdit::singleline(&mut self.backup_password)
                        .password(true)
                        .desired_width(280.0),
                );
                ui.add_space(8.0);
                if primary_btn(ui, "Reveal seed phrase", true).clicked() {
                    match reveal_mnemonic(&self.backup_password) {
                        Ok(phrase) => {
                            self.revealed_seed = phrase;
                            self.backup_password.clear();
                            self.status = "Seed revealed — keep private".into();
                            self.status_ok = true;
                        }
                        Err(e) => {
                            self.status = e.to_string();
                            self.status_ok = false;
                        }
                    }
                }
            }

            ui.add_space(16.0);
            label_upper(ui, "Cryptography");
            ui.label(
                RichText::new(
                    "Industry-standard recovery stack (same idea as hardware wallets), adapted for Ed25519:",
                )
                .color(MUTED)
                .size(12.0),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "• BIP39 24 words — human backup of 256-bit entropy (paper / offline)\n\
                     • SLIP-0010 HD path m/44'/999778'/0'/0'/N' — Ed25519 derivation (BIP32 is secp256k1-only)\n\
                     • Coin type 999778 — provisional MESH id until registered\n\
                     • Argon2id (RFC 9106, 64 MiB) + XChaCha20-Poly1305 — password encrypts the vault; raw key never stored\n\
                     • NIST SP 800-63B-4 — 15+ character passphrase, no composition rules",
                )
                .color(MUTED)
                .size(11.5),
            );
        });
    }

    fn tx_rows(&self, ui: &mut egui::Ui, limit: usize) {
        if self.txs.is_empty() {
            ui.label(RichText::new("No transactions yet.").color(MUTED).size(13.0));
            return;
        }
        for tx in self.txs.iter().rev().take(limit) {
            let mine: Vec<_> = tx
                .outputs
                .iter()
                .filter(|o| o.address == self.address)
                .collect();
            egui::Frame::new()
                .fill(Color32::from_rgb(12, 20, 28))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(26, 42, 52)))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (tag, col) = if tx.in_mempool {
                            ("pool", CYAN)
                        } else if tx.memo.starts_with("pomc:") {
                            ("mine", OK)
                        } else {
                            ("ok", OK)
                        };
                        ui.label(RichText::new(tag).color(col).size(11.0).strong());
                        let h = tx
                            .height
                            .map(|h| format!("#{h}"))
                            .unwrap_or_else(|| "—".into());
                        ui.label(RichText::new(h).color(MUTED).monospace().size(12.0));
                        if let Some(at) = tx.timestamp.and_then(format_unix_local) {
                            ui.label(
                                RichText::new(at)
                                    .color(MUTED)
                                    .monospace()
                                    .size(11.5),
                            );
                        }
                        ui.label(
                            RichText::new(short(&tx.txid, 10, 6))
                                .color(CYAN)
                                .monospace()
                                .size(12.0),
                        );
                    });
                    if mine.is_empty() {
                        let amt = tx
                            .outputs
                            .first()
                            .map(|o| o.amount.as_str())
                            .unwrap_or("—");
                        ui.label(RichText::new(amt).color(INK).size(13.0));
                    } else {
                        for o in mine {
                            ui.horizontal(|ui| {
                                let title = o
                                    .title
                                    .as_deref()
                                    .filter(|s| !s.is_empty())
                                    .unwrap_or(if tx.memo.starts_with("pomc:") {
                                        "Block reward"
                                    } else {
                                        "Received"
                                    });
                                ui.label(RichText::new(title).color(MUTED).size(11.5));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(&o.amount).color(INK).size(13.0));
                                });
                            });
                            if let Some(why) = o.paid_for.as_deref().filter(|s| !s.is_empty()) {
                                ui.label(RichText::new(why).color(MUTED).size(10.5));
                            }
                        }
                    }
                });
            ui.add_space(4.0);
        }
    }
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

fn pow_era_label(height: u64) -> String {
    let ver = pow_version_for_height(height);
    if fusion_sequential_active(height) {
        format!("v{ver} sequential")
    } else {
        let left = pow_fusion_sequential_height().saturating_sub(height);
        format!("v{ver} · v5 in {left}")
    }
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

fn local_node_snap_height() -> Option<u64> {
    let p = app_dir().join("data").join("local-node").join("chain.snap.json");
    let raw = std::fs::read_to_string(p).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("height")?.as_u64()
}

fn paint_event_line(ui: &mut egui::Ui, line: &EventLine) {
    let col = match line.kind {
        EventKind::Info => INK,
        EventKind::Ok => OK,
        EventKind::Warn => WARN,
        EventKind::Err => DANGER,
    };
    let (tag, tag_col) = match line.src {
        EventSrc::Mine => ("MINE", CYAN),
        EventSrc::Node => ("NODE", INK),
        EventSrc::Pay => ("PAY", OK),
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(&line.at).color(MUTED).size(11.0).monospace());
        ui.label(
            RichText::new(tag)
                .color(tag_col)
                .size(11.0)
                .strong()
                .monospace(),
        );
        ui.label(RichText::new(&line.msg).color(col).size(12.0));
    });
}

fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(14, 22, 32, 236))
        .stroke(egui::Stroke::new(1.0, theme::RULE))
        .corner_radius(theme::leaf_radius())
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_width(96.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(label).color(MUTED).size(10.5));
                ui.label(
                    RichText::new(value)
                        .color(CYAN)
                        .size(16.0)
                        .family(theme::ui_family())
                        .strong(),
                );
            });
        });
    ui.add_space(8.0);
}

fn field(ui: &mut egui::Ui, value: &mut String, hint: &str, wide: bool) {
    let w = if wide {
        ui.available_width()
    } else {
        160.0
    };
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(w)
            .hint_text(hint)
            .margin(egui::Margin::symmetric(10, 8)),
    );
}

fn short(s: &str, head: usize, tail: usize) -> String {
    if s.len() <= head + tail + 1 {
        s.to_string()
    } else {
        format!("{}…{}", &s[..head], &s[s.len() - tail..])
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
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

fn make_qr_texture(ctx: &egui::Context, data: &str) -> Option<TextureHandle> {
    let code = QrCode::new(data.as_bytes()).ok()?;
    let n = code.width();
    let scale = 6usize;
    let out = n * scale;
    let mut px = vec![0u8; out * out * 4];
    for y in 0..n {
        for x in 0..n {
            let on = code[(x, y)] == qrcode::Color::Dark;
            let c = if on {
                [0u8, 220, 240, 255]
            } else {
                [8u8, 14, 18, 255]
            };
            for dy in 0..scale {
                for dx in 0..scale {
                    let i = ((y * scale + dy) * out + x * scale + dx) * 4;
                    px[i..i + 4].copy_from_slice(&c);
                }
            }
        }
    }
    Some(ctx.load_texture(
        "qr",
        egui::ColorImage::from_rgba_unmultiplied([out, out], &px),
        Default::default(),
    ))
}

fn load_icon_data(png: &[u8]) -> egui::IconData {
    let img = image::load_from_memory(png)
        .map(|i| i.to_rgba8())
        .unwrap_or_else(|_| image::RgbaImage::from_pixel(32, 32, image::Rgba([0, 220, 240, 255])));
    let width = img.width();
    let height = img.height();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}
