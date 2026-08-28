//! Quantum Research Guardians — evolving specialist models + absolute board (Build/26).
//!
//! Three legs (pqc / grover / harvest) train tiny deterministic MLPs on hardening
//! quantum-era curricula. Seed verifies by re-executing the same steps.

use blake3::Hasher;
use libm::{exp, tanh};
use serde::{Deserialize, Serialize};

use crate::protocol_sim::{eval_research_input, ResearchScores};
use crate::research::ResearchScenario;

pub const FEAT: usize = 24;
pub const HIDDEN: usize = 48;
pub const OUT: usize = 4;
pub const WEIGHTS_MAGIC: &[u8] = b"MESHQNTv1";
const WEIGHTS_FLOATS: usize = HIDDEN * FEAT + HIDDEN + OUT * HIDDEN + OUT;
pub const WEIGHTS_BLOB_LEN: usize = WEIGHTS_MAGIC.len() + 4 + WEIGHTS_FLOATS * 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantumId {
    Pqc,
    Grover,
    Harvest,
}

impl QuantumId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pqc => "pqc",
            Self::Grover => "grover",
            Self::Harvest => "harvest",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pqc" | "post_quantum" => Some(Self::Pqc),
            "grover" | "pow" | "search" => Some(Self::Grover),
            "harvest" | "hndl" | "secrecy" => Some(Self::Harvest),
            _ => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Pqc, Self::Grover, Self::Harvest]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pqc => "PQC",
            Self::Grover => "Grover",
            Self::Harvest => "Harvest",
        }
    }

    pub fn genesis_seed(self) -> u64 {
        match self {
            Self::Pqc => 0x5051_4352_4541_4459, // PQCREADY
            Self::Grover => 0x4752_4F56_4552_0000,
            Self::Harvest => 0x4841_5256_4553_5400,
        }
    }

    fn scenario(self) -> ResearchScenario {
        match self {
            Self::Pqc => ResearchScenario::QuantumPqc,
            Self::Grover => ResearchScenario::QuantumGrover,
            Self::Harvest => ResearchScenario::QuantumHarvest,
        }
    }
}

