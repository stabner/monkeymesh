use anyhow::{bail, Context, Result};
use mesh_chain::build_signed_payment;
use mesh_crypto::Keypair;
use mesh_types::{Address, Amount, Hash, OutPoint, Utxo};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone, Deserialize)]
pub struct NodeInfo {
    pub height: u64,
    pub tip: String,
    pub next_difficulty: u32,
    pub mempool: usize,
    pub peer_id: Option<String>,
    #[serde(default)]
    pub finalized_height: u64,
    #[serde(default)]
    pub finalized_hash: String,
    #[serde(default)]
    pub finality_active: bool,
    #[serde(default)]
    pub peers: usize,
    #[serde(default)]
    pub genesis: String,
    #[serde(default)]
    pub supply_cap_mesh: u64,
    #[serde(default)]
    pub emitted_atomic: String,
    #[serde(default)]
    pub coinbase_maturity: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TxRow {
    pub height: Option<u64>,
    #[serde(default)]
    pub timestamp: Option<u64>,
    pub txid: String,
    pub memo: String,
    pub outputs: Vec<OutRow>,
    pub in_mempool: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutRow {
    pub address: String,
    pub amount: String,
    #[serde(default)]
    pub atomic: u64,
    #[serde(default)]
    pub lane: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub paid_for: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RewardsView {
    #[serde(default)]
    pub rewards: String,
    #[serde(default)]
    pub atomic: u64,
    #[serde(default)]
    pub by_lane: Vec<LaneRow>,
    #[serde(default)]
    pub recent: Vec<PayoutRow>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LaneRow {
    #[serde(default)]
    pub lane: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub paid_for: String,
    #[serde(default)]
    pub amount: String,
    #[serde(default)]
    pub atomic: u64,
    #[serde(default)]
    pub count: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PayoutRow {
    #[serde(default)]
    pub height: u64,
    #[serde(default)]
    pub timestamp: u64,
    #[serde(default)]
    pub amount: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub paid_for: String,
    #[serde(default)]
    pub mature: bool,
    #[serde(default)]
    pub confirmations: u64,
    #[serde(default)]
    pub vault: bool,
}

#[derive(Deserialize)]
struct BalanceResp {
    balance: String,
    #[serde(default)]
    spendable: Option<String>,
}

#[derive(Deserialize)]
struct UtxoRpc {
    txid: String,
    vout: u32,
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
    height: u64,
    tip: String,
}

pub struct RpcClient {
    base: String,
}

impl RpcClient {
    pub fn new(rpc: &str) -> Self {
        Self {
            base: rpc.trim().trim_end_matches('/').to_string(),
        }
    }

    /// First candidate that answers `getnodeinfo` (2s each). Falls back to the first entry.
    pub fn pick_live(candidates: &[String]) -> String {
        for u in candidates {
            let url = format!("{u}/v1/getnodeinfo");
            if let Ok(resp) = ureq::get(&url)
                .timeout(std::time::Duration::from_secs(2))
                .call()
            {
                if (200..300).contains(&resp.status()) {
                    return u.clone();
                }
            }
        }
        candidates
            .first()
            .cloned()
            .unwrap_or_else(mesh_types::default_seed_rpc_url)
    }

    pub fn refresh(
        &self,
        address: &str,
    ) -> Result<(NodeInfo, String, Option<String>, Vec<TxRow>, RewardsView)> {
        let info: NodeInfo = self.get("/v1/getnodeinfo")?;
        let bal: BalanceResp = self.get(&format!("/v1/getbalance?address={address}"))?;
        let txs: Vec<TxRow> = self.get(&format!("/v1/listtransactions?address={address}"))?;
        let rewards = self
            .get::<RewardsView>(&format!("/v1/getrewards?address={address}"))
            .unwrap_or_default();
        Ok((info, bal.balance, bal.spendable, txs, rewards))
    }

    pub fn send(&self, key: &Keypair, to: &str, amount: &str, memo: &str) -> Result<String> {
        let dest = Address::from_hex(to).context("bad address")?;
        let amt = Amount::parse_mesh(amount).context("bad amount")?;
        let items: Vec<UtxoRpc> =
            self.get(&format!("/v1/utxos?address={}", key.address()))?;
        let utxos = items
            .into_iter()
            .filter(|u| u.mature.unwrap_or(true))
            .map(|u| {
                let txid = Hash::from_hex(&u.txid)?;
                Ok((
                    OutPoint::new(txid, u.vout),
                    Utxo {
                        address: key.address(),
                        amount: Amount::from_atomic(u.atomic),
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let tx = build_signed_payment(key, &utxos, dest, amt, memo.to_string())?;
        let tx_hex = hex::encode(bincode::serialize(&tx)?);
        let resp: SendResp = self.post("/v1/submittx", &json!({ "tx_hex": tx_hex }))?;
        Ok(resp.txid)
    }

    pub fn mine(&self, address: &str, blocks: u64) -> Result<(u64, String)> {
        let resp: MineResp = self.post(
            "/v1/mine",
            &json!({
                "blocks": blocks.max(1),
                "address": address,
            }),
        )?;
        Ok((resp.height, resp.tip))
    }

    fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let url = format!("{}{path}", self.base);
        let resp = ureq::get(&url)
            .timeout(std::time::Duration::from_secs(30))
            .call()
            .with_context(|| format!("GET {url}"))?;
        if !(200..300).contains(&resp.status()) {
            bail!("GET {url} -> {}", resp.status());
        }
        Ok(resp.into_json()?)
    }

    fn post<T: for<'de> Deserialize<'de>>(&self, path: &str, body: &serde_json::Value) -> Result<T> {
        let url = format!("{}{path}", self.base);
        let mut req = ureq::post(&url).timeout(std::time::Duration::from_secs(300));
        if let Ok(token) = std::env::var("MESH_RPC_TOKEN") {
            let t = token.trim();
            if !t.is_empty() {
                req = req.set("X-Mesh-Token", t);
            }
        }
        let resp = req
            .send_json(body.clone())
            .with_context(|| format!("POST {url}"))?;
        if !(200..300).contains(&resp.status()) {
            let status = resp.status();
            let text = resp.into_string().unwrap_or_default();
            bail!("POST {url} -> {status}: {text}");
        }
        Ok(resp.into_json()?)
    }
}
