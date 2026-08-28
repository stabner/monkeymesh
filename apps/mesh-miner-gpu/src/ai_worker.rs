//! Embedded AI / research worker (GPU market) — shared network brain + protocol sims.
//!
//! Smart client: keep-alive HTTP, weight cache, re-advertise on unknown worker,
//! and pullers sized to VRAM train_slots so CUDA research can saturate.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use mesh_ai::{
    is_leg_train, is_ml_train_shared, is_quantum_train, parse_leg_job, parse_ml_train_shared_input,
    parse_quantum_job, run_benchmark, run_leg_train, run_ml_train_job, run_ml_train_shared,
    run_protocol_eval, run_quantum_train, train_slots_for_vram,
};
use mesh_ai_v2::{
    cuda_brain_available, is_ml_train_shared_v2, parse_job as parse_v2_job, run_job_prefer_cuda,
    BRAIN_CONTRACT,
};
use serde::Deserialize;

use crate::engine::MinerEvent;

fn resolve_ai_token() -> Option<String> {
    if let Ok(token) = std::env::var("MESH_AI_TOKEN") {
        let t = token.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    let candidates = [
        std::path::PathBuf::from("data/ai.token"),
        std::path::PathBuf::from("../Node/data/ai.token"),
        std::path::PathBuf::from("../../Launchers/Node/data/ai.token"),
    ];
    for p in candidates {
        if let Ok(s) = std::fs::read_to_string(&p) {
            let t = s.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn with_ai_token(mut req: ureq::Request) -> ureq::Request {
    if let Some(t) = resolve_ai_token() {
        req = req.set("X-Mesh-Token", &t);
    }
    req
}

#[derive(Deserialize)]
struct JobResp {
    job_id: String,
    kind: String,
    input_hex: String,
}

#[derive(Deserialize)]
struct ModelResp {
    epoch: u64,
    weights_hex: String,
}

#[derive(Deserialize)]
struct ResultOk {
    #[serde(default)]
    brain_epoch: Option<u64>,
    #[serde(default)]
    brain_v2_epoch: Option<u64>,
}

#[derive(Clone)]
struct ModelCache {
    epoch: u64,
    ver: u32,
    /// Empty = MNIST shared; otherwise leg name.
    leg: String,
    weights: Vec<u8>,
}

/// Hardware capacity advertised to the node AI board.
#[derive(Clone, Debug)]
pub struct AiCapacity {
    pub gpu_name: String,
    pub vram_mb: u32,
    pub train_slots: u32,
}

impl AiCapacity {
    pub fn from_vram(gpu_name: impl Into<String>, vram_bytes: u64) -> Self {
        let vram_mb = (vram_bytes / (1024 * 1024)).max(1) as u32;
        // Cap advertised slots so the seed board does not dump a 3090-sized queue.
        let train_slots = train_slots_for_vram(vram_mb).min(2);
        Self {
            gpu_name: gpu_name.into(),
            vram_mb,
            train_slots,
        }
    }

    pub fn fallback() -> Self {
        Self::from_vram("miner-gui", 8 * 1024 * 1024 * 1024)
    }

    /// Local HTTP pullers — keep this at 1–2 so Fusion mix is not vacuumed off the card.
    pub fn local_pullers(&self) -> u32 {
        self.train_slots.clamp(1, 2)
    }
}

/// Advertise + pull AI jobs until `stop` is set.
pub fn run_ai_loop(
    orch: String,
    address: String,
    capacity: AiCapacity,
    stop: Arc<AtomicBool>,
    tx: Sender<MinerEvent>,
) {
    let urls = resolve_ai_orch_urls(&orch, &address);
    if urls.is_empty() {
        let _ = tx.send(MinerEvent::Error("AI orch URL empty".into()));
        let _ = tx.send(MinerEvent::AiStopped);
        return;
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(1))
        .timeout_read(Duration::from_secs(120))
        .max_idle_connections(4)
        .build();

    let mut orch_i = 0usize;
    let mut advertised = false;
    for (i, u) in urls.iter().enumerate() {
        match advertise(&agent, u, &address, &capacity) {
            Ok(()) => {
                orch_i = i;
                advertised = true;
                break;
            }
            Err(e) => {
                let _ = tx.send(MinerEvent::Status(format!("AI advertise {u} failed: {e}")));
            }
        }
    }
    if !advertised {
        let _ = tx.send(MinerEvent::Error("AI advertise failed on all orch URLs".into()));
        let _ = tx.send(MinerEvent::AiStopped);
        return;
    }
    let orch = urls[orch_i].clone();
    let pullers = capacity.local_pullers();
    let _ = tx.send(MinerEvent::Status(format!(
        "AI online · {} · {} MB · advertise {} slots · {} local pullers · {}",
        capacity.gpu_name, capacity.vram_mb, capacity.train_slots, pullers, orch
    )));

    let cache = Arc::new(Mutex::new(None::<ModelCache>));
    let orch_idx = Arc::new(AtomicUsize::new(orch_i));
    let urls = Arc::new(urls);
    let capacity = Arc::new(capacity);
    let mut handles = Vec::with_capacity(pullers as usize);
    for slot in 0..pullers {
        let agent = agent.clone();
        let urls = urls.clone();
        let orch_idx = orch_idx.clone();
        let address = address.clone();
        let stop = stop.clone();
        let tx = tx.clone();
        let cache = cache.clone();
        let capacity = capacity.clone();
        handles.push(thread::spawn(move || {
            worker_slot(agent, urls, orch_idx, address, stop, tx, cache, slot, capacity);
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    let _ = tx.send(MinerEvent::AiStopped);
}

fn resolve_ai_orch_urls(orch: &str, address: &str) -> Vec<String> {
    let mut urls = mesh_types::parse_rpc_list(orch);
    if urls.is_empty() {
        urls = mesh_types::default_rpc_urls();
    } else {
        for u in mesh_types::default_rpc_urls() {
            if !urls.iter().any(|x| x == &u) {
                urls.push(u);
            }
        }
    }

    // Discover reachable shard map from health/nodeinfo (field miners rarely set MESH_AI_SHARD_*).
    let mut discovered: Option<(u32, Vec<String>)> = None;
    for base in urls.iter() {
        if let Some(d) = discover_ai_shards(base) {
            discovered = Some(d);
            break;
        }
    }
    let (count, shard_urls) = match discovered {
        Some((c, u)) => (c, u),
        None => {
            let (_, c) = mesh_types::local_ai_shard_config();
            (c, mesh_types::ai_shard_urls(c))
        }
    };
    let mut urls = mesh_types::prefer_worker_ai_shard(&urls, address, count, &shard_urls);
    // Prefer seed/primary edge; :18083 is often a mine-only edge.
    urls.sort_by_key(|u| ai_url_backoff_rank(u));
    urls
}

fn ai_url_backoff_rank(url: &str) -> u8 {
    let u = url.to_ascii_lowercase();
    if u.contains(":18083") {
        2
    } else if u.contains(":18081") {
        1
    } else {
        0
    }
}

/// Pull `ai_shard_count` + `ai_shard_urls` from `/v1/ai/health` (fallback: getnodeinfo).
fn discover_ai_shards(rpc: &str) -> Option<(u32, Vec<String>)> {
    if rpc.is_empty() {
        return None;
    }
    #[derive(Deserialize)]
    struct ShardInfo {
        #[serde(default)]
        ai_shard_count: u32,
        #[serde(default)]
        ai_shard_urls: Vec<String>,
    }
    let base = rpc.trim_end_matches('/');
    for path in ["/v1/ai/health", "/v1/getnodeinfo"] {
        let url = format!("{base}{path}");
        let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(3)).call() else {
            continue;
        };
        let Ok(info) = resp.into_json::<ShardInfo>() else {
            continue;
        };
        if info.ai_shard_count > 1 && !info.ai_shard_urls.is_empty() {
            let urls: Vec<String> = info
                .ai_shard_urls
                .into_iter()
                .map(|u| u.trim().trim_end_matches('/').to_string())
                .filter(|u| !u.is_empty())
                .collect();
            if !urls.is_empty() {
                return Some((info.ai_shard_count, urls));
            }
        }
    }
    None
}

fn pin_orch_to_url(urls: &[String], orch_idx: &AtomicUsize, try_url: &str) -> bool {
    let want = try_url.trim().trim_end_matches('/');
    if let Some(pos) = urls
        .iter()
        .position(|u| u.trim().trim_end_matches('/') == want)
    {
        orch_idx.store(pos, Ordering::SeqCst);
        return true;
    }
    false
}

fn worker_slot(
    agent: ureq::Agent,
    urls: Arc<Vec<String>>,
    orch_idx: Arc<AtomicUsize>,
    address: String,
    stop: Arc<AtomicBool>,
    tx: Sender<MinerEvent>,
    cache: Arc<Mutex<Option<ModelCache>>>,
    slot: u32,
    capacity: Arc<AiCapacity>,
) {
    let vram_mb = capacity.vram_mb;
    let mut idle_backoff_ms = 150u64;
    while !stop.load(Ordering::SeqCst) {
        let i = orch_idx.load(Ordering::SeqCst) % urls.len().max(1);
        let orch = &urls[i];
        match pull_and_run(&agent, orch, &address, &cache, vram_mb) {
            Ok((job_id, kind, brain_epoch, backend)) => {
                idle_backoff_ms = 150;
                let _ = tx.send(MinerEvent::AiJobDone {
                    job_id,
                    kind: format!("{kind}/{backend}"),
                    brain_epoch,
                });
                // Cheap CPU jobs used to chain instantly and look like a burst.
                if kind == "protocol_eval" || kind == "benchmark" {
                    thread::sleep(Duration::from_millis(250));
                } else if slot == 0 {
                    thread::sleep(Duration::from_millis(20));
                }
            }
            Err(e) => {
                let msg = e.to_string();
                let unknown = msg.to_ascii_lowercase().contains("unknown worker");
                if unknown {
                    let _ = tx.send(MinerEvent::Status(format!(
                        "AI unknown worker — re-advertising on {orch}"
                    )));
                    if let Err(ae) = advertise(&agent, orch, &address, &capacity) {
                        let _ = tx.send(MinerEvent::Error(format!("AI re-advertise failed: {ae}")));
                        if urls.len() > 1 {
                            let _ = orch_idx.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    idle_backoff_ms = 400;
                    thread::sleep(Duration::from_millis(idle_backoff_ms));
                    continue;
                }
                let quiet = msg.contains("no job")
                    || msg.contains("NoJob")
                    || msg.contains("no pending")
                    || msg.contains("empty")
                    || msg.contains("gpu busy")
                    || msg.contains("cpu research declined")
                    || msg.contains("404")
                    || msg.contains("204")
                    || msg.contains("429")
                    || msg.contains("job HTTP 204")
                    || msg.contains("job HTTP 429")
                    || msg.contains("status code 204")
                    || msg.contains("status code 429")
                    || msg.contains("rate limit")
                    || msg.contains("result HTTP 409")
                    || msg.contains("10054")
                    || msg.to_ascii_lowercase().contains("connection was forcibly closed")
                    || msg.to_ascii_lowercase().contains("connection reset")
                    || msg.to_ascii_lowercase().contains("stale shared brain")
                    || msg.contains("wrong AI shard")
                    || msg.contains("421")
                    // Generic 400 without unknown-worker is often empty/no-match noise.
                    || (msg.contains("job HTTP 400") && !unknown);
                if msg.contains("429") || msg.contains("rate limit") {
                    let secs = msg
                        .split("retry-after=")
                        .nth(1)
                        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(2)
                        .clamp(1, 60);
                    thread::sleep(Duration::from_secs(secs));
                    continue;
                }
                if let Some(try_url) = mesh_types::parse_wrong_shard_try_url(&msg) {
                    if pin_orch_to_url(&urls, &orch_idx, &try_url) {
                        let _ = tx.send(MinerEvent::Status(format!(
                            "AI shard sticky → {try_url}"
                        )));
                        idle_backoff_ms = 200;
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                }
                let hard = msg.contains("HTTP 5")
                    || msg.to_ascii_lowercase().contains("connection")
                    || msg.to_ascii_lowercase().contains("timed out")
                    || msg.to_ascii_lowercase().contains("refused");
                if hard && urls.len() > 1 {
                    let next = (orch_idx.fetch_add(1, Ordering::SeqCst).wrapping_add(1)) % urls.len();
                    let _ = tx.send(MinerEvent::Status(format!("AI failover → {}", urls[next])));
                }
                let display = if msg.to_ascii_lowercase().contains("ml_train verification failed")
                {
                    "AI job: train verify failed (f64 brain OS mismatch or stale build — seed routes Windows to v2/protocol)".into()
                } else {
                    format!("AI job: {msg}")
                };
                if !quiet {
                    let _ = tx.send(MinerEvent::Error(display));
                }
                if msg.contains("409") || msg.to_ascii_lowercase().contains("stale") {
                    let _ = cache.lock().map(|mut g| *g = None);
                }
                // Stay close to the board — exponential backoff made AI look like it "pulsed".
                idle_backoff_ms = if quiet {
                    150
                } else {
                    (idle_backoff_ms.saturating_mul(2)).min(400)
                };
                thread::sleep(Duration::from_millis(idle_backoff_ms));
                continue;
            }
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}


fn advertise(
    agent: &ureq::Agent,
    orch: &str,
    address: &str,
    cap: &AiCapacity,
) -> Result<(), String> {
    let url = format!("{orch}/v1/advertise");
    // GPU miners train v2 on the card when Fusion is not mixing. They do not advertise
    // cpu_v1 / protocol_eval — those cheap jobs used to run on mine cores. The seed
    // rematches (seals) brain weights; the miner CPU only runs the exam sidecar.
    let (brain_backends, brain_contract, kinds) = if cuda_brain_available() {
        (
            serde_json::json!(["cuda_v2"]),
            BRAIN_CONTRACT,
            serde_json::json!(["ml_train", "ml_train_shared"]),
        )
    } else {
        (
            serde_json::json!(["cpu_v1"]),
            "",
            serde_json::json!(["echo", "benchmark", "protocol_eval", "ml_train", "ml_train_shared"]),
        )
    };
    let body = serde_json::json!({
        "address": address,
        "gpu_name": cap.gpu_name,
        "vram_mb": cap.vram_mb,
        "train_slots": cap.train_slots,
        "kinds": kinds,
        "brain_backends": brain_backends,
        "brain_contract": brain_contract,
        "os_family": std::env::consts::OS,
    });
    let resp = match with_ai_token(agent.post(&url)).send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            return Err(format!("advertise HTTP {code}: {text}"));
        }
        Err(e) => return Err(e.to_string()),
    };
    if resp.status() >= 300 {
        return Err(format!("advertise HTTP {}", resp.status()));
    }
    Ok(())
}

fn weights_for_epoch(
    agent: &ureq::Agent,
    orch: &str,
    need_epoch: u64,
    cache: &Mutex<Option<ModelCache>>,
    ver: u32,
) -> Result<Vec<u8>, String> {
    weights_cached(agent, orch, need_epoch, cache, ver, "", false)
}

fn weights_for_leg(
    agent: &ureq::Agent,
    orch: &str,
    leg: &str,
    need_epoch: u64,
    cache: &Mutex<Option<ModelCache>>,
) -> Result<Vec<u8>, String> {
    weights_cached(agent, orch, need_epoch, cache, 0, leg, false)
}

fn weights_for_qleg(
    agent: &ureq::Agent,
    orch: &str,
    leg: &str,
    need_epoch: u64,
    cache: &Mutex<Option<ModelCache>>,
) -> Result<Vec<u8>, String> {
    weights_cached(agent, orch, need_epoch, cache, 0, leg, true)
}

fn weights_cached(
    agent: &ureq::Agent,
    orch: &str,
    need_epoch: u64,
    cache: &Mutex<Option<ModelCache>>,
    ver: u32,
    leg: &str,
    quantum: bool,
) -> Result<Vec<u8>, String> {
    let cache_leg = if quantum {
        format!("q:{leg}")
    } else {
        leg.to_string()
    };
    let cache_key_ok = |c: &ModelCache| c.epoch == need_epoch && c.ver == ver && c.leg == cache_leg;
    if let Ok(guard) = cache.lock() {
        if let Some(c) = guard.as_ref() {
            if cache_key_ok(c) {
                return Ok(c.weights.clone());
            }
        }
    }
    // Prefer binary weights (half the wire of hex JSON); fall back to legacy JSON.
    let bin_url = if quantum && !leg.is_empty() {
        format!("{orch}/v1/qleg/{leg}/bin")
    } else if !leg.is_empty() {
        format!("{orch}/v1/leg/{leg}/bin")
    } else if ver == 2 {
        format!("{orch}/v1/model/bin?ver=2")
    } else {
        format!("{orch}/v1/model/bin?ver=1")
    };
    if let Ok(resp) = agent.get(&bin_url).call() {
        if resp.status() < 300 {
            let epoch = resp
                .header("x-mesh-epoch")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let mut weights = Vec::new();
            std::io::Read::read_to_end(&mut resp.into_reader(), &mut weights)
                .map_err(|e| e.to_string())?;
            if epoch != need_epoch {
                return Err(format!(
                    "result HTTP 409: stale shared brain epoch (job {need_epoch} live {epoch})"
                ));
            }
            if let Ok(mut guard) = cache.lock() {
                *guard = Some(ModelCache {
                    epoch,
                    ver,
                    leg: cache_leg.clone(),
                    weights: weights.clone(),
                });
            }
            return Ok(weights);
        }
    }
    let url = if quantum && !leg.is_empty() {
        format!("{orch}/v1/qleg/{leg}")
    } else if !leg.is_empty() {
        format!("{orch}/v1/leg/{leg}")
    } else if ver == 2 {
        format!("{orch}/v1/model?ver=2")
    } else {
        format!("{orch}/v1/model")
    };
    let resp = agent.get(&url).call().map_err(|e| e.to_string())?;
    let model: ModelResp = resp.into_json().map_err(|e| e.to_string())?;
    let weights = hex::decode(&model.weights_hex).map_err(|e| e.to_string())?;
    if model.epoch != need_epoch {
        return Err(format!(
            "result HTTP 409: stale shared brain epoch (job {need_epoch} live {})",
            model.epoch
        ));
    }
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(ModelCache {
            epoch: model.epoch,
            ver,
            leg: cache_leg,
            weights: weights.clone(),
        });
    }
    Ok(weights)
}

/// Train v2 on CUDA when Fusion mix is not holding the card.
/// Waits for the mix lock — never CPU-trains. Miner CPU is for pad fill + exams;
/// the seed rematches submitted weights.
fn run_v2_on_free_gpu(
    weights: &[u8],
    input: &[u8],
    workspace: u64,
) -> Result<(mesh_ai_v2::MlTrainV2Result, &'static str), String> {
    if !cuda_brain_available() {
        return Err("gpu required".into());
    }
    // Fusion owns the card for the whole mine session. Do not wait 30s between waves.
    if crate::gpu_gate::pow_holds_gpu() {
        return Err("gpu busy".into());
    }
    for _ in 0..40 {
        if crate::gpu_gate::pow_holds_gpu() {
            return Err("gpu busy".into());
        }
        if let Some(_gpu) = crate::gpu_gate::try_lock_gpu() {
            let r = run_job_prefer_cuda(weights, input, workspace).map_err(|e| e.to_string())?;
            return Ok((r, "cuda"));
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err("gpu busy".into())
}

fn pull_and_run(
    agent: &ureq::Agent,
    orch: &str,
    worker: &str,
    cache: &Mutex<Option<ModelCache>>,
    vram_mb: u32,
) -> Result<(String, String, Option<u64>, &'static str), String> {
    let url = format!("{orch}/v1/job");
    let resp = match with_ai_token(agent.post(&url)).send_json(serde_json::json!({ "worker": worker }))
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let retry_after = r
                .header("retry-after")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let text = r.into_string().unwrap_or_default();
            if code == 204 || code == 404 {
                return Err("no job".into());
            }
            if code == 429 {
                return Err(format!(
                    "job HTTP 429 retry-after={}: {text}",
                    retry_after.max(1)
                ));
            }
            // Legacy nodes returned NoJob as 400 — treat empty-board text as idle.
            if code == 400
                && (text.contains("no pending")
                    || text.contains("NoJob")
                    || text.to_ascii_lowercase().contains("no job"))
            {
                return Err("no job".into());
            }
            return Err(format!("job HTTP {code}: {text}"));
        }
        Err(e) => return Err(e.to_string()),
    };
    if resp.status() == 204 || resp.status() == 404 {
        return Err("no job".into());
    }
    if resp.status() >= 300 {
        return Err(format!("job HTTP {}", resp.status()));
    }
    let job: JobResp = resp.into_json().map_err(|e| e.to_string())?;
    let input = hex::decode(&job.input_hex).map_err(|e| e.to_string())?;
    let started = Instant::now();

    let mut trained_new_weights: Option<(u64, u32, String, Vec<u8>)> = None;
    let mut used_backend = "cpu";
    let gpu_trainer = cuda_brain_available();
    let decline_cpu = || -> Result<Vec<u8>, String> { Err("cpu research declined".into()) };
    let output = match job.kind.as_str() {
        "benchmark" if gpu_trainer => decline_cpu()?,
        "protocol_eval" if gpu_trainer => decline_cpu()?,
        "benchmark" => run_benchmark(&input).to_vec(),
        "protocol_eval" => run_protocol_eval(&input).to_vec(),
        "leg_train" if gpu_trainer => decline_cpu()?,
        "leg_train" => {
            let spec = parse_leg_job(&input).map_err(|e| e.to_string())?;
            let leg = spec.leg.as_str();
            let weights = weights_for_leg(agent, orch, leg, spec.epoch, cache)?;
            let r = run_leg_train(&weights, &input).map_err(|e| e.to_string())?;
            trained_new_weights = Some((
                spec.epoch.saturating_add(1),
                0,
                leg.to_string(),
                r.new_weights.clone(),
            ));
            r.output
        }
        "quantum_train" if gpu_trainer => decline_cpu()?,
        "quantum_train" => {
            let spec = parse_quantum_job(&input).map_err(|e| e.to_string())?;
            let leg = spec.leg.as_str();
            let weights = weights_for_qleg(agent, orch, leg, spec.epoch, cache)?;
            let r = run_quantum_train(&weights, &input).map_err(|e| e.to_string())?;
            trained_new_weights = Some((
                spec.epoch.saturating_add(1),
                0,
                format!("q:{leg}"),
                r.new_weights.clone(),
            ));
            r.output
        }
        "ml_train_shared_v2" => {
            let spec = parse_v2_job(&input).map_err(|e| e.to_string())?;
            let weights = weights_for_epoch(agent, orch, spec.epoch, cache, 2)?;
            let workspace = ((vram_mb as u64) * 1024 * 1024 * 40 / 100).max(64 * 1024 * 1024);
            let (r, backend) = run_v2_on_free_gpu(&weights, &input, workspace)?;
            used_backend = backend;
            trained_new_weights =
                Some((spec.epoch.saturating_add(1), 2, String::new(), r.new_weights.clone()));
            r.output
        }
        "ml_train_shared" if gpu_trainer => decline_cpu()?,
        "ml_train_shared" => {
            let spec = parse_ml_train_shared_input(&input).map_err(|e| e.to_string())?;
            let weights = weights_for_epoch(agent, orch, spec.epoch, cache, 1)?;
            let r = run_ml_train_shared(&weights, &input).map_err(|e| e.to_string())?;
            trained_new_weights =
                Some((spec.epoch.saturating_add(1), 1, String::new(), r.new_weights.clone()));
            r.output
        }
        "ml_train" => {
            if is_ml_train_shared_v2(&input) {
                let spec = parse_v2_job(&input).map_err(|e| e.to_string())?;
                let weights = weights_for_epoch(agent, orch, spec.epoch, cache, 2)?;
                let workspace = ((vram_mb as u64) * 1024 * 1024 * 40 / 100).max(64 * 1024 * 1024);
                let (r, backend) = run_v2_on_free_gpu(&weights, &input, workspace)?;
                used_backend = backend;
                trained_new_weights =
                    Some((spec.epoch.saturating_add(1), 2, String::new(), r.new_weights.clone()));
                r.output
            } else if gpu_trainer {
                return Err("cpu research declined".into());
            } else if is_quantum_train(&input) {
                let spec = parse_quantum_job(&input).map_err(|e| e.to_string())?;
                let leg = spec.leg.as_str();
                let weights = weights_for_qleg(agent, orch, leg, spec.epoch, cache)?;
                let r = run_quantum_train(&weights, &input).map_err(|e| e.to_string())?;
                trained_new_weights = Some((
                    spec.epoch.saturating_add(1),
                    0,
                    format!("q:{leg}"),
                    r.new_weights.clone(),
                ));
                r.output
            } else if is_leg_train(&input) {
                let spec = parse_leg_job(&input).map_err(|e| e.to_string())?;
                let leg = spec.leg.as_str();
                let weights = weights_for_leg(agent, orch, leg, spec.epoch, cache)?;
                let r = run_leg_train(&weights, &input).map_err(|e| e.to_string())?;
                trained_new_weights = Some((
                    spec.epoch.saturating_add(1),
                    0,
                    leg.to_string(),
                    r.new_weights.clone(),
                ));
                r.output
            } else if is_ml_train_shared(&input) {
                let spec = parse_ml_train_shared_input(&input).map_err(|e| e.to_string())?;
                let weights = weights_for_epoch(agent, orch, spec.epoch, cache, 1)?;
                let r = run_ml_train_shared(&weights, &input).map_err(|e| e.to_string())?;
                trained_new_weights =
                    Some((spec.epoch.saturating_add(1), 1, String::new(), r.new_weights.clone()));
                r.output
            } else {
                run_ml_train_job(&input)
            }
        }
        _ => input.clone(),
    };
    let latency_ms = started.elapsed().as_millis() as u64;

    let result_url = format!("{orch}/v1/result");
    let body = serde_json::json!({
        "worker": worker,
        "job_id": job.job_id,
        "output_hex": hex::encode(&output),
        "latency_ms": latency_ms,
    });
    let resp = {
        let mut last_err = None;
        let mut ok_resp = None;
        for attempt in 0..3 {
            match with_ai_token(agent.post(&result_url)).send_json(body.clone()) {
                Ok(r) => {
                    ok_resp = Some(r);
                    break;
                }
                Err(ureq::Error::Status(code, r)) => {
                    let text = r.into_string().unwrap_or_default();
                    return Err(format!("result HTTP {code}: {text}"));
                }
                Err(e) => {
                    let s = e.to_string();
                    let transient = s.contains("10054")
                        || s.to_ascii_lowercase().contains("forcibly closed")
                        || s.to_ascii_lowercase().contains("connection reset")
                        || s.to_ascii_lowercase().contains("broken pipe")
                        || s.to_ascii_lowercase().contains("connection refused");
                    last_err = Some(s);
                    if !transient || attempt == 2 {
                        break;
                    }
                    thread::sleep(Duration::from_millis(150 * (attempt as u64 + 1)));
                }
            }
        }
        match ok_resp {
            Some(r) => r,
            None => return Err(last_err.unwrap_or_else(|| "result submit failed".into())),
        }
    };
    if resp.status() >= 300 {
        return Err(format!("result HTTP {}", resp.status()));
    }
    let result_ok = resp.into_json::<ResultOk>().ok();
    let brain_epoch = result_ok.as_ref().and_then(|r| {
        if trained_new_weights
            .as_ref()
            .map(|(_, ver, _, _)| *ver == 2)
            .unwrap_or(false)
        {
            r.brain_v2_epoch.or(r.brain_epoch)
        } else {
            r.brain_epoch
        }
    });

    if let Some((next_epoch, ver, leg, w)) = trained_new_weights {
        let store_epoch = brain_epoch.unwrap_or(next_epoch);
        if let Ok(mut guard) = cache.lock() {
            *guard = Some(ModelCache {
                epoch: store_epoch,
                ver,
                leg,
                weights: w,
            });
        }
    }

    Ok((job.job_id, job.kind, brain_epoch, used_backend))
}
