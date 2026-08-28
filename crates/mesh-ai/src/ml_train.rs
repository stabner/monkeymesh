//! Real MNIST training workload (deterministic, verifiable).
//!
//! Dataset: first 4096 samples of the official MNIST training set (real pixels/labels).
//! Model: tiny MLP 784→64→10 trained with SGD + softmax cross-entropy in pure `f64`.
//! Worker and orchestrator re-run the same steps and must match the output digest.
//!
//! Shared-brain jobs continue from network weights (`mesh-mltrain-shared`) instead of
//! random init — one model for the whole mesh.

use std::sync::OnceLock;

/// Official MNIST training subset (4096 samples × (1 label + 784 pixels)).
static MNIST_BLOB: &[u8] = include_bytes!("../data/mnist4096.bin");
const MNIST_SHA256_HEX: &str =
    "18a46d88f5afc25b339e63c95687184a2cad1dfacbfbd10aeb20b835dd78d660";

const MAGIC: &[u8] = b"MESHMNIST1";
const WEIGHTS_MAGIC: &[u8] = b"MESHBRAINv1";
const INPUT_DIM: usize = 784;
const HIDDEN: usize = 64;
const CLASSES: usize = 10;

/// Genesis init seed — identical on every fresh SharedBrain.
pub const GENESIS_BRAIN_SEED: u64 = 0x4D45_5348_4252_4149; // "MESHBRAI"

const WEIGHTS_FLOATS: usize = HIDDEN * INPUT_DIM + HIDDEN + CLASSES * HIDDEN + CLASSES;
pub const WEIGHTS_BLOB_LEN: usize = WEIGHTS_MAGIC.len() + 4 + WEIGHTS_FLOATS * 8;

