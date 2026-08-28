//! Optional CUDA FFI for shared-brain v2 (cfg `mesh_brain_cuda`).

use crate::job::{parse_job, run_job, MlTrainV2Error, MlTrainV2Result};
use crate::mlp::{weights_digest, N_WEIGHTS, WEIGHTS_BLOB_LEN, WEIGHTS_MAGIC};

#[link(name = "mesh_brain_v2_cuda", kind = "static")]
extern "C" {
    fn mesh_brain_v2_cuda_available() -> i32;
    fn mesh_brain_v2_run_job(
        weights_q16: *mut i32,
        n_weights: u32,
        mnist: *const u8,
        mnist_len: u32,
        steps: u32,
        lr_milli: u32,
        samples: u32,
        offset: u32,
        workspace_bytes: u64,
        loss_q16_out: *mut i32,
        acc_q16_out: *mut i32,
    ) -> i32;
}

/// True when this build linked a working CUDA brain backend.
pub fn cuda_available() -> bool {
    unsafe { mesh_brain_v2_cuda_available() != 0 }
}

/// Run a v2 job on CUDA when available; otherwise CPU contract.
pub fn run_job_auto(weights: &[u8], input: &[u8]) -> Result<MlTrainV2Result, MlTrainV2Error> {
    if cuda_available() {
        run_job_cuda(weights, input, 0)
    } else {
        run_job(weights, input)
    }
}

/// CUDA train path. `workspace_bytes` sizes activation buffers (0 = default ~64 MiB).
pub fn run_job_cuda(
    weights: &[u8],
    input: &[u8],
    workspace_bytes: u64,
) -> Result<MlTrainV2Result, MlTrainV2Error> {
    let spec = parse_job(input)?;
    if weights.len() < WEIGHTS_BLOB_LEN || &weights[..WEIGHTS_MAGIC.len()] != WEIGHTS_MAGIC {
        return Err(MlTrainV2Error::BadWeights);
    }
    let mut o = WEIGHTS_MAGIC.len();
    let n = u32::from_le_bytes(weights[o..o + 4].try_into().unwrap());
    o += 4;
    if n as usize != N_WEIGHTS || weights.len() < o + N_WEIGHTS * 4 {
        return Err(MlTrainV2Error::BadWeights);
    }
    let mut w = Vec::with_capacity(N_WEIGHTS);
    for i in 0..N_WEIGHTS {
        let bits = i32::from_le_bytes(weights[o + i * 4..o + i * 4 + 4].try_into().unwrap());
        w.push(bits);
    }

    let mnist: &[u8] = include_bytes!("../data/mnist4096.bin");
    let mut loss = 0i32;
    let mut acc = 0i32;
    let rc = unsafe {
        mesh_brain_v2_run_job(
            w.as_mut_ptr(),
            N_WEIGHTS as u32,
            mnist.as_ptr(),
            mnist.len() as u32,
            spec.steps,
            spec.lr_milli,
            spec.samples,
            spec.offset,
            workspace_bytes,
            &mut loss,
            &mut acc,
        )
    };
    if rc != 0 {
        // Fall back to CPU if CUDA path fails (device busy / OOM).
        return run_job(weights, input);
    }

    let mut new_weights = Vec::with_capacity(WEIGHTS_BLOB_LEN);
    new_weights.extend_from_slice(WEIGHTS_MAGIC);
    new_weights.extend_from_slice(&(N_WEIGHTS as u32).to_le_bytes());
    for x in &w {
        new_weights.extend_from_slice(&x.to_le_bytes());
    }
    let weight_digest = weights_digest(&new_weights);
    Ok(MlTrainV2Result {
        loss_q16: loss,
        accuracy_q16: acc,
        weight_digest,
        new_weights: new_weights.clone(),
        output: new_weights,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::{encode_job, genesis_blob, run_job};

    #[test]
    fn cuda_matches_cpu_when_available() {
        if !cuda_available() {
            return;
        }
        let w0 = genesis_blob();
        let job = encode_job(0, 8, 50, 32, 0);
        let cpu = run_job(&w0, &job).unwrap();
        let gpu = run_job_cuda(&w0, &job, 64 * 1024 * 1024).unwrap();
        assert_eq!(cpu.output, gpu.output);
        assert_eq!(cpu.weight_digest, gpu.weight_digest);
        assert_eq!(cpu.loss_q16, gpu.loss_q16);
        assert_eq!(cpu.accuracy_q16, gpu.accuracy_q16);
    }
}
