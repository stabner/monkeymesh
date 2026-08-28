//! MonkeyMesh multi-backend miner — CLI (headless).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use mesh_crypto::Keypair;
use mesh_miner_gpu::engine::{
    format_hashrate, run_rpc_loop, MinerBackend, MinerConfig, MinerEvent,
};
use mesh_types::Address;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Clone, ValueEnum, Debug)]
enum BackendArg {
    Auto,
    Nvidia,
    Amd,
    Cpu,
}

impl From<BackendArg> for MinerBackend {
    fn from(v: BackendArg) -> Self {
        match v {
            BackendArg::Auto => MinerBackend::Auto,
            BackendArg::Nvidia => MinerBackend::Nvidia,
            BackendArg::Amd => MinerBackend::Amd,
            BackendArg::Cpu => MinerBackend::Cpu,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "mesh-miner-gpu",
    about = "MonkeyMesh miner CLI (NVIDIA CUDA / AMD OpenCL / CPU)"
)]
struct Args {
    /// Node RPC base URL(s), comma-separated (seed + edge failover)
    #[arg(long, default_value = "")]
    rpc: String,

    /// Coinbase / payout address
    #[arg(long)]
    address: Option<String>,

    /// Local keyfile used only if --address is omitted
    #[arg(long, default_value = "data/miner.key")]
    keyfile: PathBuf,

    /// Blocks to mine then exit (0 = until Ctrl+C)
    #[arg(long, default_value_t = 0)]
    blocks: u64,

    /// Nonces per mix batch
    #[arg(long, default_value_t = 0, help = "Nonces per mix batch (0 = auto from GPU VRAM)")]
    batch: u32,

    /// Max nonces to try per template before fetching a fresh one
    #[arg(long, default_value_t = 5_000_000)]
    max_nonces: u64,

    /// Device index (CUDA or OpenCL)
    #[arg(long, default_value_t = 0)]
    device: i32,

    /// Mining backend (PoW = CPU only; GPU backends refuse MeshHash blocks)
    #[arg(long, value_enum, default_value_t = BackendArg::Cpu)]
    backend: BackendArg,

    /// Worker / rig name — pool identity is `address.worker` (default: default)
    #[arg(long, default_value = "default")]
    worker_name: String,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let args = Args::parse();
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        let _ = ctrlc::set_handler(move || {
            stop.store(true, Ordering::SeqCst);
            eprintln!("\nStopping miner…");
        });
    }

    let payout = resolve_payout(&args)?;
    let backend = MinerBackend::from(args.backend);
    info!(?backend, device = args.device, "starting miner");

    let rpc = if args.rpc.trim().is_empty() {
        mesh_types::default_rpc_urls().join(",")
    } else {
        args.rpc.trim().to_string()
    };
    let cfg = MinerConfig {
        rpc,
        address: payout,
        batch: args.batch,
        max_nonces: args.max_nonces,
        device: args.device,
        backend,
        devices: Vec::new(),
        pool: None,
        worker_name: Some(args.worker_name),
    };

    let (tx, rx) = mpsc::channel::<MinerEvent>();
    let stop_w = stop.clone();
    let target = if args.blocks == 0 {
        u64::MAX
    } else {
        args.blocks
    };
    let worker = thread::spawn(move || run_rpc_loop(cfg, stop_w, tx));

    let mut mined = 0u64;
    while let Ok(ev) = rx.recv() {
        match ev {
            MinerEvent::Status(s) => info!("{s}"),
            MinerEvent::Hashrate { cpu_hs, gpu_hs } => {
                let fusion = MinerEvent::hashrate_fusion(cpu_hs, gpu_hs);
                info!(
                    fusion = %format_hashrate(fusion),
                    cpu = %format_hashrate(cpu_hs),
                    gpu = %format_hashrate(gpu_hs),
                    "hashrate"
                );
            }
            MinerEvent::BlockFound { height, id } => {
                mined += 1;
                info!(height, %id, mined, "block accepted");
                if mined >= target {
                    stop.store(true, Ordering::SeqCst);
                }
            }
            MinerEvent::AiJobDone {
                job_id,
                kind,
                brain_epoch,
            } => {
                info!(%job_id, %kind, ?brain_epoch, "ai job done");
            }
            MinerEvent::Error(e) => warn!("mine error: {e}"),
            MinerEvent::Stopped | MinerEvent::AiStopped => break,
        }
    }
    let _ = worker.join();
    info!(mined, "miner exit");
    Ok(())
}

fn resolve_payout(args: &Args) -> Result<Address> {
    if let Some(a) = &args.address {
        return Address::from_hex(a.trim()).ok_or_else(|| anyhow::anyhow!("bad --address"));
    }
    warn!("--address not set; deriving from --keyfile");
    load_or_create_key(&args.keyfile).map(|k| k.address())
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
