//! GPU AI worker — advertise to node AI board, pull jobs, submit verified results.
//!
//! Shared-brain jobs: GET /v1/model → train from network weights → submit new weights.
//! Protocol sims stay deterministic re-exec verify.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use mesh_ai::{run_benchmark, run_leg_train, run_ml_train_job, run_ml_train_shared, run_protocol_eval, run_quantum_train};
use mesh_ai_v2::{cuda_brain_available, is_ml_train_shared_v2, run_job_prefer_cuda, BRAIN_CONTRACT};
use mesh_ai::{is_leg_train, is_quantum_train, parse_leg_job, parse_quantum_job};
use mesh_crypto::Keypair;
use serde::Deserialize;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "mesh-gpu-worker", about = "MonkeyMesh GPU AI job worker (shared brain)")]
struct Args {
    /// Node RPC with embedded AI board
    #[arg(long, default_value = "http://seednode.hashmonkeys.cloud:18080")]
    orch: String,

    /// Payout / worker address (mesh…)
    #[arg(long)]
    address: Option<String>,

    /// Keyfile used if --address omitted
    #[arg(long, default_value = "data/gpu-worker.key")]
    keyfile: PathBuf,

    /// Display name for capability advertise
    #[arg(long, default_value = "local-gpu")]
    gpu_name: String,

    #[arg(long, default_value_t = 8192)]
    vram_mb: u32,

    /// Parallel AI job slots (0 = derive from --vram-mb)
    #[arg(long, default_value_t = 0)]
    train_slots: u32,

    /// Jobs to complete then exit (0 = forever)
    #[arg(long, default_value_t = 0)]
    jobs: u64,

    /// Pause between job polls
    #[arg(long, default_value_t = 500)]
    poll_ms: u64,
}

#[derive(Deserialize)]
struct JobResp {
    job_id: String,
    kind: String,
    input_hex: String,
    #[allow(dead_code)]
    input_commitment: String,
}

#[derive(Deserialize)]
struct ModelResp {
    epoch: u64,
    weights_hex: String,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let args = Args::parse();
    let address = if let Some(a) = args.address.clone() {
        a
    } else {
        let kp = load_or_create_key(&args.keyfile)?;
        let a = kp.address().to_string();
        info!(%a, path = %args.keyfile.display(), "worker address");
        a
    };

    let urls = resolve_ai_orch_urls(&args.orch, &address);
    if urls.is_empty() {
        bail!("AI orch URL empty");
    }

    let mut orch = urls[0].clone();
    let mut advertised = false;
    for u in &urls {
        match advertise(u, &address, &args.gpu_name, args.vram_mb, args.train_slots) {
            Ok(()) => {
                orch = u.clone();
                advertised = true;
                break;
            }
            Err(e) => {
                if let Some(try_url) = mesh_types::parse_wrong_shard_try_url(&e.to_string()) {
                    warn!(%u, %try_url, "advertise wrong shard; will prefer redirect");
                }
                warn!(%u, error = %e, "advertise failed");
            }
        }
    }
    if !advertised {
        bail!("advertise failed on all orch URLs");
    }
    let slots = if args.train_slots > 0 {
        args.train_slots
    } else {
        mesh_ai::train_slots_for_vram(args.vram_mb)
    };
    info!(%orch, %address, vram_mb = args.vram_mb, slots, cuda_v2 = cuda_brain_available(), "advertised; pulling shared-brain + protocol jobs");

