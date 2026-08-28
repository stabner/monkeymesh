//! Shared-brain v2 contract (Build/24).
//!
//! Bit-exact Q16.16 fixed-point MLP `784 → 512 → 128 → 10` (arch `mlp512`).
//! CPU reference is the verify path; CUDA must match this crate byte-for-byte.

mod brain;
mod job;
mod mlp;
mod q16;

pub use brain::{BrainAdvanceV2, BrainError, BrainMeta, SharedBrainV2};
pub use job::{
    encode_job, genesis_blob, is_ml_train_shared_v2, parse_job, run_job, MlTrainV2Error,
    MlTrainV2Result, MlTrainV2Spec, ARCH_MLP512, BRAIN_CONTRACT, DTYPE_Q16,
};
pub use mlp::{genesis_weights, weights_digest, GENESIS_BRAIN_SEED, WEIGHTS_BLOB_LEN, WEIGHTS_MAGIC};
pub use q16::{q_from_milli, ONE as Q16_ONE};

#[cfg(mesh_brain_cuda)]
pub mod cuda_api;

/// Prefer CUDA train when linked; otherwise CPU contract.
pub fn run_job_prefer_cuda(
    weights: &[u8],
    input: &[u8],
    workspace_bytes: u64,
) -> Result<job::MlTrainV2Result, job::MlTrainV2Error> {
    #[cfg(mesh_brain_cuda)]
    {
        if cuda_api::cuda_available() {
            return cuda_api::run_job_cuda(weights, input, workspace_bytes);
        }
    }
    let _ = workspace_bytes;
    job::run_job(weights, input)
}

/// True when this build has a CUDA brain backend and a device is present.
pub fn cuda_brain_available() -> bool {
    #[cfg(mesh_brain_cuda)]
    {
        return cuda_api::cuda_available();
    }
    #[cfg(not(mesh_brain_cuda))]
    {
        false
    }
}