/// Absolute public Quantum Board (0–100 integers).
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct QuantumBoard {
    pub pqc: u8,
    pub grover: u8,
    pub secrecy: u8,
    pub readiness: u8,
    pub weakest: String,
    pub note: String,
    #[serde(default)]
    pub leg_epochs: QuantumEpochs,
    #[serde(default)]
    pub leg_smart: QuantumSmart,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct QuantumEpochs {
    pub pqc: u64,
    pub grover: u64,
    pub harvest: u64,
}

/// How “smart” each quantum guardian is right now (last train accuracy × 100).
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct QuantumSmart {
    pub pqc: u8,
    pub grover: u8,
    pub harvest: u8,
}

#[derive(Clone, Debug)]
pub struct QuantumTrainSpec {
    pub leg: QuantumId,
    pub epoch: u64,
    pub steps: u32,
    pub lr: f64,
    pub samples: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct QuantumTrainResult {
    pub loss: f64,
    pub accuracy: f64,
    pub weight_digest: [u8; 32],
    pub new_weights: Vec<u8>,
    pub output: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum QuantumError {
    #[error("bad quantum_train input")]
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

    fn from_weights(blob: &[u8]) -> Result<Self, QuantumError> {
        if blob.len() < WEIGHTS_BLOB_LEN || &blob[..WEIGHTS_MAGIC.len()] != WEIGHTS_MAGIC {
            return Err(QuantumError::BadWeights);
        }
        let mut o = WEIGHTS_MAGIC.len();
        let n = u32::from_le_bytes(blob[o..o + 4].try_into().unwrap()) as usize;
        o += 4;
        if n != WEIGHTS_FLOATS || blob.len() < o + n * 8 {
            return Err(QuantumError::BadWeights);
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

fn sample_pair(leg: QuantumId, epoch: u64, idx: u32) -> ([f64; FEAT], [f64; OUT]) {
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
    x[FEAT - 2] = hardness.clamp(0.0, 1.0);
    x[FEAT - 1] = ((epoch % 100) as f64) / 100.0;
    (x, target)
}

fn scores_to_target(leg: QuantumId, s: &ResearchScores) -> [f64; OUT] {
    match leg {
        QuantumId::Pqc => [
            s.primary,
            1.0 - s.detect_rate,
            1.0 - s.linkability * 0.5,
            s.primary * 0.7 + (1.0 - s.detect_rate) * 0.3,
        ],
        QuantumId::Grover => [
            s.primary,
            1.0 - s.orphan_risk,
            1.0 - s.backlog_ratio,
            s.primary * 0.8 + (1.0 - s.orphan_risk) * 0.2,
        ],
        QuantumId::Harvest => [
            s.primary,
            1.0 - s.linkability,
            1.0 - s.linkability * 0.85,
            s.primary * 0.6 + (1.0 - s.linkability) * 0.4,
        ],
    }
}

pub fn encode_quantum_job(
    leg: QuantumId,
    epoch: u64,
    steps: u32,
    lr_milli: u32,
    samples: u32,
    offset: u32,
) -> Vec<u8> {
    format!(
        "mesh-qtrain:v1:leg={}:epoch={epoch}:steps={steps}:lr_milli={lr_milli}:samples={samples}:offset={offset}",
        leg.as_str()
    )
    .into_bytes()
}

pub fn is_quantum_train(input: &[u8]) -> bool {
    std::str::from_utf8(input)
        .map(|s| s.starts_with("mesh-qtrain:v1:"))
        .unwrap_or(false)
}

pub fn parse_quantum_job(input: &[u8]) -> Result<QuantumTrainSpec, QuantumError> {
    let s = std::str::from_utf8(input).map_err(|_| QuantumError::BadInput)?;
    if !s.starts_with("mesh-qtrain:v1:") {
        return Err(QuantumError::BadInput);
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
                "leg" => leg = QuantumId::parse(v),
                "epoch" => epoch = v.parse().unwrap_or(epoch),
                "steps" => steps = v.parse().unwrap_or(steps),
                "lr_milli" => lr_milli = v.parse().unwrap_or(lr_milli),
                "samples" => samples = v.parse().unwrap_or(samples),
                "offset" => offset = v.parse().unwrap_or(offset),
                _ => {}
            }
        }
    }
    let leg = leg.ok_or(QuantumError::BadInput)?;
    steps = steps.clamp(1, 256);
    lr_milli = lr_milli.clamp(1, 500);
    samples = samples.clamp(8, 256);
    Ok(QuantumTrainSpec {
        leg,
        epoch,
        steps,
        lr: lr_milli as f64 / 1000.0,
        samples,
        offset,
    })
}

pub fn genesis_quantum_weights(leg: QuantumId) -> Vec<u8> {
    GuardianMlp::genesis(leg.genesis_seed()).to_weights()
}

pub fn run_quantum_train(weights: &[u8], input: &[u8]) -> Result<QuantumTrainResult, QuantumError> {
    let spec = parse_quantum_job(input)?;
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
    Ok(QuantumTrainResult {
        loss: last_loss,
        accuracy,
        weight_digest,
        new_weights: new_weights.clone(),
        output: new_weights,
    })
}

/// Build absolute quantum board from protocol sim trends + guardian smarts.
pub fn build_quantum_board(
    pqc_primary: f64,
    pqc_detect: f64,
    grover_primary: f64,
    grover_orphan: f64,
    grover_backlog: f64,
    harvest_primary: f64,
    harvest_linkability: f64,
    leg_epochs: QuantumEpochs,
    leg_smart: QuantumSmart,
) -> QuantumBoard {
    let pqc = pct(
        0.45 * pqc_primary
            + 0.35 * (1.0 - pqc_detect)
            + 0.10 * (1.0 - harvest_linkability * 0.3)
            + 0.10 * (leg_smart.pqc as f64 / 100.0),
    );
    let grover = pct(
        0.50 * grover_primary
            + 0.25 * (1.0 - grover_orphan)
            + 0.15 * (1.0 - grover_backlog)
            + 0.10 * (leg_smart.grover as f64 / 100.0),
    );
    let secrecy = pct(
        0.55 * harvest_primary
            + 0.35 * (1.0 - harvest_linkability)
            + 0.10 * (leg_smart.harvest as f64 / 100.0),
    );

    let readiness = pqc.min(grover).min(secrecy);

    let weakest = {
        let mut pairs = [
            (pqc, "pqc"),
            (grover, "grover"),
            (secrecy, "harvest"),
        ];
        pairs.sort_by_key(|(v, _)| *v);
        pairs[0].1.to_string()
    };

    QuantumBoard {
        pqc,
        grover,
        secrecy,
        readiness,
        weakest,
        note: "Absolute 0–100 quantum readiness — feed the weakest leg; readiness is the min needle"
            .into(),
        leg_epochs,
        leg_smart,
    }
}

fn pct(x: f64) -> u8 {
    (x.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// Prefer training order: board.weakest first, then others by ascending smart + needle.
pub fn quantum_priority(board: &QuantumBoard) -> Vec<QuantumId> {
    let mut legs = QuantumId::all().to_vec();
    legs.sort_by_key(|l| {
        let smart = match l {
            QuantumId::Pqc => board.leg_smart.pqc,
            QuantumId::Grover => board.leg_smart.grover,
            QuantumId::Harvest => board.leg_smart.harvest,
        };
        let needle = match l {
            QuantumId::Pqc => board.pqc,
            QuantumId::Grover => board.grover,
            QuantumId::Harvest => board.secrecy,
        };
        (needle as u16) * 2 + smart as u16
    });
    if let Some(first) = QuantumId::parse(&board.weakest) {
        legs.retain(|l| *l != first);
        legs.insert(0, first);
    }
    legs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantum_train_deterministic() {
        let leg = QuantumId::Pqc;
        let w0 = genesis_quantum_weights(leg);
        let job = encode_quantum_job(leg, 0, 8, 50, 16, 0);
        let a = run_quantum_train(&w0, &job).unwrap();
        let b = run_quantum_train(&w0, &job).unwrap();
        assert_eq!(a.output, b.output);
        assert_ne!(a.weight_digest, weights_digest(&w0));
    }

    #[test]
    fn board_readiness_is_min() {
        let b = build_quantum_board(
            0.8,
            0.2,
            0.7,
            0.15,
            0.1,
            0.6,
            0.3,
            QuantumEpochs::default(),
            QuantumSmart {
                pqc: 70,
                grover: 65,
                harvest: 60,
            },
        );
        assert_eq!(b.readiness, b.pqc.min(b.grover).min(b.secrecy));
        assert!(!b.weakest.is_empty());
    }
}