    let mut done = 0u64;
    let mut orch_i = urls.iter().position(|u| u == &orch).unwrap_or(0);
    loop {
        let orch = &urls[orch_i % urls.len()];
        match pull_and_run(orch, &address, args.vram_mb) {
            Ok((job_id, kind)) => {
                done += 1;
                info!(%job_id, %kind, done, "job completed");
                if args.jobs > 0 && done >= args.jobs {
                    break;
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("429") || msg.contains("rate limit") {
                    thread::sleep(Duration::from_secs(2));
                } else if msg.to_ascii_lowercase().contains("unknown worker") {
                    // Board lost worker registry (restart / wiped ai_queue) — re-advertise.
                    warn!(%orch, "unknown worker; re-advertising");
                    if let Err(ae) = advertise(orch, &address, &args.gpu_name, args.vram_mb, args.train_slots)
                    {
                        warn!(error = %ae, "re-advertise failed");
                        orch_i = orch_i.wrapping_add(1);
                    }
                } else if let Some(try_url) = mesh_types::parse_wrong_shard_try_url(&msg) {
                    if let Some(pos) = urls
                        .iter()
                        .position(|u| u.trim().trim_end_matches('/') == try_url.trim_end_matches('/'))
                    {
                        orch_i = pos;
                        warn!(next = %urls[orch_i], "AI shard sticky");
                    } else {
                        warn!(%try_url, "AI shard redirect not in URL list");
                    }
                } else if msg.to_ascii_lowercase().contains("connection") || msg.contains("HTTP 5")
                {
                    orch_i = orch_i.wrapping_add(1);
                    warn!(next = %urls[orch_i % urls.len()], "AI orch failover");
                } else {
                    warn!(error = %e, "job cycle failed; retrying");
                }
                thread::sleep(Duration::from_millis(args.poll_ms.max(200)));
            }
        }
        thread::sleep(Duration::from_millis(args.poll_ms));
    }
    Ok(())
}

fn load_or_create_key(path: &PathBuf) -> Result<Keypair> {
    if path.exists() {
        let hex = std::fs::read_to_string(path)?;
        return Ok(Keypair::from_hex(hex.trim())?);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let kp = Keypair::generate();
    std::fs::write(path, kp.to_hex())?;
    Ok(kp)
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
    mesh_types::prefer_worker_ai_shard(&urls, address, count, &shard_urls)
}

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
        std::path::PathBuf::from("ai.token"),
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

fn advertise(orch: &str, address: &str, gpu_name: &str, vram_mb: u32, train_slots: u32) -> Result<()> {
    let slots = if train_slots > 0 {
        train_slots
    } else {
        mesh_ai::train_slots_for_vram(vram_mb)
    };
    let (brain_backends, brain_contract) = if cuda_brain_available() {
        (
            serde_json::json!(["cpu_v1", "cuda_v2"]),
            BRAIN_CONTRACT,
        )
    } else {
        (serde_json::json!(["cpu_v1"]), "")
    };
    let url = format!("{orch}/v1/advertise");
    let body = serde_json::json!({
        "address": address,
        "gpu_name": gpu_name,
        "vram_mb": vram_mb,
        "train_slots": slots,
        "kinds": ["echo", "benchmark", "protocol_eval", "ml_train", "ml_train_shared"],
        "brain_backends": brain_backends,
        "brain_contract": brain_contract,
        "os_family": std::env::consts::OS,
    });
    let resp = match with_ai_token(ureq::post(&url)).send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            bail!("advertise HTTP {code}: {text}");
        }
        Err(e) => return Err(e).context("advertise"),
    };
    if resp.status() >= 300 {
        bail!("advertise HTTP {}", resp.status());
    }
    Ok(())
}

fn fetch_model(orch: &str, ver: u32) -> Result<(u64, Vec<u8>)> {
    let url = if ver == 2 {
        format!("{orch}/v1/model?ver=2")
    } else {
        format!("{orch}/v1/model")
    };
    let resp = with_ai_token(ureq::get(&url)).call().context("get model")?;
    let model: ModelResp = resp.into_json().context("parse model")?;
    let weights = hex::decode(&model.weights_hex).context("weights hex")?;
    Ok((model.epoch, weights))
}

