use std::fs;
use std::path::PathBuf;

use mesh_chain::build_signed_payment;
use mesh_crypto::Keypair;
use mesh_types::{Address, Amount, Hash, OutPoint, Utxo};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::Manager;

#[derive(Serialize)]
struct WalletView {
    address: String,
    balance: String,
}

#[derive(Serialize)]
struct NodeInfoView {
    height: u64,
    tip: String,
    next_difficulty: u32,
    mempool: usize,
    peer_id: Option<String>,
    finalized_height: u64,
    finality_active: bool,
}

#[derive(Deserialize)]
struct BalanceResp {
    balance: String,
}

#[derive(Deserialize)]
struct NodeInfoResp {
    height: u64,
    tip: String,
    next_difficulty: u32,
    mempool: usize,
    peer_id: Option<String>,
    #[serde(default)]
    finalized_height: u64,
    #[serde(default)]
    finality_active: bool,
}

#[derive(Deserialize)]
struct UtxoRpc {
    txid: String,
    vout: u32,
    atomic: u64,
}

#[derive(Deserialize)]
struct SendResp {
    txid: String,
}

#[derive(Deserialize)]
struct MineResp {
    height: u64,
    tip: String,
}

#[derive(Serialize)]
struct MineView {
    height: u64,
    tip: String,
}

#[derive(Serialize)]
struct SendView {
    txid: String,
}

fn default_rpc_url(app: &tauri::AppHandle) -> String {
    if let Ok(v) = std::env::var("MESH_RPC") {
        let t = v.trim().trim_end_matches('/').to_string();
        if !t.is_empty() {
            return t;
        }
    }
    // Prefer config.json beside the executable (Launchers/Wallet/bin/config.json)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cfg = dir.join("config.json");
            if cfg.exists() {
                if let Ok(raw) = fs::read_to_string(&cfg) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                        if let Some(rpc) = v.get("rpc").and_then(|x| x.as_str()) {
                            let t = rpc.trim().trim_end_matches('/').to_string();
                            if !t.is_empty() {
                                return t;
                            }
                        }
                    }
                }
            }
        }
    }
    let _ = app;
    "http://127.0.0.1:18080".into()
}

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    Ok(json!({
        "rpc": default_rpc_url(&app),
    }))
}

fn wallet_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("wallet.key"))
}

fn load_or_create_wallet(app: &tauri::AppHandle) -> Result<Keypair, String> {
    let path = wallet_path(app)?;
    if path.exists() {
        let hex = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        return Keypair::from_hex(hex.trim()).map_err(|e| e.to_string());
    }
    let kp = Keypair::generate();
    fs::write(&path, kp.to_hex()).map_err(|e| e.to_string())?;
    Ok(kp)
}

fn rpc_get<T: for<'de> Deserialize<'de>>(rpc_url: &str, path: &str) -> Result<T, String> {
    let url = format!("{}{}", rpc_url.trim_end_matches('/'), path);
    let resp = ureq::get(&url).call().map_err(|e| format!("RPC GET {url}: {e}"))?;
    if !(200..300).contains(&resp.status()) {
        return Err(format!("RPC GET {url} returned {}", resp.status()));
    }
    resp.into_json().map_err(|e| e.to_string())
}

fn rpc_post<T: for<'de> Deserialize<'de>>(
    rpc_url: &str,
    path: &str,
    body: &serde_json::Value,
) -> Result<T, String> {
    let url = format!("{}{}", rpc_url.trim_end_matches('/'), path);
    let resp = ureq::post(&url)
        .send_json(body.clone())
        .map_err(|e| format!("RPC POST {url}: {e}"))?;
    if !(200..300).contains(&resp.status()) {
        return Err(format!("RPC POST {url} returned {}", resp.status()));
    }
    resp.into_json().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_wallet(app: tauri::AppHandle, rpc_url: Option<String>) -> Result<WalletView, String> {
    let rpc = rpc_url
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default_rpc_url(&app));
    let kp = load_or_create_wallet(&app)?;
    let addr = kp.address().to_string();
    let path = format!("/v1/getbalance?address={addr}");
    let bal: BalanceResp = rpc_get(&rpc, &path)?;
    Ok(WalletView {
        address: addr,
        balance: bal.balance,
    })
}

#[tauri::command]
fn get_node_info(app: tauri::AppHandle, rpc_url: Option<String>) -> Result<NodeInfoView, String> {
    let rpc = rpc_url
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default_rpc_url(&app));
    let info: NodeInfoResp = rpc_get(&rpc, "/v1/getnodeinfo")?;
    Ok(NodeInfoView {
        height: info.height,
        tip: info.tip,
        next_difficulty: info.next_difficulty,
        mempool: info.mempool,
        peer_id: info.peer_id,
        finalized_height: info.finalized_height,
        finality_active: info.finality_active,
    })
}

#[tauri::command]
fn send_mesh(
    app: tauri::AppHandle,
    rpc_url: Option<String>,
    to: String,
    amount: String,
    memo: String,
) -> Result<SendView, String> {
    let rpc = rpc_url
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default_rpc_url(&app));
    let kp = load_or_create_wallet(&app)?;
    let dest = Address::from_hex(&to).ok_or_else(|| "bad address".to_string())?;
    let amt = Amount::parse_mesh(&amount).ok_or_else(|| "bad amount".to_string())?;

    let path = format!("/v1/utxos?address={}", kp.address());
    let items: Vec<UtxoRpc> = rpc_get(&rpc, &path)?;
    let utxos = items
        .into_iter()
        .map(|u| {
            let txid = Hash::from_hex(&u.txid).map_err(|e| e.to_string())?;
            Ok((
                OutPoint::new(txid, u.vout),
                Utxo {
                    address: kp.address(),
                    amount: Amount::from_atomic(u.atomic),
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let tx = build_signed_payment(&kp, &utxos, dest, amt, memo).map_err(|e| e.to_string())?;
    let tx_hex = hex::encode(bincode::serialize(&tx).map_err(|e| e.to_string())?);
    let resp: SendResp = rpc_post(&rpc, "/v1/submittx", &json!({ "tx_hex": tx_hex }))?;
    Ok(SendView { txid: resp.txid })
}

#[tauri::command]
fn mine_blocks(
    app: tauri::AppHandle,
    rpc_url: Option<String>,
    blocks: u64,
) -> Result<MineView, String> {
    let rpc = rpc_url
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default_rpc_url(&app));
    let kp = load_or_create_wallet(&app)?;
    let resp: MineResp = rpc_post(
        &rpc,
        "/v1/mine",
        &json!({
            "blocks": blocks.max(1),
            "address": kp.address().to_string()
        }),
    )?;
    Ok(MineView {
        height: resp.height,
        tip: resp.tip,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            get_wallet,
            get_node_info,
            send_mesh,
            mine_blocks
        ])
        .run(tauri::generate_context!())
        .expect("error while running MonkeyMesh wallet");
}
