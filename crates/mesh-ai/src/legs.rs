//! Trilemma Guardians — evolving specialist models + absolute board (Build/25).
//!
//! Four legs (security / network / blocks / transpar) train tiny deterministic MLPs
//! on hardening adversarial curricula. Seed verifies by re-executing the same steps.

use blake3::Hasher;
use libm::{exp, tanh};
use serde::{Deserialize, Serialize};

use crate::protocol_sim::{eval_research_input, ResearchScores};
use crate::research::ResearchScenario;

pub const FEAT: usize = 24;
pub const HIDDEN: usize = 48;
pub const OUT: usize = 4;
pub const WEIGHTS_MAGIC: &[u8] = b"MESHLEGv1";
const WEIGHTS_FLOATS: usize = HIDDEN * FEAT + HIDDEN + OUT * HIDDEN + OUT;
pub const WEIGHTS_BLOB_LEN: usize = WEIGHTS_MAGIC.len() + 4 + WEIGHTS_FLOATS * 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegId {
    Security,
    Network,
    Blocks,
    Transpar,
}

impl LegId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::Network => "network",
            Self::Blocks => "blocks",
            Self::Transpar => "transpar",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "security" | "sec" => Some(Self::Security),
            "network" | "net" | "scale" => Some(Self::Network),
            "blocks" | "block" | "chain" => Some(Self::Blocks),
            "transpar" | "transparency" | "privacy" => Some(Self::Transpar),
            _ => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Security, Self::Network, Self::Blocks, Self::Transpar]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Security => "Security",
            Self::Network => "Network",
            Self::Blocks => "Blocks",
            Self::Transpar => "Transparency",
        }
    }

    pub fn genesis_seed(self) -> u64 {
        match self {
            Self::Security => 0x5345_4355_5249_5459, // SECURITYish
            Self::Network => 0x4E45_5457_4F52_4B00,
            Self::Blocks => 0x424C_4F43_4B53_0000,
            Self::Transpar => 0x5452_414E_5350_4152,
        }
    }

    fn scenario(self) -> ResearchScenario {
        match self {
            Self::Security => ResearchScenario::SecurityAdversary,
            Self::Network => ResearchScenario::ScaleThroughput,
            Self::Blocks => ResearchScenario::BlockPropagation,
            Self::Transpar => ResearchScenario::PrivacyLeakage,
        }
    }
}

