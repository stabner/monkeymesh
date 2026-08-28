//! Immune-exam sidecar: run the template's protocol sim and POST the digest.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use mesh_ai::run_protocol_eval;
use mesh_types::Address;

use crate::engine::MinerEvent;

static LAST_EXAM_HEIGHT: Mutex<u64> = Mutex::new(0);

#[derive(Clone, Debug)]
pub struct ExamHint {
    pub height: u64,
    pub scenario: String,
    pub title: String,
    pub payload_hex: String,
    pub job_id: String,
}

pub fn try_submit_exam(
    rpc: &str,
    payout: &Address,
    hint: &ExamHint,
    tx: &std::sync::mpsc::Sender<MinerEvent>,
) {
    if hint.payload_hex.is_empty() || hint.height == 0 {
        return;
    }
    {
        let mut g = LAST_EXAM_HEIGHT.lock().unwrap_or_else(|e| e.into_inner());
        if *g == hint.height {
            return;
        }
        *g = hint.height;
    }
    let payload = match hex::decode(&hint.payload_hex) {
        Ok(p) => p,
        Err(_) => {
            let _ = tx.send(MinerEvent::Error("exam payload hex invalid".into()));
            return;
        }
    };
    let t0 = Instant::now();
    let digest = run_protocol_eval(&payload);
    let latency_ms = t0.elapsed().as_millis() as u64;
    let body = serde_json::json!({
        "address": payout.to_hex(),
        "height": hint.height,
        "digest_hex": hex::encode(digest),
        "latency_ms": latency_ms,
    });
    let mut urls = vec![rpc.trim_end_matches('/').to_string()];
    for u in mesh_types::default_rpc_urls() {
        if !urls.iter().any(|x| x == &u) {
            urls.push(u);
        }
    }
    let mut last_err = String::new();
    for base in urls {
        let url = format!("{base}/v1/exam/submit");
        match ureq::post(&url)
            .timeout(Duration::from_secs(15))
            .send_json(&body)
        {
            Ok(resp) => {
                let ok = resp.status() >= 200 && resp.status() < 300;
                if let Ok(v) = resp.into_json::<serde_json::Value>() {
                    if v.get("ok").and_then(|x| x.as_bool()).unwrap_or(ok) {
                        let title = v
                            .get("title")
                            .and_then(|x| x.as_str())
                            .unwrap_or(hint.title.as_str());
                        let rematch = v.get("rematch_ms").and_then(|x| x.as_u64()).unwrap_or(0);
                        let _ = tx.send(MinerEvent::AiJobDone {
                            job_id: hint.job_id.clone(),
                            kind: format!("exam/{}", hint.scenario),
                            brain_epoch: None,
                        });
                        let _ = tx.send(MinerEvent::Status(format!(
                            "Exam {} · {title} · rematch {rematch} ms MATCH",
                            hint.height
                        )));
                        return;
                    }
                    last_err = v
                        .get("message")
                        .or_else(|| v.get("error"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("exam rejected")
                        .to_string();
                    continue;
                }
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    if !last_err.is_empty() {
        let _ = tx.send(MinerEvent::Status(format!("Exam: {last_err}")));
    }
}
