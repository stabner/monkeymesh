//! Job wire + CPU run_job for shared-brain v2.

use std::sync::OnceLock;

use crate::mlp::{
    genesis_weights, lr_from_milli, sample_to_q, weights_digest, Mlp, GENESIS_BRAIN_SEED, INPUT,
};

pub const ARCH_MLP512: &str = "mlp512";
pub const DTYPE_Q16: &str = "q16";
pub const BRAIN_CONTRACT: &str = "v2.0.0";

static MNIST_BLOB: &[u8] = include_bytes!("../data/mnist4096.bin");
const MAGIC: &[u8] = b"MESHMNIST1";

#[derive(Clone, Debug)]
pub struct MlTrainV2Spec {
    pub epoch: u64,
    pub steps: u32,
    pub lr_milli: u32,
    pub samples: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct MlTrainV2Result {
    pub loss_q16: i32,
    pub accuracy_q16: i32,
    pub weight_digest: [u8; 32],
    pub new_weights: Vec<u8>,
    pub output: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum MlTrainV2Error {
    #[error("bad ml_train_shared v2 input")]
    BadInput,
    #[error("mnist dataset missing or corrupt")]
    BadDataset,
    #[error("bad weight blob")]
    BadWeights,
}

fn verify_dataset() -> Result<&'static [u8], MlTrainV2Error> {
    static OK: OnceLock<bool> = OnceLock::new();
    let ok = *OK.get_or_init(|| {
        if MNIST_BLOB.len() < MAGIC.len() + 8 {
            return false;
        }
        if &MNIST_BLOB[..MAGIC.len()] != MAGIC {
            return false;
        }
        let n = u32::from_le_bytes(MNIST_BLOB[10..14].try_into().unwrap_or([0; 4])) as usize;
        let dim = u32::from_le_bytes(MNIST_BLOB[14..18].try_into().unwrap_or([0; 4])) as usize;
        n == 4096 && dim == INPUT && MNIST_BLOB.len() == 18 + n * (1 + dim)
    });
    if ok {
        Ok(MNIST_BLOB)
    } else {
        Err(MlTrainV2Error::BadDataset)
    }
}

pub fn encode_job(epoch: u64, steps: u32, lr_milli: u32, samples: u32, offset: u32) -> Vec<u8> {
    format!(
        "mesh-mltrain-shared:v2:epoch={epoch}:steps={steps}:lr_milli={lr_milli}:samples={samples}:offset={offset}:arch={ARCH_MLP512}:dtype={DTYPE_Q16}"
    )
    .into_bytes()
}

pub fn is_ml_train_shared_v2(input: &[u8]) -> bool {
    std::str::from_utf8(input)
        .map(|s| s.starts_with("mesh-mltrain-shared:v2:"))
        .unwrap_or(false)
}

pub fn parse_job(input: &[u8]) -> Result<MlTrainV2Spec, MlTrainV2Error> {
    let s = std::str::from_utf8(input).map_err(|_| MlTrainV2Error::BadInput)?;
    if !s.starts_with("mesh-mltrain-shared:v2:") {
        return Err(MlTrainV2Error::BadInput);
    }
    let mut epoch = 0u64;
    let mut steps = 64u32;
    let mut lr_milli = 50u32;
    let mut samples = 256u32;
    let mut offset = 0u32;
    let mut arch = String::new();
    let mut dtype = String::new();
    for part in s.split(':').skip(2) {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "epoch" => epoch = v.parse().unwrap_or(epoch),
                "steps" => steps = v.parse().unwrap_or(steps),
                "lr_milli" => lr_milli = v.parse().unwrap_or(lr_milli),
                "samples" => samples = v.parse().unwrap_or(samples),
                "offset" => offset = v.parse().unwrap_or(offset),
                "arch" => arch = v.to_string(),
                "dtype" => dtype = v.to_string(),
                _ => {}
            }
        }
    }
    if !arch.is_empty() && arch != ARCH_MLP512 {
        return Err(MlTrainV2Error::BadInput);
    }
    if !dtype.is_empty() && dtype != DTYPE_Q16 {
        return Err(MlTrainV2Error::BadInput);
    }
    steps = steps.clamp(1, 1024);
    lr_milli = lr_milli.clamp(1, 500);
    samples = samples.clamp(16, 2048);
    offset = offset.min(4096 - 16);
    if offset as u64 + samples as u64 > 4096 {
        samples = (4096 - offset).max(16);
    }
    Ok(MlTrainV2Spec {
        epoch,
        steps,
        lr_milli,
        samples,
        offset,
    })
}

fn sample_at(blob: &[u8], idx: usize) -> (u8, [i32; INPUT]) {
    let base = 18 + idx * (1 + INPUT);
    let label = blob[base];
    let x = sample_to_q(&blob[base + 1..base + 1 + INPUT]);
    (label, x)
}

/// CPU reference train (verify path).
pub fn run_job(weights: &[u8], input: &[u8]) -> Result<MlTrainV2Result, MlTrainV2Error> {
    let spec = parse_job(input)?;
    let blob = verify_dataset()?;
    let mut model = Mlp::from_weights(weights).map_err(|_| MlTrainV2Error::BadWeights)?;
    let lr = lr_from_milli(spec.lr_milli);
    let mut last_loss = 0i32;
    let samples = spec.samples as usize;
    let offset = spec.offset as usize;

    for step in 0..spec.steps {
        let idx = offset + (step as usize % samples);
        let (y, x) = sample_at(blob, idx);
        last_loss = model.train_step_v2(&x, y, lr);
    }

    // Eval on a short window for meta (deterministic).
    let eval_n = samples.min(64);
    let mut xs = Vec::with_capacity(eval_n);
    let mut ys = Vec::with_capacity(eval_n);
    for i in 0..eval_n {
        let (y, x) = sample_at(blob, offset + i);
        xs.push(x);
        ys.push(y);
    }
    let accuracy_q16 = model.eval_accuracy(&xs, &ys);
    let new_weights = model.to_weights();
    let weight_digest = weights_digest(&new_weights);
    Ok(MlTrainV2Result {
        loss_q16: last_loss,
        accuracy_q16,
        weight_digest,
        new_weights: new_weights.clone(),
        output: new_weights,
    })
}

pub fn genesis_blob() -> Vec<u8> {
    genesis_weights(GENESIS_BRAIN_SEED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_advances() {
        use crate::mlp::{WEIGHTS_BLOB_LEN, WEIGHTS_MAGIC};
        let w0 = genesis_blob();
        assert_eq!(w0.len(), WEIGHTS_BLOB_LEN);
        assert!(w0.starts_with(WEIGHTS_MAGIC));
        let job = encode_job(0, 8, 50, 32, 0);
        let a = run_job(&w0, &job).unwrap();
        let b = run_job(&w0, &job).unwrap();
        assert_eq!(a.output, b.output);
        assert_ne!(a.weight_digest, weights_digest(&w0));
        let job2 = encode_job(1, 8, 50, 32, 16);
        let c = run_job(&a.new_weights, &job2).unwrap();
        assert_ne!(c.weight_digest, a.weight_digest);
    }
}