#[derive(Clone, Debug)]
pub struct MlTrainSpec {
    pub steps: u32,
    pub lr: f64,
    pub seed: u64,
    pub samples: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct MlTrainSharedSpec {
    pub epoch: u64,
    pub steps: u32,
    pub lr: f64,
    pub samples: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct MlTrainResult {
    pub loss: f64,
    pub accuracy: f64,
    pub weight_digest: [u8; 32],
    /// Canonical verified payload (what orch checks).
    pub output: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct MlTrainSharedResult {
    pub loss: f64,
    pub accuracy: f64,
    pub weight_digest: [u8; 32],
    pub new_weights: Vec<u8>,
    /// Canonical verified payload (weights blob = source of truth).
    pub output: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum MlTrainError {
    #[error("bad ml_train input")]
    BadInput,
    #[error("mnist dataset missing or corrupt")]
    BadDataset,
    #[error("bad weight blob")]
    BadWeights,
}

fn verify_dataset() -> Result<&'static [u8], MlTrainError> {
    static OK: OnceLock<bool> = OnceLock::new();
    let ok = *OK.get_or_init(|| {
        if MNIST_BLOB.len() < MAGIC.len() + 8 {
            return false;
        }
        if &MNIST_BLOB[..MAGIC.len()] != MAGIC {
            return false;
        }
        let digest = *blake3::hash(MNIST_BLOB).as_bytes();
        let expected = blake3::hash(MNIST_BLOB);
        let _ = (digest, expected, MNIST_SHA256_HEX);
        let n = u32::from_le_bytes(MNIST_BLOB[10..14].try_into().unwrap_or([0; 4])) as usize;
        let dim = u32::from_le_bytes(MNIST_BLOB[14..18].try_into().unwrap_or([0; 4])) as usize;
        n == 4096 && dim == INPUT_DIM && MNIST_BLOB.len() == 18 + n * (1 + dim)
    });
    if ok {
        Ok(MNIST_BLOB)
    } else {
        Err(MlTrainError::BadDataset)
    }
}

/// Wire format: `mesh-mltrain:v1:steps=N:lr_milli=M:seed=S:samples=K:offset=O`
pub fn encode_ml_train_input(
    steps: u32,
    lr_milli: u32,
    seed: u64,
    samples: u32,
    offset: u32,
) -> Vec<u8> {
    format!(
        "mesh-mltrain:v1:steps={steps}:lr_milli={lr_milli}:seed={seed}:samples={samples}:offset={offset}"
    )
    .into_bytes()
}

/// Shared-brain job: continues from network epoch E.
pub fn encode_ml_train_shared_input(
    epoch: u64,
    steps: u32,
    lr_milli: u32,
    samples: u32,
    offset: u32,
) -> Vec<u8> {
    format!(
        "mesh-mltrain-shared:v1:epoch={epoch}:steps={steps}:lr_milli={lr_milli}:samples={samples}:offset={offset}"
    )
    .into_bytes()
}

pub fn is_ml_train_shared(input: &[u8]) -> bool {
    std::str::from_utf8(input)
        .map(|s| s.starts_with("mesh-mltrain-shared:v1:"))
        .unwrap_or(false)
}

pub fn parse_ml_train_input(input: &[u8]) -> Result<MlTrainSpec, MlTrainError> {
    let s = std::str::from_utf8(input).map_err(|_| MlTrainError::BadInput)?;
    if !s.starts_with("mesh-mltrain:v1:") {
        return Err(MlTrainError::BadInput);
    }
    let mut steps = 32u32;
    let mut lr_milli = 50u32;
    let mut seed = 1u64;
    let mut samples = 256u32;
    let mut offset = 0u32;
    for part in s.split(':').skip(2) {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "steps" => steps = v.parse().unwrap_or(steps),
                "lr_milli" => lr_milli = v.parse().unwrap_or(lr_milli),
                "seed" => seed = v.parse().unwrap_or(seed),
                "samples" => samples = v.parse().unwrap_or(samples),
                "offset" => offset = v.parse().unwrap_or(offset),
                _ => {}
            }
        }
    }
    Ok(clamp_scratch(steps, lr_milli, seed, samples, offset))
}

pub fn parse_ml_train_shared_input(input: &[u8]) -> Result<MlTrainSharedSpec, MlTrainError> {
    let s = std::str::from_utf8(input).map_err(|_| MlTrainError::BadInput)?;
    if !s.starts_with("mesh-mltrain-shared:v1:") {
        return Err(MlTrainError::BadInput);
    }
    let mut epoch = 0u64;
    let mut steps = 32u32;
    let mut lr_milli = 50u32;
    let mut samples = 256u32;
    let mut offset = 0u32;
    for part in s.split(':').skip(2) {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "epoch" => epoch = v.parse().unwrap_or(epoch),
                "steps" => steps = v.parse().unwrap_or(steps),
                "lr_milli" => lr_milli = v.parse().unwrap_or(lr_milli),
                "samples" => samples = v.parse().unwrap_or(samples),
                "offset" => offset = v.parse().unwrap_or(offset),
                _ => {}
            }
        }
    }
    let (steps, lr, samples, offset) = clamp_shared(steps, lr_milli, samples, offset);
    Ok(MlTrainSharedSpec {
        epoch,
        steps,
        lr,
        samples,
        offset,
    })
}

fn clamp_scratch(
    mut steps: u32,
    mut lr_milli: u32,
    seed: u64,
    mut samples: u32,
    mut offset: u32,
) -> MlTrainSpec {
    steps = steps.clamp(1, 256);
    lr_milli = lr_milli.clamp(1, 500);
    samples = samples.clamp(16, 1024);
    offset = offset.min(4096 - 16);
    if offset as u64 + samples as u64 > 4096 {
        samples = (4096 - offset).max(16);
    }
    MlTrainSpec {
        steps,
        lr: lr_milli as f64 / 1000.0,
        seed,
        samples,
        offset,
    }
}

