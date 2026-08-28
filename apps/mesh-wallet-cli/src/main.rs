use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use mesh_chain::build_signed_payment;
use mesh_crypto::Keypair;
use mesh_types::{Address, Amount, OutPoint, Utxo};
use serde::Deserialize;
use serde_json::json;

#[derive(Parser, Debug)]
#[command(name = "mesh-wallet-cli", about = "MonkeyMesh wallet CLI")]
struct Args {
    #[arg(long, default_value = "data/wallet.key")]
    wallet: PathBuf,

    /// Node REST RPC base URL (preferred). Example: http://127.0.0.1:18080
    #[arg(long, default_value = "http://127.0.0.1:18080")]
    rpc: String,

    /// Local chain file fallback when --offline is set
    #[arg(long, default_value = "data/chain.bin")]
    chain: PathBuf,

    /// Use local chain file instead of RPC
    #[arg(long)]
    offline: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Create a new wallet key file
    New,
    /// Print address for the wallet
    Address,
    /// Show on-chain balance
    Balance,
    /// List UTXOs for this wallet
    Utxos,
    /// Export public key hex
    Pubkey,
    /// Show node info via RPC
    Info,
    /// Mine blocks via the node RPC (preferred over mesh-miner-cpu while node runs)
    Mine {
        #[arg(long, default_value_t = 1)]
        blocks: u64,
        /// Coinbase address (defaults to this wallet)
        #[arg(long)]
        address: Option<String>,
    },
    /// Send MESH to an address (signs locally, submits via RPC)
    Send {
        to: String,
        /// Amount in MESH (e.g. 1.5)
        amount: String,
        #[arg(long, default_value = "")]
        memo: String,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.cmd {
        Cmd::New => {
            if args.wallet.exists() {
                bail!("wallet already exists at {}", args.wallet.display());
            }
            if let Some(parent) = args.wallet.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let kp = Keypair::generate();
            std::fs::write(&args.wallet, kp.to_hex())?;
            println!("created {}", args.wallet.display());
            println!("address {}", kp.address());
        }
        Cmd::Address => {
            let kp = load(&args.wallet)?;
            println!("{}", kp.address());
        }
        Cmd::Pubkey => {
            let kp = load(&args.wallet)?;
            println!("{}", hex::encode(kp.public_key_bytes()));
        }
        Cmd::Info => {
            if args.offline {
                bail!("info requires RPC (omit --offline)");
            }
            let v: serde_json::Value = rpc_get(&args.rpc, "/v1/getnodeinfo")?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        Cmd::Mine { blocks, address } => {
            if args.offline {
                bail!("mine requires RPC (omit --offline)");
            }
            let kp = load(&args.wallet)?;
            let addr = address.unwrap_or_else(|| kp.address().to_string());
            let resp: MineResp = rpc_post(
                &args.rpc,
                "/v1/mine",
                &json!({ "blocks": blocks, "address": addr }),
            )?;
            for b in &resp.mined {
                println!(
                    "mined height={} id={} diff={} txs={}",
                    b.height, b.id, b.difficulty, b.txs
                );
            }
            println!("tip height={} {}", resp.height, resp.tip);
        }
        Cmd::Balance => {
            let kp = load(&args.wallet)?;
            if args.offline {
                let chain = mesh_chain::Chain::open(&args.chain)?;
                println!("{}", chain.balance(&kp.address()));
            } else {
                let path = format!("/v1/getbalance?address={}", kp.address());
                let v: BalanceResp = rpc_get(&args.rpc, &path)?;
                println!("{}", v.balance);
            }
        }
        Cmd::Utxos => {
            let kp = load(&args.wallet)?;
            if args.offline {
                let chain = mesh_chain::Chain::open(&args.chain)?;
                let utxos = chain.utxos_for(&kp.address());
                if utxos.is_empty() {
                    println!("(none)");
                } else {
                    for (op, u) in utxos {
                        println!("{op}  {}", u.amount);
                    }
                }
            } else {
                let path = format!("/v1/utxos?address={}", kp.address());
                let items: Vec<UtxoRpc> = rpc_get(&args.rpc, &path)?;
                if items.is_empty() {
                    println!("(none)");
                } else {
                    for u in items {
                        println!("{}:{}  {}", u.txid, u.vout, u.amount);
                    }
                }
            }
        }
        Cmd::Send { to, amount, memo } => {
            let kp = load(&args.wallet)?;
            let dest = Address::from_hex(&to).ok_or_else(|| anyhow::anyhow!("bad address"))?;
            let amt = Amount::parse_mesh(&amount)
                .ok_or_else(|| anyhow::anyhow!("bad amount (use MESH units, e.g. 1.5)"))?;

            if args.offline {
                let mut chain = mesh_chain::Chain::open(&args.chain)?;
                let txid = chain.send(&kp, dest, amt, memo)?;
                println!("submitted {txid}");
                println!("balance (confirmed) {}", chain.balance(&kp.address()));
            } else {
                let path = format!("/v1/utxos?address={}", kp.address());
                let items: Vec<UtxoRpc> = rpc_get(&args.rpc, &path)?;
                let utxos = items
                    .into_iter()
                    .filter(|u| u.mature.unwrap_or(true))
                    .map(|u| {
                        let txid = mesh_types::Hash::from_hex(&u.txid)
                            .map_err(|e| anyhow::anyhow!("bad utxo txid: {e}"))?;
                        Ok((
                            OutPoint::new(txid, u.vout),
                            Utxo {
                                address: kp.address(),
                                amount: Amount::from_atomic(u.atomic),
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let tx = build_signed_payment(&kp, &utxos, dest, amt, memo)?;
                let tx_hex = hex::encode(bincode::serialize(&tx)?);
                let resp: SendResp = rpc_post(
                    &args.rpc,
                    "/v1/submittx",
                    &json!({ "tx_hex": tx_hex }),
                )?;
                println!("submitted {}", resp.txid);
                let bal_path = format!("/v1/getbalance?address={}", kp.address());
                let bal: BalanceResp = rpc_get(&args.rpc, &bal_path)?;
                println!("balance (confirmed) {}", bal.balance);
                println!("mine a block on the node to confirm");
            }
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct BalanceResp {
    balance: String,
}

#[derive(Deserialize)]
struct UtxoRpc {
    txid: String,
    vout: u32,
    amount: String,
    atomic: u64,
    #[serde(default)]
    mature: Option<bool>,
}

#[derive(Deserialize)]
struct SendResp {
    txid: String,
}

#[derive(Deserialize)]
struct MineResp {
    mined: Vec<MinedBlock>,
    height: u64,
    tip: String,
}

#[derive(Deserialize)]
struct MinedBlock {
    height: u64,
    id: String,
    difficulty: u32,
    txs: usize,
}

fn load(path: &PathBuf) -> Result<Keypair> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.len() == 32 {
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&bytes);
        return Ok(Keypair::from_bytes(secret));
    }
    let hex = String::from_utf8(bytes).context("wallet key is not UTF-8 hex or 32 raw bytes")?;
    Ok(Keypair::from_hex(hex.trim())?)
}

fn rpc_get<T: for<'de> Deserialize<'de>>(base: &str, path: &str) -> Result<T> {
    let url = format!("{}{path}", base.trim_end_matches('/'));
    let resp = ureq::get(&url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    if !(200..300).contains(&resp.status()) {
        bail!("RPC {url} returned {}", resp.status());
    }
    Ok(resp.into_json()?)
}

fn rpc_post<T: for<'de> Deserialize<'de>>(
    base: &str,
    path: &str,
    body: &serde_json::Value,
) -> Result<T> {
    let url = format!("{}{path}", base.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .send_json(body.clone())
        .with_context(|| format!("POST {url}"))?;
    if !(200..300).contains(&resp.status()) {
        bail!("RPC {url} returned {}", resp.status());
    }
    Ok(resp.into_json()?)
}