fn fetch_leg(orch: &str, leg: &str) -> Result<(u64, Vec<u8>)> {
    let url = format!("{orch}/v1/leg/{leg}");
    let resp = with_ai_token(ureq::get(&url)).call().context("get leg")?;
    let model: ModelResp = resp.into_json().context("parse leg")?;
    let weights = hex::decode(&model.weights_hex).context("weights hex")?;
    Ok((model.epoch, weights))
}

fn fetch_qleg(orch: &str, leg: &str) -> Result<(u64, Vec<u8>)> {
    let url = format!("{orch}/v1/qleg/{leg}");
    let resp = with_ai_token(ureq::get(&url)).call().context("get qleg")?;
    let model: ModelResp = resp.into_json().context("parse qleg")?;
    let weights = hex::decode(&model.weights_hex).context("weights hex")?;
    Ok((model.epoch, weights))
}

fn pull_and_run(orch: &str, worker: &str, vram_mb: u32) -> Result<(String, String)> {
    let url = format!("{orch}/v1/job");
    let resp = match with_ai_token(ureq::post(&url)).send_json(serde_json::json!({ "worker": worker }))
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            bail!("job HTTP {code}: {text}");
        }
        Err(e) => return Err(e).context("take job"),
    };
    let job: JobResp = resp.into_json().context("parse job")?;
    let input = hex::decode(&job.input_hex).context("input hex")?;
    let started = Instant::now();
    let workspace = ((vram_mb as u64) * 1024 * 1024 * 40 / 100).max(64 * 1024 * 1024);

    let output = match job.kind.as_str() {
        "benchmark" => run_benchmark(&input).to_vec(),
        "protocol_eval" => run_protocol_eval(&input).to_vec(),
        "leg_train" => {
            let spec = parse_leg_job(&input)?;
            let (epoch, weights) = fetch_leg(orch, spec.leg.as_str())?;
            info!(epoch, leg = spec.leg.as_str(), "training trilemma guardian");
            run_leg_train(&weights, &input)?.output
        }
        "quantum_train" => {
            let spec = parse_quantum_job(&input)?;
            let (epoch, weights) = fetch_qleg(orch, spec.leg.as_str())?;
            info!(epoch, leg = spec.leg.as_str(), "training quantum guardian");
            run_quantum_train(&weights, &input)?.output
        }
        "ml_train_shared_v2" => {
            let (epoch, weights) = fetch_model(orch, 2)?;
            info!(epoch, "training shared brain v2");
            run_job_prefer_cuda(&weights, &input, workspace)?.output
        }
        "ml_train_shared" => {
            let (epoch, weights) = fetch_model(orch, 1)?;
            info!(epoch, "training shared brain");
            let r = run_ml_train_shared(&weights, &input).context("shared train")?;
            r.output
        }
        "ml_train" => {
            if is_quantum_train(&input) {
                let spec = parse_quantum_job(&input)?;
                let (_epoch, weights) = fetch_qleg(orch, spec.leg.as_str())?;
                run_quantum_train(&weights, &input)?.output
            } else if is_leg_train(&input) {
                let spec = parse_leg_job(&input)?;
                let (_epoch, weights) = fetch_leg(orch, spec.leg.as_str())?;
                run_leg_train(&weights, &input)?.output
            } else if is_ml_train_shared_v2(&input) {
                let (_epoch, weights) = fetch_model(orch, 2)?;
                run_job_prefer_cuda(&weights, &input, workspace)?.output
            } else if mesh_ai::is_ml_train_shared(&input) {
                let (_epoch, weights) = fetch_model(orch, 1)?;
                run_ml_train_shared(&weights, &input)
                    .context("shared train")?
                    .output
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
    let resp = match with_ai_token(ureq::post(&result_url)).send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            bail!("result HTTP {code}: {text}");
        }
        Err(e) => return Err(e).context("submit result"),
    };
    if resp.status() >= 300 {
        bail!("result HTTP {}", resp.status());
    }
    Ok((job.job_id, job.kind))
}