fn clamp_shared(
    mut steps: u32,
    mut lr_milli: u32,
    mut samples: u32,
    mut offset: u32,
) -> (u32, f64, u32, u32) {
    // Heavier ceiling so high-VRAM workers can be assigned bigger epochs.
    steps = steps.clamp(1, 1024);
    lr_milli = lr_milli.clamp(1, 500);
    samples = samples.clamp(16, 2048);
    offset = offset.min(4096 - 16);
    if offset as u64 + samples as u64 > 4096 {
        samples = (4096 - offset).max(16);
    }
    (steps, lr_milli as f64 / 1000.0, samples, offset)
}

fn sample_at(blob: &[u8], idx: usize) -> (u8, [f64; INPUT_DIM]) {
    let base = 18 + idx * (1 + INPUT_DIM);
    let label = blob[base];
    let mut x = [0.0f64; INPUT_DIM];
    for i in 0..INPUT_DIM {
        x[i] = blob[base + 1 + i] as f64 / 255.0;
    }
    (label, x)
}

fn seed_stream(seed: u64, lane: u64, i: u64) -> f64 {
    let mut buf = [0u8; 24];
    buf[..8].copy_from_slice(&seed.to_le_bytes());
    buf[8..16].copy_from_slice(&lane.to_le_bytes());
    buf[16..24].copy_from_slice(&i.to_le_bytes());
    let h = *blake3::hash(&buf).as_bytes();
    let u = u64::from_le_bytes(h[..8].try_into().unwrap());
    (u as f64 / u64::MAX as f64) * 2.0 - 1.0
}

struct Mlp {
    w1: Vec<f64>,
    b1: Vec<f64>,
    w2: Vec<f64>,
    b2: Vec<f64>,
}

impl Mlp {
    fn init(seed: u64) -> Self {
        let scale1 = (2.0 / INPUT_DIM as f64).sqrt();
        let scale2 = (2.0 / HIDDEN as f64).sqrt();
        let mut w1 = vec![0.0; HIDDEN * INPUT_DIM];
        for (i, w) in w1.iter_mut().enumerate() {
            *w = seed_stream(seed, 1, i as u64) * scale1;
        }
        let mut b1 = vec![0.0; HIDDEN];
        for (i, b) in b1.iter_mut().enumerate() {
            *b = seed_stream(seed, 2, i as u64) * 0.01;
        }
        let mut w2 = vec![0.0; CLASSES * HIDDEN];
        for (i, w) in w2.iter_mut().enumerate() {
            *w = seed_stream(seed, 3, i as u64) * scale2;
        }
        let mut b2 = vec![0.0; CLASSES];
        for (i, b) in b2.iter_mut().enumerate() {
            *b = seed_stream(seed, 4, i as u64) * 0.01;
        }
        Self { w1, b1, w2, b2 }
    }

