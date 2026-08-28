//! Persistent Quantum Research Guardian pack — three evolving leg brains (Build/26).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::quantum::{
    encode_quantum_job, genesis_quantum_weights, parse_quantum_job, run_quantum_train,
    QuantumEpochs, QuantumError, QuantumId, QuantumSmart, WEIGHTS_BLOB_LEN,
};

const PERSIST_MAGIC: &[u8] = b"MESHQNTSTATEv1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuantumMeta {
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
    fn genesis(leg: QuantumId) -> Self {
        let weights = genesis_quantum_weights(leg);
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

    fn meta(&self, leg: QuantumId) -> QuantumMeta {
        QuantumMeta {
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
pub struct QuantumBrainPack {
    pqc: OneLeg,
    grover: OneLeg,
    harvest: OneLeg,
    persist_path: Option<PathBuf>,
}

/// CPU verify result for one quantum guardian leg.
#[derive(Clone, Debug)]
pub struct QuantumAdvance {
    pub job_epoch: u64,
    pub steps: u32,
    pub new_weights: Vec<u8>,
    pub digest: [u8; 32],
    pub loss: f64,
    pub accuracy: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum QuantumBrainError {
    #[error(transparent)]
    Train(#[from] QuantumError),
    #[error("stale quantum epoch job={job} live={live}")]
    StaleEpoch { job: u64, live: u64 },
    #[error("verify failed")]
    VerifyFailed,
    #[error("io")]
    Io,
    #[error("bad persist")]
    BadPersist,
}

impl QuantumBrainPack {
    pub fn genesis(persist_path: Option<PathBuf>) -> Self {
        let p = Self {
            pqc: OneLeg::genesis(QuantumId::Pqc),
            grover: OneLeg::genesis(QuantumId::Grover),
            harvest: OneLeg::genesis(QuantumId::Harvest),
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

    fn get(&self, leg: QuantumId) -> &OneLeg {
        match leg {
            QuantumId::Pqc => &self.pqc,
            QuantumId::Grover => &self.grover,
            QuantumId::Harvest => &self.harvest,
        }
    }

    fn get_mut(&mut self, leg: QuantumId) -> &mut OneLeg {
        match leg {
            QuantumId::Pqc => &mut self.pqc,
            QuantumId::Grover => &mut self.grover,
            QuantumId::Harvest => &mut self.harvest,
        }
    }

    pub fn epoch(&self, leg: QuantumId) -> u64 {
        self.get(leg).epoch
    }

    pub fn weights(&self, leg: QuantumId) -> &[u8] {
        &self.get(leg).weights
    }

    pub fn meta(&self, leg: QuantumId) -> QuantumMeta {
        self.get(leg).meta(leg)
    }

    pub fn all_meta(&self) -> Vec<QuantumMeta> {
        QuantumId::all().iter().map(|l| self.meta(*l)).collect()
    }

    pub fn epochs(&self) -> QuantumEpochs {
        QuantumEpochs {
            pqc: self.pqc.epoch,
            grover: self.grover.epoch,
            harvest: self.harvest.epoch,
        }
    }

    pub fn smart(&self) -> QuantumSmart {
        QuantumSmart {
            pqc: (self.pqc.last_acc * 100.0).round().clamp(0.0, 100.0) as u8,
            grover: (self.grover.last_acc * 100.0).round().clamp(0.0, 100.0) as u8,
            harvest: (self.harvest.last_acc * 100.0).round().clamp(0.0, 100.0) as u8,
        }
    }

    pub fn encode_job(
        &self,
        leg: QuantumId,
        steps: u32,
        lr_milli: u32,
        samples: u32,
        offset: u32,
    ) -> Vec<u8> {
        encode_quantum_job(leg, self.epoch(leg), steps, lr_milli, samples, offset)
    }

    pub fn verify_and_advance(
        &mut self,
        job_input: &[u8],
        worker_output: &[u8],
    ) -> Result<QuantumId, QuantumBrainError> {
        let (leg, adv) = self.verify_only(job_input, worker_output)?;
        self.apply_advance(leg, adv)?;
        Ok(leg)
    }

    pub fn verify_only(
        &self,
        job_input: &[u8],
        worker_output: &[u8],
    ) -> Result<(QuantumId, QuantumAdvance), QuantumBrainError> {
        let spec = parse_quantum_job(job_input)?;
        let live = self.epoch(spec.leg);
        if spec.epoch != live {
            return Err(QuantumBrainError::StaleEpoch {
                job: spec.epoch,
                live,
            });
        }
        let expected = run_quantum_train(self.weights(spec.leg), job_input)?;
        if worker_output != expected.output.as_slice() {
            return Err(QuantumBrainError::VerifyFailed);
        }
        Ok((
            spec.leg,
            QuantumAdvance {
                job_epoch: spec.epoch,
                steps: spec.steps,
                new_weights: expected.new_weights,
                digest: expected.weight_digest,
                loss: expected.loss,
                accuracy: expected.accuracy,
            },
        ))
    }

    pub fn apply_advance(
        &mut self,
        leg_id: QuantumId,
        adv: QuantumAdvance,
    ) -> Result<(), QuantumBrainError> {
        let live = self.epoch(leg_id);
        if adv.job_epoch != live {
            return Err(QuantumBrainError::StaleEpoch {
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

    fn save(&self) -> Result<(), QuantumBrainError> {
        let Some(path) = &self.persist_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, self.to_persist_bytes()).map_err(|_| QuantumBrainError::Io)?;
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

    fn read_one(bytes: &[u8], mut o: usize) -> Result<(OneLeg, usize), QuantumBrainError> {
        if bytes.len() < o + 8 * 3 + 8 * 2 + 32 + 4 {
            return Err(QuantumBrainError::BadPersist);
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
            return Err(QuantumBrainError::BadPersist);
        }
        let weights = bytes[o..o + wlen].to_vec();
        o += wlen;
        if *blake3::hash(&weights).as_bytes() != digest {
            return Err(QuantumBrainError::BadPersist);
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
        let mut out = Vec::with_capacity(64 + WEIGHTS_BLOB_LEN * 3);
        out.extend_from_slice(PERSIST_MAGIC);
        Self::write_one(&mut out, &self.pqc);
        Self::write_one(&mut out, &self.grover);
        Self::write_one(&mut out, &self.harvest);
        out
    }

    fn from_persist_bytes(bytes: &[u8], path: Option<PathBuf>) -> Result<Self, QuantumBrainError> {
        if bytes.len() < PERSIST_MAGIC.len() || &bytes[..PERSIST_MAGIC.len()] != PERSIST_MAGIC {
            return Err(QuantumBrainError::BadPersist);
        }
        let mut o = PERSIST_MAGIC.len();
        let (pqc, o1) = Self::read_one(bytes, o)?;
        o = o1;
        let (grover, o2) = Self::read_one(bytes, o)?;
        o = o2;
        let (harvest, _) = Self::read_one(bytes, o)?;
        Ok(Self {
            pqc,
            grover,
            harvest,
            persist_path: path,
        })
    }
}
