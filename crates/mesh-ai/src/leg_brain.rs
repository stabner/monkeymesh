//! Persistent Trilemma Guardian pack — four evolving leg brains (Build/25).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::legs::{
    encode_leg_job, genesis_leg_weights, parse_leg_job, run_leg_train, LegEpochs, LegError, LegId,
    LegSmart, WEIGHTS_BLOB_LEN,
};

const PERSIST_MAGIC: &[u8] = b"MESHLEGSTATEv1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegMeta {
    pub leg: String,
    pub epoch: u64,
    pub digest_hex: String,
    pub last_loss: f64,
    pub last_acc: f64,
    pub advances: u64,
    pub train_steps_total: u64,
}

#[derive(Clone, Debug)]
struct OneLeg {
    epoch: u64,
    weights: Vec<u8>,
    digest: [u8; 32],
    last_loss: f64,
    last_acc: f64,
    advances: u64,
    train_steps_total: u64,
}

impl OneLeg {
    fn genesis(leg: LegId) -> Self {
        let weights = genesis_leg_weights(leg);
        let digest = *blake3::hash(&weights).as_bytes();
        Self {
            epoch: 0,
            weights,
            digest,
            last_loss: 0.0,
            last_acc: 0.0,
            advances: 0,
            train_steps_total: 0,
        }
    }