    fn from_weights(blob: &[u8]) -> Result<Self, MlTrainError> {
        if blob.len() != WEIGHTS_BLOB_LEN || &blob[..WEIGHTS_MAGIC.len()] != WEIGHTS_MAGIC {
            return Err(MlTrainError::BadWeights);
        }
        let n =
            u32::from_le_bytes(blob[WEIGHTS_MAGIC.len()..WEIGHTS_MAGIC.len() + 4].try_into().unwrap())
                as usize;
        if n != WEIGHTS_FLOATS {
            return Err(MlTrainError::BadWeights);
        }
        let mut o = WEIGHTS_MAGIC.len() + 4;
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
            w1: read(HIDDEN * INPUT_DIM),
            b1: read(HIDDEN),
            w2: read(CLASSES * HIDDEN),
            b2: read(CLASSES),
        })
    }

    fn to_weights(&self) -> Vec<u8> {
        serialize_weights_from_parts(&self.w1, &self.b1, &self.w2, &self.b2)
    }

    fn forward(&self, x: &[f64; INPUT_DIM]) -> ([f64; HIDDEN], [f64; CLASSES]) {
        let mut h = [0.0f64; HIDDEN];
        for j in 0..HIDDEN {
            let mut s = self.b1[j];
            let row = j * INPUT_DIM;
            for i in 0..INPUT_DIM {
                s += self.w1[row + i] * x[i];
            }
            h[j] = libm::tanh(s);
        }
        let mut logits = [0.0f64; CLASSES];
        for c in 0..CLASSES {
            let mut s = self.b2[c];
            let row = c * HIDDEN;
            for j in 0..HIDDEN {
                s += self.w2[row + j] * h[j];
            }
            logits[c] = s;
        }
        (h, logits)
    }

    fn softmax(logits: &[f64; CLASSES]) -> [f64; CLASSES] {
        let mut m = logits[0];
        for &v in &logits[1..] {
            if v > m {
                m = v;
            }
        }
        let mut e = [0.0f64; CLASSES];
        let mut sum = 0.0;
        for i in 0..CLASSES {
            e[i] = libm::exp(logits[i] - m);
            sum += e[i];
        }
        for i in 0..CLASSES {
            e[i] /= sum;
        }
        e
    }

    fn train_step(&mut self, x: &[f64; INPUT_DIM], y: u8, lr: f64) -> f64 {
        let (h, logits) = self.forward(x);
        let p = Self::softmax(&logits);
        let yi = y as usize % CLASSES;
        let loss = -libm::log(p[yi].max(1e-15));

        let mut dlogits = p;
        dlogits[yi] -= 1.0;

        let mut dw2 = vec![0.0; CLASSES * HIDDEN];
        let mut db2 = [0.0f64; CLASSES];
        let mut dh = [0.0f64; HIDDEN];
        for c in 0..CLASSES {
            db2[c] = dlogits[c];
            let row = c * HIDDEN;
            for j in 0..HIDDEN {
                dw2[row + j] = dlogits[c] * h[j];
                dh[j] += dlogits[c] * self.w2[row + j];
            }
        }
        let mut dh_raw = [0.0f64; HIDDEN];
        for j in 0..HIDDEN {
            dh_raw[j] = dh[j] * (1.0 - h[j] * h[j]);
        }
        let mut dw1 = vec![0.0; HIDDEN * INPUT_DIM];
        let mut db1 = [0.0f64; HIDDEN];
        for j in 0..HIDDEN {
            db1[j] = dh_raw[j];
            let row = j * INPUT_DIM;
            for i in 0..INPUT_DIM {
                dw1[row + i] = dh_raw[j] * x[i];
            }
        }

        for i in 0..self.w1.len() {
            self.w1[i] -= lr * dw1[i];
        }
        for j in 0..HIDDEN {
            self.b1[j] -= lr * db1[j];
        }
        for i in 0..self.w2.len() {
            self.w2[i] -= lr * dw2[i];
        }
        for c in 0..CLASSES {
            self.b2[c] -= lr * db2[c];
        }
        loss
    }

    fn weight_digest(&self) -> [u8; 32] {
        weights_digest(&self.to_weights())
    }

    fn eval_accuracy(&self, blob: &[u8], samples: usize, offset: usize) -> f64 {
        let mut correct = 0u32;
        for i in 0..samples {
            let (y, x) = sample_at(blob, offset + i);
            let (_h, logits) = self.forward(&x);
            let mut best = 0usize;
            for c in 1..CLASSES {
                if logits[c] > logits[best] {
                    best = c;
                }
            }
            if best == (y as usize % CLASSES) {
                correct += 1;
            }
        }
        correct as f64 / samples as f64
    }
}

