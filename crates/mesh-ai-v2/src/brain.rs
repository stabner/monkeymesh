//! Shared brain v2 persistence (parallel to v1).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::job::{encode_job, parse_job, run_job};
use crate::mlp::{genesis_weights, weights_digest, GENESIS_BRAIN_SEED};

const PERSIST_MAGIC: &[u8] = b"MESHBRAINSTATEv2";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrainMeta {
    pub epoch: u64,
    pub digest_hex: String,
    pub updated_height: u64,
    pub train_steps_total: u64,
    pub last_loss_q16: i32,
    pub last_acc_q16: i32,
    pub advances: u64,
    pub contract: String,
    pub ver: u32,
}

#[derive(Clone, Debug)]
pub struct SharedBrainV2 {
    pub epoch: u64,
    pub weights: Vec<u8>,
    pub digest: [u8; 32],
    pub updated_height: u64,
    pub train_steps_total: u64,
    pub last_loss_q16: i32,
    pub last_acc_q16: i32,
    pub advances: u64,
    persist_path: Option<PathBuf>,
}

/// CPU verify result ready to apply under the AI lock (Build/27 N9).
#[derive(Clone, Debug)]
pub struct BrainAdvanceV2 {
    pub job_epoch: u64,
    pub steps: u32,
    pub new_weights: Vec<u8>,
    pub digest: [u8; 32],
    pub loss_q16: i32,
    pub accuracy_q16: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum BrainError {
    #[error("bad v2 job")]
    BadJob,
    #[error("stale brain epoch job={job} live={live}")]
    StaleEpoch { job: u64, live: u64 },
    #[error("train failed")]
    TrainFailed,
    #[error("verify failed")]
    VerifyFailed,
    #[error("io")]
    Io,
    #[error("bad persist")]
    BadPersist,
}

impl SharedBrainV2 {
    pub fn genesis(persist_path: Option<PathBuf>) -> Self {
        let weights = genesis_weights(GENESIS_BRAIN_SEED);
        let digest = weights_digest(&weights);
        let b = Self {
            epoch: 0,
            weights,
            digest,
            updated_height: 0,
            train_steps_total: 0,
            last_loss_q16: 0,
            last_acc_q16: 0,
            advances: 0,
            persist_path,
        };
        let _ = b.save();
        b
    }

    pub fn load_or_genesis(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(b) = Self::from_persist_bytes(&bytes, Some(path.to_path_buf())) {
                return b;
            }
        }
        Self::genesis(Some(path.to_path_buf()))
    }

    pub fn meta(&self) -> BrainMeta {
        BrainMeta {
            epoch: self.epoch,
            digest_hex: hex::encode(self.digest),
            updated_height: self.updated_height,
            train_steps_total: self.train_steps_total,
            last_loss_q16: self.last_loss_q16,
            last_acc_q16: self.last_acc_q16,
            advances: self.advances,
            contract: crate::job::BRAIN_CONTRACT.into(),
            ver: 2,
        }
    }

    pub fn encode_job(&self, steps: u32, lr_milli: u32, samples: u32, offset: u32) -> Vec<u8> {
        encode_job(self.epoch, steps, lr_milli, samples, offset)
    }

    pub fn verify_and_advance(
        &mut self,
        job_input: &[u8],
        worker_output: &[u8],
        height: u64,
    ) -> Result<(), BrainError> {
        let adv = self.verify_only(job_input, worker_output)?;
        self.apply_advance(adv, height)
    }

    pub fn verify_only(
        &self,
        job_input: &[u8],
        worker_output: &[u8],
    ) -> Result<BrainAdvanceV2, BrainError> {
        let spec = parse_job(job_input).map_err(|_| BrainError::BadJob)?;
        if spec.epoch != self.epoch {
            return Err(BrainError::StaleEpoch {
                job: spec.epoch,
                live: self.epoch,
            });
        }
        let expected = run_job(&self.weights, job_input).map_err(|_| BrainError::TrainFailed)?;
        if worker_output != expected.output.as_slice() {
            return Err(BrainError::VerifyFailed);
        }
        Ok(BrainAdvanceV2 {
            job_epoch: spec.epoch,
            steps: spec.steps,
            new_weights: expected.new_weights,
            digest: expected.weight_digest,
            loss_q16: expected.loss_q16,
            accuracy_q16: expected.accuracy_q16,
        })
    }

    pub fn apply_advance(&mut self, adv: BrainAdvanceV2, height: u64) -> Result<(), BrainError> {
        if adv.job_epoch != self.epoch {
            return Err(BrainError::StaleEpoch {
                job: adv.job_epoch,
                live: self.epoch,
            });
        }
        self.weights = adv.new_weights;
        self.digest = adv.digest;
        self.epoch = self.epoch.saturating_add(1);
        self.updated_height = height;
        self.train_steps_total = self.train_steps_total.saturating_add(adv.steps as u64);
        self.last_loss_q16 = adv.loss_q16;
        self.last_acc_q16 = adv.accuracy_q16;
        self.advances = self.advances.saturating_add(1);
        let _ = self.save();
        Ok(())
    }

    fn save(&self) -> Result<(), BrainError> {
        let Some(path) = &self.persist_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, self.to_persist_bytes()).map_err(|_| BrainError::Io)?;
        Ok(())
    }

    fn to_persist_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.weights.len());
        out.extend_from_slice(PERSIST_MAGIC);
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&self.updated_height.to_le_bytes());
        out.extend_from_slice(&self.train_steps_total.to_le_bytes());
        out.extend_from_slice(&self.advances.to_le_bytes());
        out.extend_from_slice(&self.last_loss_q16.to_le_bytes());
        out.extend_from_slice(&self.last_acc_q16.to_le_bytes());
        out.extend_from_slice(&self.digest);
        out.extend_from_slice(&(self.weights.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.weights);
        out
    }

    fn from_persist_bytes(bytes: &[u8], path: Option<PathBuf>) -> Result<Self, BrainError> {
        if bytes.len() < PERSIST_MAGIC.len() + 8 * 4 + 4 + 4 + 32 + 4 {
            return Err(BrainError::BadPersist);
        }
        if &bytes[..PERSIST_MAGIC.len()] != PERSIST_MAGIC {
            return Err(BrainError::BadPersist);
        }
        let mut o = PERSIST_MAGIC.len();
        let epoch = u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        o += 8;
        let updated_height = u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        o += 8;
        let train_steps_total = u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        o += 8;
        let advances = u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        o += 8;
        let last_loss_q16 = i32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        o += 4;
        let last_acc_q16 = i32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        o += 4;
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&bytes[o..o + 32]);
        o += 32;
        let wlen = u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
        o += 4;
        if bytes.len() < o + wlen {
            return Err(BrainError::BadPersist);
        }
        let weights = bytes[o..o + wlen].to_vec();
        if weights_digest(&weights) != digest {
            return Err(BrainError::BadPersist);
        }
        Ok(Self {
            epoch,
            weights,
            digest,
            updated_height,
            train_steps_total,
            last_loss_q16,
            last_acc_q16,
            advances,
            persist_path: path,
        })
    }
}