/// Absolute public Trilemma Board (0–100 integers).
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TrilemmaBoard {
    pub sec: u8,
    pub scale: u8,
    pub decent: u8,
    pub transpar: u8,
    /// 100 * min(sec,scale,decent) / max(sec,scale,decent) — evenness of the classic triangle.
    pub balance: u8,
    pub weakest: String,
    pub note: String,
    #[serde(default)]
    pub leg_epochs: LegEpochs,
    #[serde(default)]
    pub leg_smart: LegSmart,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LegEpochs {
    pub security: u64,
    pub network: u64,
    pub blocks: u64,
    pub transpar: u64,
}

/// How “smart” each guardian is right now (last train accuracy × 100).
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LegSmart {
    pub security: u8,
    pub network: u8,
    pub blocks: u8,
    pub transpar: u8,
}

#[derive(Clone, Debug)]
pub struct LegTrainSpec {
    pub leg: LegId,
    pub epoch: u64,
    pub steps: u32,
    pub lr: f64,
    pub samples: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct LegTrainResult {
    pub loss: f64,
    pub accuracy: f64,
    pub weight_digest: [u8; 32],
    pub new_weights: Vec<u8>,
    pub output: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum LegError {
    #[error("bad leg_train input")]
    BadInput,
    #[error("bad weight blob")]
    BadWeights,
}

struct GuardianMlp {
    w1: Vec<f64>,
    b1: Vec<f64>,
    w2: Vec<f64>,
    b2: Vec<f64>,
}

impl GuardianMlp {
    fn genesis(seed: u64) -> Self {
        let mut w1 = vec![0.0; HIDDEN * FEAT];
        for (i, w) in w1.iter_mut().enumerate() {
            *w = seed_f(seed, 1, i as u64) * 0.08;
        }
        let mut b1 = vec![0.0; HIDDEN];
        for (i, b) in b1.iter_mut().enumerate() {
            *b = seed_f(seed, 2, i as u64) * 0.02;
        }
        let mut w2 = vec![0.0; OUT * HIDDEN];
        for (i, w) in w2.iter_mut().enumerate() {
            *w = seed_f(seed, 3, i as u64) * 0.08;
        }
        let mut b2 = vec![0.0; OUT];
        for (i, b) in b2.iter_mut().enumerate() {
            *b = seed_f(seed, 4, i as u64) * 0.02;
        }
        Self { w1, b1, w2, b2 }
    }

    fn from_weights(blob: &[u8]) -> Result<Self, LegError> {
        if blob.len() < WEIGHTS_BLOB_LEN || &blob[..WEIGHTS_MAGIC.len()] != WEIGHTS_MAGIC {
            return Err(LegError::BadWeights);
        }
        let mut o = WEIGHTS_MAGIC.len();
        let n = u32::from_le_bytes(blob[o..o + 4].try_into().unwrap()) as usize;
        o += 4;
        if n != WEIGHTS_FLOATS || blob.len() < o + n * 8 {
            return Err(LegError::BadWeights);
        }
        let mut read = |len: usize| -> Vec<f64> {
            let mut v = Vec::with_capacity(len);
            for _ in 0..len {
                let bits = u64::from_le_bytes(blob[o..o + 8].try_into().unwrap());
                o += 8;
                v.push(f64::from_bits(bits));
            }
            v
        };
        Ok(Self {
            w1: read(HIDDEN * FEAT),
            b1: read(HIDDEN),
            w2: read(OUT * HIDDEN),
            b2: read(OUT),
        })
    }

    fn to_weights(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(WEIGHTS_BLOB_LEN);
        out.extend_from_slice(WEIGHTS_MAGIC);
        out.extend_from_slice(&(WEIGHTS_FLOATS as u32).to_le_bytes());
        for x in self
            .w1
            .iter()
            .chain(self.b1.iter())
            .chain(self.w2.iter())
            .chain(self.b2.iter())
        {
            out.extend_from_slice(&x.to_bits().to_le_bytes());
        }
        out
    }

    fn forward(&self, x: &[f64; FEAT]) -> ([f64; HIDDEN], [f64; OUT]) {
        let mut h = [0.0; HIDDEN];
        for j in 0..HIDDEN {
            let mut s = self.b1[j];
            let row = j * FEAT;
            for i in 0..FEAT {
                s += self.w1[row + i] * x[i];
            }
            h[j] = tanh(s);
        }
        let mut y = [0.0; OUT];
        for c in 0..OUT {
            let mut s = self.b2[c];
            let row = c * HIDDEN;
            for j in 0..HIDDEN {
                s += self.w2[row + j] * h[j];
            }
            y[c] = 1.0 / (1.0 + exp(-s.clamp(-20.0, 20.0)));
        }
        (h, y)
    }

    fn train_step(&mut self, x: &[f64; FEAT], target: &[f64; OUT], lr: f64) -> f64 {
        let (h, y) = self.forward(x);
        let mut loss = 0.0;
        let mut dy = [0.0; OUT];
        for c in 0..OUT {
            let err = y[c] - target[c];
            loss += err * err;
            // sigmoid grad
            dy[c] = err * y[c] * (1.0 - y[c]);
        }
        let mut dh = [0.0; HIDDEN];
        for c in 0..OUT {
            let row = c * HIDDEN;
            for j in 0..HIDDEN {
                dh[j] += dy[c] * self.w2[row + j];
            }
        }
        for c in 0..OUT {
            let row = c * HIDDEN;
            for j in 0..HIDDEN {
                self.w2[row + j] -= lr * dy[c] * h[j];
            }
            self.b2[c] -= lr * dy[c];
        }
        for j in 0..HIDDEN {
            let grad_h = dh[j] * (1.0 - h[j] * h[j]);
            let row = j * FEAT;
            for i in 0..FEAT {
                self.w1[row + i] -= lr * grad_h * x[i];
            }
            self.b1[j] -= lr * grad_h;
        }
        loss
    }

    fn eval_accuracy(&self, xs: &[[f64; FEAT]], ys: &[[f64; OUT]]) -> f64 {
        if xs.is_empty() {
            return 0.0;
        }
        let mut ok = 0.0;
        for (x, t) in xs.iter().zip(ys.iter()) {
            let (_, y) = self.forward(x);
            let mut good = true;
            for c in 0..OUT {
                if (y[c] - t[c]).abs() > 0.20 {
                    good = false;
                    break;
                }
            }
            if good {
                ok += 1.0;
            }
        }
        ok / xs.len() as f64
    }
}

fn seed_f(seed: u64, lane: u64, i: u64) -> f64 {
    let mut h = Hasher::new();
    h.update(&seed.to_le_bytes());
    h.update(&lane.to_le_bytes());
    h.update(&i.to_le_bytes());
    let b = *h.finalize().as_bytes();
    let u = u64::from_le_bytes(b[..8].try_into().unwrap());
    (u as f64 / u64::MAX as f64) * 2.0 - 1.0
}

fn weights_digest(weights: &[u8]) -> [u8; 32] {
    *blake3::hash(weights).as_bytes()
}

/// Hardening curriculum: features + labels from protocol sims at rising adversarial intensity.
fn sample_pair(leg: LegId, epoch: u64, idx: u32) -> ([f64; FEAT], [f64; OUT]) {
    let hardness = 0.15 + (epoch as f64 * 0.01).min(0.75) + (idx as f64 % 17.0) * 0.01;
    let height = 100 + epoch.saturating_mul(3) + idx as u64;
    let pulse = hardness.clamp(0.05, 2.5);
    let scen = leg.scenario();
    let input = scen.encode(height, pulse);
    let scores = eval_research_input(&input)
        .map(|r| r.scores)
        .unwrap_or_default();
    let target = scores_to_target(leg, &scores);
    let mut x = [0.0; FEAT];
    for i in 0..FEAT {
        let base = seed_f(leg.genesis_seed(), epoch.wrapping_add(9), idx as u64 * 64 + i as u64);
        let mix = match i % 6 {
            0 => scores.primary,
            1 => scores.orphan_risk,
            2 => scores.detect_rate,
            3 => scores.linkability,
            4 => scores.backlog_ratio,
            _ => (scores.latency_p95_ms / 5_000.0).clamp(0.0, 1.0),
        };
        x[i] = (0.55 * mix + 0.45 * ((base + 1.0) * 0.5) + hardness * 0.05).clamp(0.0, 1.0);
    }
    // Stamp hardness + epoch into last dims so model sees curriculum phase.
    x[FEAT - 2] = hardness.clamp(0.0, 1.0);
    x[FEAT - 1] = ((epoch % 100) as f64) / 100.0;
    (x, target)
}

fn scores_to_target(leg: LegId, s: &ResearchScores) -> [f64; OUT] {
    match leg {
        LegId::Security => [
            s.detect_rate,
            1.0 - s.orphan_risk * 0.5,
            s.primary,
            1.0 - (1.0 - s.detect_rate) * 0.8,
        ],
        LegId::Network => [
            1.0 - s.backlog_ratio,
            1.0 - (s.latency_p95_ms / 5_000.0).clamp(0.0, 1.0),
            s.primary,
            1.0 - s.backlog_ratio * 0.7,
        ],
        LegId::Blocks => [
            1.0 - s.orphan_risk,
            s.primary,
            1.0 - s.backlog_ratio * 0.4,
            1.0 - s.orphan_risk * 0.8,
        ],
        LegId::Transpar => [
            1.0 - s.linkability,
            s.primary,
            s.detect_rate * 0.5 + (1.0 - s.linkability) * 0.5,
            1.0 - s.linkability * 0.9,
        ],
    }
}

pub fn encode_leg_job(
    leg: LegId,
    epoch: u64,
    steps: u32,
    lr_milli: u32,
    samples: u32,
    offset: u32,
) -> Vec<u8> {
    format!(
        "mesh-legtrain:v1:leg={}:epoch={epoch}:steps={steps}:lr_milli={lr_milli}:samples={samples}:offset={offset}",
        leg.as_str()
    )
    .into_bytes()
}

pub fn is_leg_train(input: &[u8]) -> bool {
    std::str::from_utf8(input)
        .map(|s| s.starts_with("mesh-legtrain:v1:"))
        .unwrap_or(false)
}

pub fn parse_leg_job(input: &[u8]) -> Result<LegTrainSpec, LegError> {
    let s = std::str::from_utf8(input).map_err(|_| LegError::BadInput)?;
    if !s.starts_with("mesh-legtrain:v1:") {
        return Err(LegError::BadInput);
    }
    let mut leg = None;
    let mut epoch = 0u64;
    let mut steps = 32u32;
    let mut lr_milli = 50u32;
    let mut samples = 64u32;
    let mut offset = 0u32;
    for part in s.split(':').skip(2) {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "leg" => leg = LegId::parse(v),
                "epoch" => epoch = v.parse().unwrap_or(epoch),
                "steps" => steps = v.parse().unwrap_or(steps),
                "lr_milli" => lr_milli = v.parse().unwrap_or(lr_milli),
                "samples" => samples = v.parse().unwrap_or(samples),
                "offset" => offset = v.parse().unwrap_or(offset),
                _ => {}
            }
        }
    }
    let leg = leg.ok_or(LegError::BadInput)?;
    steps = steps.clamp(1, 256);
    lr_milli = lr_milli.clamp(1, 500);
    samples = samples.clamp(8, 256);
    Ok(LegTrainSpec {
        leg,
        epoch,
        steps,
        lr: lr_milli as f64 / 1000.0,
        samples,
        offset,
    })
}

pub fn genesis_leg_weights(leg: LegId) -> Vec<u8> {
    GuardianMlp::genesis(leg.genesis_seed()).to_weights()
}

pub fn run_leg_train(weights: &[u8], input: &[u8]) -> Result<LegTrainResult, LegError> {
    let spec = parse_leg_job(input)?;
    let mut model = GuardianMlp::from_weights(weights)?;
    let mut last_loss = 0.0;
    for step in 0..spec.steps {
        let idx = spec.offset.wrapping_add(step) % spec.samples.max(1);
        let (x, y) = sample_pair(spec.leg, spec.epoch, idx);
        last_loss = model.train_step(&x, &y, spec.lr);
    }
    let eval_n = spec.samples.min(32);
    let mut xs = Vec::with_capacity(eval_n as usize);
    let mut ys = Vec::with_capacity(eval_n as usize);
    for i in 0..eval_n {
        let (x, y) = sample_pair(spec.leg, spec.epoch, spec.offset.wrapping_add(i));
        xs.push(x);
        ys.push(y);
    }
    let accuracy = model.eval_accuracy(&xs, &ys);
    let new_weights = model.to_weights();
    let weight_digest = weights_digest(&new_weights);
    Ok(LegTrainResult {
        loss: last_loss,
        accuracy,
        weight_digest,
        new_weights: new_weights.clone(),
        output: new_weights,
    })
}

/// Build absolute board from receipt trends + live chain/node counts + guardian smarts.
pub fn build_trilemma_board(
    mean_detect: f64,
    mean_orphan: f64,
    mean_backlog: f64,
    mean_latency_ms: f64,
    mean_linkability: f64,
    mean_primary: f64,
    distinct_nodes: u32,
    distinct_gpu_workers: u32,
    leg_epochs: LegEpochs,
    leg_smart: LegSmart,
) -> TrilemmaBoard {
    let sec = pct(
        0.45 * mean_detect
            + 0.25 * mean_primary
            + 0.20 * (1.0 - mean_orphan)
            + 0.10 * (leg_smart.security as f64 / 100.0),
    );
    let scale = pct(
        0.40 * (1.0 - mean_backlog)
            + 0.30 * (1.0 - (mean_latency_ms / 5_000.0).clamp(0.0, 1.0))
            + 0.20 * mean_primary
            + 0.10 * (leg_smart.network as f64 / 100.0),
    );
    // Decentralization proxy: more independent nodes + GPU workers → higher.
    let node_score = (distinct_nodes as f64 / 12.0).clamp(0.0, 1.0);
    let gpu_score = (distinct_gpu_workers as f64 / 8.0).clamp(0.0, 1.0);
    let decent = pct(0.55 * node_score + 0.35 * gpu_score + 0.10 * mean_primary);
    let transpar = pct(
        0.55 * (1.0 - mean_linkability)
            + 0.25 * mean_primary
            + 0.20 * (leg_smart.transpar as f64 / 100.0),
    );

    let core = [sec, scale, decent];
    let min_c = *core.iter().min().unwrap_or(&0);
    let max_c = *core.iter().max().unwrap_or(&1).max(&1);
    let balance = ((min_c as u32 * 100) / max_c as u32) as u8;

    let weakest = {
        let mut pairs = [
            (sec, "security"),
            (scale, "network"),
            (decent, "decent"),
            (transpar, "transpar"),
        ];
        pairs.sort_by_key(|(v, _)| *v);
        pairs[0].1.to_string()
    };

    TrilemmaBoard {
        sec,
        scale,
        decent,
        transpar,
        balance,
        weakest,
        note: "Absolute 0–100 needles — feed the weakest leg; balance near 100 means even triangle"
            .into(),
        leg_epochs,
        leg_smart,
    }
}

fn pct(x: f64) -> u8 {
    (x.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// Prefer training order: map board.weakest → LegId, then others by ascending smart.
pub fn legs_priority(board: &TrilemmaBoard) -> Vec<LegId> {
    let mut legs = LegId::all().to_vec();
    legs.sort_by_key(|l| {
        let smart = match l {
            LegId::Security => board.leg_smart.security,
            LegId::Network => board.leg_smart.network,
            LegId::Blocks => board.leg_smart.blocks,
            LegId::Transpar => board.leg_smart.transpar,
        };
        let needle = match l {
            LegId::Security => board.sec,
            LegId::Network => board.scale,
            LegId::Blocks => board.sec.min(board.scale), // blocks pressure both
            LegId::Transpar => board.transpar,
        };
        // Weak needle + less-smart guardian first.
        (needle as u16) * 2 + smart as u16
    });
    // Force named weakest first when it maps to a leg.
    if let Some(first) = LegId::parse(&board.weakest) {
        legs.retain(|l| *l != first);
        legs.insert(0, first);
    }
    legs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leg_train_deterministic() {
        let leg = LegId::Security;
        let w0 = genesis_leg_weights(leg);
        let job = encode_leg_job(leg, 0, 8, 50, 16, 0);
        let a = run_leg_train(&w0, &job).unwrap();
        let b = run_leg_train(&w0, &job).unwrap();
        assert_eq!(a.output, b.output);
        assert_ne!(a.weight_digest, weights_digest(&w0));
    }

    #[test]
    fn board_balance_bounds() {
        let b = build_trilemma_board(
            0.9,
            0.1,
            0.1,
            100.0,
            0.1,
            0.8,
            10,
            6,
            LegEpochs::default(),
            LegSmart {
                security: 70,
                network: 60,
                blocks: 55,
                transpar: 65,
            },
        );
        assert!(b.sec >= 50);
        assert!(b.balance <= 100);
        assert!(!b.weakest.is_empty());
    }
}