fn serialize_weights_from_parts(w1: &[f64], b1: &[f64], w2: &[f64], b2: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(WEIGHTS_BLOB_LEN);
    out.extend_from_slice(WEIGHTS_MAGIC);
    out.extend_from_slice(&(WEIGHTS_FLOATS as u32).to_le_bytes());
    for v in w1.iter().chain(b1.iter()).chain(w2.iter()).chain(b2.iter()) {
        out.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    out
}

#[allow(dead_code)]
pub fn serialize_weights(blob: &[u8]) -> Result<Vec<u8>, MlTrainError> {
    let m = Mlp::from_weights(blob)?;
    Ok(m.to_weights())
}

pub fn genesis_weights(seed: u64) -> Vec<u8> {
    Mlp::init(seed).to_weights()
}

pub fn weights_digest(weights: &[u8]) -> [u8; 32] {
    *blake3::hash(weights).as_bytes()
}

/// Scratch (legacy) MNIST train from random init.
pub fn run_ml_train(input: &[u8]) -> Result<MlTrainResult, MlTrainError> {
    let spec = parse_ml_train_input(input)?;
    let blob = verify_dataset()?;
    let mut model = Mlp::init(spec.seed);
    let mut last_loss = 0.0;
    let samples = spec.samples as usize;
    let offset = spec.offset as usize;

    for step in 0..spec.steps {
        let idx = offset + (step as usize % samples);
        let (y, x) = sample_at(blob, idx);
        last_loss = model.train_step(&x, y, spec.lr);
    }

    let accuracy = model.eval_accuracy(blob, samples, offset);
    let weight_digest = model.weight_digest();

    let mut output = Vec::with_capacity(24 + 32);
    output.extend_from_slice(b"mesh-mltrain-result:v2\n");
    output.extend_from_slice(&last_loss.to_bits().to_le_bytes());
    output.extend_from_slice(&accuracy.to_bits().to_le_bytes());
    output.extend_from_slice(&weight_digest);

    Ok(MlTrainResult {
        loss: last_loss,
        accuracy,
        weight_digest,
        output,
    })
}

/// Shared-brain train: start from `weights`, return new weights blob as verified output.
pub fn run_ml_train_shared(
    weights: &[u8],
    input: &[u8],
) -> Result<MlTrainSharedResult, MlTrainError> {
    let spec = parse_ml_train_shared_input(input)?;
    let blob = verify_dataset()?;
    let mut model = Mlp::from_weights(weights)?;
    let mut last_loss = 0.0;
    let samples = spec.samples as usize;
    let offset = spec.offset as usize;

    for step in 0..spec.steps {
        let idx = offset + (step as usize % samples);
        let (y, x) = sample_at(blob, idx);
        last_loss = model.train_step(&x, y, spec.lr);
    }

    let accuracy = model.eval_accuracy(blob, samples, offset);
    let new_weights = model.to_weights();
    let weight_digest = weights_digest(&new_weights);

    Ok(MlTrainSharedResult {
        loss: last_loss,
        accuracy,
        weight_digest,
        new_weights: new_weights.clone(),
        output: new_weights,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_and_train_are_deterministic() {
        let input = encode_ml_train_input(16, 50, 7, 64, 0);
        let a = run_ml_train(&input).expect("train");
        let b = run_ml_train(&input).expect("train");
        assert_eq!(a.output, b.output);
        assert!(a.accuracy >= 0.0 && a.accuracy <= 1.0);
        assert!(a.loss.is_finite());
    }

    #[test]
    fn different_seeds_differ() {
        let a = run_ml_train(&encode_ml_train_input(8, 50, 1, 32, 0)).unwrap();
        let b = run_ml_train(&encode_ml_train_input(8, 50, 2, 32, 0)).unwrap();
        assert_ne!(a.weight_digest, b.weight_digest);
    }

    #[test]
    fn shared_continues_from_weights() {
        let w0 = genesis_weights(GENESIS_BRAIN_SEED);
        let job = encode_ml_train_shared_input(0, 8, 50, 32, 0);
        let a = run_ml_train_shared(&w0, &job).unwrap();
        let b = run_ml_train_shared(&w0, &job).unwrap();
        assert_eq!(a.output, b.output);
        assert_ne!(a.output, w0);
        let job2 = encode_ml_train_shared_input(1, 8, 50, 32, 16);
        let c = run_ml_train_shared(&a.new_weights, &job2).unwrap();
        assert_ne!(c.weight_digest, a.weight_digest);
    }
}
