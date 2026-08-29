//! CPU rematch of GPU-produced AI board offers (`GET /v1/result/pending`).

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use mesh_ai::{
    is_leg_train, is_quantum_train, parse_leg_job, parse_quantum_job, rematch_board_output,
};
use serde::Deserialize;

use crate::engine::MinerEvent;

#[derive(Deserialize)]
struct PendingResp {
    #[serde(default)]
    pending: Vec<PendingOffer>,
}

#[derive(Deserialize, Clone)]
struct PendingOffer {
    job_id: String,
    kind: String,
    input_hex: String,
    #[serde(default)]
    producer: String,
}

/// Poll seed offers and rematch until `stop`.
pub fn run_seal_loop(rpc_list: String, address: String, stop: std::sync::Arc<AtomicBool>, tx: Sender<MinerEvent>) {
    let mut urls = mesh_types::parse_rpc_list(&rpc_list);
    if urls.is_empty() {
        urls = mesh_types::default_rpc_urls();
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(4))
        .timeout_read(Duration::from_secs(60))
        .build();
    let mut i = 0usize;
    while !stop.load(Ordering::SeqCst) {
        let rpc = urls[i % urls.len()].trim_end_matches('/').to_string();
        match try_seal_one(&agent, &rpc, &address) {
            Ok(Some(job_id)) => {
                let _ = tx.send(MinerEvent::AiJobDone {
                    job_id: job_id.clone(),
                    kind: "seal".into(),
                    brain_epoch: None,
                });
                let _ = tx.send(MinerEvent::Status(format!("CPU sealed AI job {job_id}")));
            }
            Ok(None) => {}
            Err(e) => {
                let low = e.to_ascii_lowercase();
                if !low.contains("no job") && !low.contains("204") && !low.contains("empty") {
                    tracing::debug!(%rpc, "seal: {e}");
                }
                i = i.wrapping_add(1);
            }
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn try_seal_one(agent: &ureq::Agent, rpc: &str, address: &str) -> Result<Option<String>, String> {
    let url = format!("{rpc}/v1/result/pending");
    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| e.to_string())?;
    let body: PendingResp = resp.into_json().map_err(|e| e.to_string())?;
    let Some(offer) = body
        .pending
        .into_iter()
        .find(|o| !o.job_id.is_empty() && o.producer != address)
    else {
        return Ok(None);
    };
    let input = hex::decode(&offer.input_hex).map_err(|e| e.to_string())?;
    let weights = fetch_weights(agent, rpc, &offer.kind, &input)?;
    let output = rematch_board_output(&offer.kind, &input, weights.as_deref())?;
    let rematch_url = format!("{rpc}/v1/result/rematch");
    let body = serde_json::json!({
        "address": address,
        "job_id": offer.job_id,
        "output_hex": hex::encode(&output),
    });
    let resp = agent
        .post(&rematch_url)
        .send_json(&body)
        .map_err(|e| e.to_string())?;
    if resp.status() == 409 {
        return Ok(None);
    }
    if resp.status() >= 300 {
        return Err(format!("rematch HTTP {}", resp.status()));
    }
    Ok(Some(offer.job_id))
}

fn fetch_weights(
    agent: &ureq::Agent,
    rpc: &str,
    kind: &str,
    input: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    let path = match kind {
        "ml_train_shared" => Some(format!("{rpc}/v1/model/bin?ver=1")),
        "ml_train_shared_v2" => Some(format!("{rpc}/v1/model/bin?ver=2")),
        "leg_train" if is_leg_train(input) => {
            let spec = parse_leg_job(input).map_err(|e| e.to_string())?;
            Some(format!("{rpc}/v1/leg/{}/bin", spec.leg.as_str()))
        }
        "quantum_train" if is_quantum_train(input) => {
            let spec = parse_quantum_job(input).map_err(|e| e.to_string())?;
            Some(format!("{rpc}/v1/qleg/{}/bin", spec.leg.as_str()))
        }
        _ => None,
    };
    let Some(url) = path else {
        return Ok(None);
    };
    let resp = agent.get(&url).call().map_err(|e| e.to_string())?;
    if resp.status() >= 300 {
        return Err(format!("weights HTTP {}", resp.status()));
    }
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("empty weights".into());
    }
    Ok(Some(bytes))
}