    fn meta(&self, leg: LegId) -> LegMeta {
        LegMeta {
            leg: leg.as_str().into(),
            epoch: self.epoch,
            digest_hex: hex::encode(self.digest),
            last_loss: self.last_loss,
            last_acc: self.last_acc,
            advances: self.advances,
            train_steps_total: self.train_steps_total,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LegBrainPack {
    security: OneLeg,
    network: OneLeg,
    blocks: OneLeg,
    transpar: OneLeg,
    persist_path: Option<PathBuf>,
}

/// CPU verify result for one guardian leg (Build/27 N9).
#[derive(Clone, Debug)]
pub struct LegAdvance {
    pub job_epoch: u64,
    pub steps: u32,
    pub new_weights: Vec<u8>,
    pub digest: [u8; 32],
    pub loss: f64,
    pub accuracy: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum LegBrainError {
    #[error(transparent)]
    Train(#[from] LegError),
    #[error("stale leg epoch job={job} live={live}")]
    StaleEpoch { job: u64, live: u64 },
    #[error("verify failed")]
    VerifyFailed,
    #[error("io")]
    Io,
    #[error("bad persist")]
    BadPersist,
}

impl LegBrainPack {
    pub fn genesis(persist_path: Option<PathBuf>) -> Self {
        let p = Self {
            security: OneLeg::genesis(LegId::Security),
            network: OneLeg::genesis(LegId::Network),
            blocks: OneLeg::genesis(LegId::Blocks),
            transpar: OneLeg::genesis(LegId::Transpar),
            persist_path,
        };
        let _ = p.save();
        p
    }

    pub fn load_or_genesis(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(p) = Self::from_persist_bytes(&bytes, Some(path.to_path_buf())) {
                return p;
            }
        }
        Self::genesis(Some(path.to_path_buf()))
    }

    fn get(&self, leg: LegId) -> &OneLeg {
        match leg {
            LegId::Security => &self.security,
            LegId::Network => &self.network,
            LegId::Blocks => &self.blocks,
            LegId::Transpar => &self.transpar,
        }
    }

    fn get_mut(&mut self, leg: LegId) -> &mut OneLeg {
        match leg {
            LegId::Security => &mut self.security,
            LegId::Network => &mut self.network,
            LegId::Blocks => &mut self.blocks,
            LegId::Transpar => &mut self.transpar,
        }
    }

    pub fn epoch(&self, leg: LegId) -> u64 {
        self.get(leg).epoch
    }

    pub fn weights(&self, leg: LegId) -> &[u8] {
        &self.get(leg).weights
    }

    pub fn meta(&self, leg: LegId) -> LegMeta {
        self.get(leg).meta(leg)
    }

    pub fn all_meta(&self) -> Vec<LegMeta> {
        LegId::all().iter().map(|l| self.meta(*l)).collect()
    }

    pub fn epochs(&self) -> LegEpochs {
        LegEpochs {
            security: self.security.epoch,
            network: self.network.epoch,
            blocks: self.blocks.epoch,
            transpar: self.transpar.epoch,
        }
    }

    pub fn smart(&self) -> LegSmart {
        LegSmart {
            security: (self.security.last_acc * 100.0).round().clamp(0.0, 100.0) as u8,
            network: (self.network.last_acc * 100.0).round().clamp(0.0, 100.0) as u8,
            blocks: (self.blocks.last_acc * 100.0).round().clamp(0.0, 100.0) as u8,
            transpar: (self.transpar.last_acc * 100.0).round().clamp(0.0, 100.0) as u8,
        }
    }

    pub fn encode_job(
        &self,
        leg: LegId,
        steps: u32,
        lr_milli: u32,
        samples: u32,
        offset: u32,
    ) -> Vec<u8> {
        encode_leg_job(leg, self.epoch(leg), steps, lr_milli, samples, offset)
    }

    pub fn verify_and_advance(
        &mut self,
        job_input: &[u8],
        worker_output: &[u8],
    ) -> Result<LegId, LegBrainError> {
        let (leg, adv) = self.verify_only(job_input, worker_output)?;
        self.apply_advance(leg, adv)?;
        Ok(leg)
    }

    pub fn verify_only(
        &self,
        job_input: &[u8],
        worker_output: &[u8],
    ) -> Result<(LegId, LegAdvance), LegBrainError> {
        let spec = parse_leg_job(job_input)?;
        let live = self.epoch(spec.leg);
        if spec.epoch != live {
            return Err(LegBrainError::StaleEpoch {
                job: spec.epoch,
                live,
            });
        }
        let expected = run_leg_train(self.weights(spec.leg), job_input)?;
        if worker_output != expected.output.as_slice() {
            return Err(LegBrainError::VerifyFailed);
        }
        Ok((
            spec.leg,
            LegAdvance {
                job_epoch: spec.epoch,
                steps: spec.steps,
                new_weights: expected.new_weights,
                digest: expected.weight_digest,
                loss: expected.loss,
                accuracy: expected.accuracy,
            },
        ))
    }

    pub fn apply_advance(&mut self, leg_id: LegId, adv: LegAdvance) -> Result<(), LegBrainError> {
        let live = self.epoch(leg_id);
        if adv.job_epoch != live {
            return Err(LegBrainError::StaleEpoch {
                job: adv.job_epoch,
                live,
            });
        }
        let leg = self.get_mut(leg_id);
        leg.weights = adv.new_weights;
        leg.digest = adv.digest;
        leg.epoch = leg.epoch.saturating_add(1);
        leg.last_loss = adv.loss;
        leg.last_acc = adv.accuracy;
        leg.advances = leg.advances.saturating_add(1);
        leg.train_steps_total = leg.train_steps_total.saturating_add(adv.steps as u64);
        let _ = self.save();
        Ok(())
    }

    fn save(&self) -> Result<(), LegBrainError> {
        let Some(path) = &self.persist_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, self.to_persist_bytes()).map_err(|_| LegBrainError::Io)?;
        Ok(())
    }

    fn write_one(out: &mut Vec<u8>, leg: &OneLeg) {
        out.extend_from_slice(&leg.epoch.to_le_bytes());
        out.extend_from_slice(&leg.advances.to_le_bytes());
        out.extend_from_slice(&leg.train_steps_total.to_le_bytes());
        out.extend_from_slice(&leg.last_loss.to_bits().to_le_bytes());
        out.extend_from_slice(&leg.last_acc.to_bits().to_le_bytes());
        out.extend_from_slice(&leg.digest);
        out.extend_from_slice(&(leg.weights.len() as u32).to_le_bytes());
        out.extend_from_slice(&leg.weights);
    }

    fn read_one(bytes: &[u8], mut o: usize) -> Result<(OneLeg, usize), LegBrainError> {
        if bytes.len() < o + 8 * 3 + 8 * 2 + 32 + 4 {
            return Err(LegBrainError::BadPersist);
        }
        let epoch = u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        o += 8;
        let advances = u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        o += 8;
        let train_steps_total = u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        o += 8;
        let last_loss = f64::from_bits(u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap()));
        o += 8;
        let last_acc = f64::from_bits(u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap()));
        o += 8;
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&bytes[o..o + 32]);
        o += 32;
        let wlen = u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
        o += 4;
        if bytes.len() < o + wlen || wlen != WEIGHTS_BLOB_LEN {
            return Err(LegBrainError::BadPersist);
        }
        let weights = bytes[o..o + wlen].to_vec();
        o += wlen;
        if *blake3::hash(&weights).as_bytes() != digest {
            return Err(LegBrainError::BadPersist);
        }
        Ok((
            OneLeg {
                epoch,
                weights,
                digest,
                last_loss,
                last_acc,
                advances,
                train_steps_total,
            },
            o,
        ))
    }

    fn to_persist_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + WEIGHTS_BLOB_LEN * 4);
        out.extend_from_slice(PERSIST_MAGIC);
        Self::write_one(&mut out, &self.security);
        Self::write_one(&mut out, &self.network);
        Self::write_one(&mut out, &self.blocks);
        Self::write_one(&mut out, &self.transpar);
        out
    }

    fn from_persist_bytes(bytes: &[u8], path: Option<PathBuf>) -> Result<Self, LegBrainError> {
        if bytes.len() < PERSIST_MAGIC.len() || &bytes[..PERSIST_MAGIC.len()] != PERSIST_MAGIC {
            return Err(LegBrainError::BadPersist);
        }
        let mut o = PERSIST_MAGIC.len();
        let (security, o1) = Self::read_one(bytes, o)?;
        o = o1;
        let (network, o2) = Self::read_one(bytes, o)?;
        o = o2;
        let (blocks, o3) = Self::read_one(bytes, o)?;
        o = o3;
        let (transpar, _) = Self::read_one(bytes, o)?;
        Ok(Self {
            security,
            network,
            blocks,
            transpar,
            persist_path: path,
        })
    }
}
