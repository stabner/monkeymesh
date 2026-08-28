//! Host-side pad fill / fold for GPU miners (parallel, no per-nonce alloc).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use mesh_types::Hash;
use meshhash_cpu::{fill_scratchpad_for_nonce, fold_pow, MeshHashParams};

use crate::engine::cpu_parallelism;

static GPU_HOST_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Cores used to Blake3-fill / fold GPU pads. Leave the rest for the CPU miner.
pub fn gpu_host_threads() -> usize {
    let n = GPU_HOST_THREADS.load(Ordering::Relaxed);
    if n == 0 {
        cpu_parallelism().clamp(1, 64)
    } else {
        n.clamp(1, cpu_parallelism())
    }
}

pub fn set_gpu_host_threads(n: usize) {
    GPU_HOST_THREADS.store(n, Ordering::Relaxed);
}

/// Contiguous host pads: `count * pad_len` bytes, filled in parallel.
pub fn fill_pads_parallel(
    commitment: &Hash,
    params: &MeshHashParams,
    start_nonce: u64,
    count: u32,
    stop: &AtomicBool,
) -> Option<Vec<u8>> {
    let n = count as usize;
    let pad_len = params.scratchpad_size;
    if n == 0 || pad_len < 64 {
        return None;
    }
    let mut host = vec![0u8; pad_len.saturating_mul(n)];
    let threads = gpu_host_threads().min(n).max(1);
    let chunk = n.div_ceil(threads);
    thread::scope(|scope| {
        let mut rest = host.as_mut_slice();
        let mut base = 0usize;
        for _ in 0..threads {
            if base >= n {
                break;
            }
            let take = chunk.min(n - base);
            let (mine, tail) = rest.split_at_mut(take * pad_len);
            rest = tail;
            let start_i = base;
            base += take;
            scope.spawn(move || {
                for j in 0..take {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let off = j * pad_len;
                    fill_scratchpad_for_nonce(
                        commitment,
                        start_nonce + (start_i + j) as u64,
                        params,
                        &mut mine[off..off + pad_len],
                    );
                }
            });
        }
    });
    if stop.load(Ordering::Relaxed) {
        None
    } else {
        Some(host)
    }
}

/// Fold already-mixed host pads. Reverse must already be applied (GPU or CPU).
pub fn fold_pads_parallel(
    host: &mut [u8],
    params: &MeshHashParams,
    start_nonce: u64,
    difficulty: u32,
    stop: &AtomicBool,
) -> Option<u64> {
    let pad_len = params.scratchpad_size;
    if pad_len == 0 || host.len() < pad_len {
        return None;
    }
    let n = host.len() / pad_len;
    let found = std::sync::atomic::AtomicU64::new(u64::MAX);
    let threads = gpu_host_threads().min(n).max(1);
    let chunk = n.div_ceil(threads);
    thread::scope(|scope| {
        let mut rest = host;
        let mut base = 0usize;
        for _ in 0..threads {
            if base >= n {
                break;
            }
            let take = chunk.min(n - base);
            let (mine, tail) = rest.split_at_mut(take * pad_len);
            rest = tail;
            let start_i = base;
            base += take;
            let params = params.clone();
            let found = &found;
            scope.spawn(move || {
                for j in 0..take {
                    if stop.load(Ordering::Relaxed) || found.load(Ordering::Relaxed) != u64::MAX {
                        return;
                    }
                    let off = j * pad_len;
                    let pow = fold_pow(&mine[off..off + pad_len], &params);
                    if pow.meets_difficulty(difficulty) {
                        let nonce = start_nonce + (start_i + j) as u64;
                        let _ = found.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
                            if cur == u64::MAX || nonce < cur {
                                Some(nonce)
                            } else {
                                None
                            }
                        });
                        return;
                    }
                }
            });
        }
    });
    let n = found.load(Ordering::Relaxed);
    if n == u64::MAX {
        None
    } else {
        Some(n)
    }
}
