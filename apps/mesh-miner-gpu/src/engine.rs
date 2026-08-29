//! Mining engine: CUDA / OpenCL (AMD) / CPU MeshHash mix + node RPC.
//! Supports one or more devices concurrently (CPU + GPUs).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use mesh_types::{Address, Block, Hash};
use meshhash_cpu::{
    fill_scratchpad_for_nonce, fold_pow, fold_pow_from_device_extract, fold_sample_count,
    fold_sample_stride, fold_samples_buf_len, fusion_program_words, meshhash_cpu_with_params,
    mix_scratchpad_with_params, pow_search_inputs, FUSION_LANES, MeshHashParams, RateWindow,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::opencl_mix::{list_opencl_gpus, opencl_gpu_available, OpenClMixer};

#[cfg(mesh_cuda)]
#[repr(C)]
struct MeshCudaCtx {
    _private: [u8; 0],
}

#[cfg(mesh_cuda)]
extern "C" {
    fn mesh_cuda_ctx_create(device_id: i32) -> *mut MeshCudaCtx;
    fn mesh_cuda_ctx_destroy(ctx: *mut MeshCudaCtx);
    fn mesh_cuda_ctx_ensure(ctx: *mut MeshCudaCtx, bytes: usize) -> i32;
    fn mesh_cuda_ctx_upload_pad(
        ctx: *mut MeshCudaCtx,
        index: u32,
        pad_len: usize,
        host_pad: *const u8,
    ) -> i32;
    fn mesh_cuda_ctx_mix(ctx: *mut MeshCudaCtx, pad_len: usize, rounds: u32, batch: u32) -> i32;
    fn mesh_cuda_ctx_mix_range(
        ctx: *mut MeshCudaCtx,
        pad_len: usize,
        start_round: u32,
        rounds: u32,
        start_index: u32,
        count: u32,
    ) -> i32;
    fn mesh_cuda_ctx_mix_reverse_range(
        ctx: *mut MeshCudaCtx,
        pad_len: usize,
        start_round: u32,
        rounds: u32,
        total_rounds: u32,
        start_index: u32,
        count: u32,
    ) -> i32;
    fn mesh_cuda_ctx_upload_range(
        ctx: *mut MeshCudaCtx,
        start_index: u32,
        count: u32,
        pad_len: usize,
        host: *const u8,
    ) -> i32;
    fn mesh_cuda_ctx_download_range(
        ctx: *mut MeshCudaCtx,
        start_index: u32,
        count: u32,
        pad_len: usize,
        host: *mut u8,
    ) -> i32;
    fn mesh_cuda_ctx_synchronize(ctx: *mut MeshCudaCtx) -> i32;
    fn mesh_cuda_host_register(device_id: i32, p: *mut u8, n: usize) -> i32;
    fn mesh_cuda_host_unregister(device_id: i32, p: *mut u8) -> i32;
    fn mesh_cuda_ctx_download_heads(
        ctx: *mut MeshCudaCtx,
        pad_len: usize,
        count: u32,
        host: *mut u8,
    ) -> i32;
    fn mesh_cuda_ctx_fold_extract(
        ctx: *mut MeshCudaCtx,
        pad_len: usize,
        count: u32,
        sample_stride: usize,
        sample_count: u32,
        host_programs: *const u64,
        host_samples: *mut u8,
        host_wave_acc: *mut u8,
        do_wave: i32,
    ) -> i32;
    fn mesh_cuda_ctx_download_pad(
        ctx: *mut MeshCudaCtx,
        index: u32,
        pad_len: usize,
        host_pad: *mut u8,
    ) -> i32;
    fn mesh_cuda_device_count(out: *mut i32) -> i32;
    fn mesh_cuda_set_device(id: i32) -> i32;
    fn mesh_cuda_device_name(device_id: i32, out: *mut u8, out_len: i32) -> i32;
    fn mesh_cuda_device_vram_bytes(device_id: i32, out: *mut u64) -> i32;
}

/// Soft ceiling on parallel pads (VRAM-resident).
const MAX_PARALLEL_PADS: u32 = 1024;
/// Host fill buffer cap (one wave). Pads stay in VRAM; fold samples are tiny.
const MAX_HOST_FILL_BYTES: usize = 2 * 1024 * 1024 * 1024;
/// OpenCL still round-trips full pads — keep PCIe copies modest.
const MAX_OPENCL_XFER_BYTES: usize = 512 * 1024 * 1024;
/// Mix-round slices so a 64–128k Fusion mix can honor Stop.
/// Each chunk must continue the previous `state` (device buffer) — not pad[0:8].
/// Chunks queue on the default stream; one device sync at the end of the pass.
const CUDA_MIX_CHUNK: u32 = 65_536;
/// CPU: keep host scratch modest per auto-batch sizing (threads own their pads).
const DEFAULT_SCRATCH_BUDGET: usize = 128 * 1024 * 1024;
const MIN_SCRATCH_BUDGET: usize = 64 * 1024 * 1024;
/// Cap device batch memory so mining rigs stay sane (pads live in VRAM).
const MAX_VRAM_BATCH_BYTES: usize = 4 * 1024 * 1024 * 1024;
/// Use a fraction of VRAM for the on-device pad batch.
const VRAM_USE_PCT: u64 = 40;
/// Never mix more pads than this for luck vs tip lifetime (VRAM auto can be hundreds).
const MAX_LUCK_BATCH: u32 = 32;

#[inline]
fn work_aborted(stop: &AtomicBool, stale: &AtomicBool) -> bool {
    stop.load(Ordering::Relaxed) || stale.load(Ordering::Relaxed)
}

/// How many parallel pads are worth mixing before the tip likely moves.
/// At low testnet difficulty a VRAM-full wave outlives the block and looks like a stall.
fn luck_batch_cap(difficulty: u32) -> u32 {
    let expected = 1u32 << difficulty.min(16);
    expected.saturating_mul(4).clamp(8, MAX_LUCK_BATCH)
}

/// Logical CPUs for MeshHash (like XMRig thread count).
pub fn cpu_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 64)
}

/// 0 = use all cores. Set when CPU+GPU PoW run together so pad-fill keeps cores.
static CPU_MINE_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Cores the CPU MeshHash worker may use (leaves the rest for GPU host pad fill).
pub fn cpu_mine_threads() -> usize {
    let n = CPU_MINE_THREADS.load(Ordering::Relaxed);
    if n == 0 {
        cpu_parallelism()
    } else {
        n.clamp(1, cpu_parallelism())
    }
}

fn set_cpu_mine_threads(has_cpu: bool, has_gpu: bool) {
    let total = cpu_parallelism();
    if has_cpu && has_gpu {
        // GPU H/s is fill-bound once mix occupancy is real — give fill the majority.
        let fill = ((total * 3) / 4).clamp(4, 24).min(total.saturating_sub(2));
        let cpu = (total - fill).max(2);
        CPU_MINE_THREADS.store(cpu, Ordering::Relaxed);
        crate::host_pads::set_gpu_host_threads(fill);
    } else if has_gpu {
        CPU_MINE_THREADS.store(0, Ordering::Relaxed);
        crate::host_pads::set_gpu_host_threads(total);
    } else {
        CPU_MINE_THREADS.store(0, Ordering::Relaxed);
        crate::host_pads::set_gpu_host_threads(0);
    }
}

/// Scratch budget from GPU VRAM (device-side). Host only stages one pad.
pub fn scratch_budget_bytes(vram_bytes: Option<u64>) -> usize {
    match vram_bytes.filter(|&v| v > 0) {
        Some(v) => {
            let usable = v.saturating_mul(VRAM_USE_PCT) / 100;
            (usable as usize).clamp(MIN_SCRATCH_BUDGET, MAX_VRAM_BATCH_BYTES)
        }
        None => DEFAULT_SCRATCH_BUDGET,
    }
}

/// Clamp / auto-scale batch from pad size + card VRAM (pads resident on GPU).
/// `user_batch == 0` means auto.
/// CPU (`vram_bytes == None`): size batch to feed all cores continuously.
/// `host_roundtrip`: OpenCL still downloads full pads — cap PCIe copies.
pub fn clamp_batch(user_batch: u32, pad_size: usize, vram_bytes: Option<u64>) -> u32 {
    clamp_batch_xfer(user_batch, pad_size, vram_bytes, false, None)
}

fn clamp_batch_xfer(
    user_batch: u32,
    pad_size: usize,
    vram_bytes: Option<u64>,
    host_roundtrip: bool,
    difficulty: Option<u32>,
) -> u32 {
    let pad = pad_size.max(64);
    if vram_bytes.is_none() {
        let threads = cpu_mine_threads().max(1) as u32;
        // Several nonces per core per wave so the scheduler stays saturated.
        let auto = threads.saturating_mul(16).max(threads).min(MAX_PARALLEL_PADS);
        return if user_batch == 0 {
            auto
        } else {
            user_batch.max(1).min(MAX_PARALLEL_PADS)
        };
    }
    let budget = scratch_budget_bytes(vram_bytes);
    let max_by_mem = ((budget / pad).max(1) as u32).min(MAX_PARALLEL_PADS);
    let max_by_host = ((MAX_HOST_FILL_BYTES / pad).max(1) as u32).min(MAX_PARALLEL_PADS);
    let mut cap = max_by_mem.min(max_by_host);
    if host_roundtrip {
        let xfer = ((MAX_OPENCL_XFER_BYTES / pad).max(1) as u32).min(MAX_PARALLEL_PADS);
        cap = cap.min(xfer);
    }
    if let Some(diff) = difficulty {
        cap = cap.min(luck_batch_cap(diff));
    }
    if user_batch == 0 {
        cap.max(1)
    } else {
        user_batch.max(1).min(cap)
    }
}

#[cfg(mesh_cuda)]
struct CudaMixer {
    ctx: *mut MeshCudaCtx,
    device: i32,
    name: String,
    vram_bytes: u64,
    fold_checked: bool,
}

#[cfg(mesh_cuda)]
unsafe impl Send for CudaMixer {}

#[cfg(mesh_cuda)]
impl Drop for CudaMixer {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { mesh_cuda_ctx_destroy(self.ctx) };
            self.ctx = std::ptr::null_mut();
        }
    }
}

#[cfg(mesh_cuda)]
impl CudaMixer {
    fn try_new(device: i32) -> Result<Self> {
        let ctx = unsafe { mesh_cuda_ctx_create(device) };
        if ctx.is_null() {
            bail!("CUDA device {device} unavailable");
        }
        let mut buf = vec![0u8; 192];
        let name = if unsafe { mesh_cuda_device_name(device, buf.as_mut_ptr(), buf.len() as i32) } == 0
        {
            let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..nul]).to_string()
        } else {
            format!("CUDA device {device}")
        };
        let mut vram = 0u64;
        let _ = unsafe { mesh_cuda_device_vram_bytes(device, &mut vram) };
        Ok(Self {
            ctx,
            device,
            name,
            vram_bytes: vram,
            fold_checked: false,
        })
    }

    fn vram_bytes(&self) -> u64 {
        self.vram_bytes
    }

    fn mix_range_interruptible(
        &mut self,
        pad_len: usize,
        rounds: u32,
        wave_start: u32,
        count: u32,
        stop: &AtomicBool,
        stale: &AtomicBool,
    ) -> Result<bool> {
        let mut done = 0u32;
        while done < rounds {
            if work_aborted(stop, stale) {
                let _ = unsafe { mesh_cuda_ctx_synchronize(self.ctx) };
                return Ok(false);
            }
            let chunk = CUDA_MIX_CHUNK.min(rounds - done);
            let rc = unsafe {
                mesh_cuda_ctx_mix_range(self.ctx, pad_len, done, chunk, wave_start, count)
            };
            if rc != 0 {
                bail!("CUDA mix range failed (rc={rc})");
            }
            done += chunk;
        }
        let rc = unsafe { mesh_cuda_ctx_synchronize(self.ctx) };
        if rc != 0 {
            bail!("CUDA sync failed (rc={rc})");
        }
        Ok(true)
    }

    fn mix_reverse_interruptible(
        &mut self,
        pad_len: usize,
        rounds: u32,
        wave_start: u32,
        count: u32,
        stop: &AtomicBool,
        stale: &AtomicBool,
    ) -> Result<bool> {
        let mut done = 0u32;
        while done < rounds {
            if work_aborted(stop, stale) {
                let _ = unsafe { mesh_cuda_ctx_synchronize(self.ctx) };
                return Ok(false);
            }
            let chunk = CUDA_MIX_CHUNK.min(rounds - done);
            let rc = unsafe {
                mesh_cuda_ctx_mix_reverse_range(
                    self.ctx, pad_len, done, chunk, rounds, wave_start, count,
                )
            };
            if rc != 0 {
                bail!("CUDA reverse mix failed (rc={rc})");
            }
            done += chunk;
        }
        let rc = unsafe { mesh_cuda_ctx_synchronize(self.ctx) };
        if rc != 0 {
            bail!("CUDA sync failed (rc={rc})");
        }
        Ok(true)
    }

    /// Parallel CPU fill → bulk H2D → GPU forward (+ reverse) → bulk D2H → parallel fold.
    fn search_batch(
        &mut self,
        commitment: &Hash,
        difficulty: u32,
        params: &MeshHashParams,
        start_nonce: u64,
        batch: u32,
        stop: &AtomicBool,
        stale: &AtomicBool,
    ) -> Result<Option<u64>> {
        let Some(host) =
            crate::host_pads::fill_pads_parallel(commitment, params, start_nonce, batch, stop, stale)
        else {
            return Ok(None);
        };
        self.mix_filled_pads(host, difficulty, params, start_nonce, batch, stop, stale)
    }

    /// Mix pads already Blake3-filled on the host. Fold samples stay on-device
    /// (no full-pad D2H). CPU rematch rebuilds any winning nonce from scratch.
    fn mix_filled_pads(
        &mut self,
        mut host: Vec<u8>,
        difficulty: u32,
        params: &MeshHashParams,
        start_nonce: u64,
        batch: u32,
        stop: &AtomicBool,
        stale: &AtomicBool,
    ) -> Result<Option<u64>> {
        let pad_len = params.scratchpad_size;
        let bytes = pad_len.saturating_mul(batch as usize);
        if host.len() < bytes {
            bail!("CUDA host pad buffer short ({} < {})", host.len(), bytes);
        }
        let _gpu = crate::gpu_gate::lock_gpu();
        let rc = unsafe { mesh_cuda_ctx_ensure(self.ctx, bytes) };
        if rc != 0 {
            bail!("CUDA ensure {} bytes failed (rc={rc})", bytes);
        }
        let pinned =
            unsafe { mesh_cuda_host_register(self.device, host.as_mut_ptr(), bytes) } == 0;
        let rc = unsafe { mesh_cuda_ctx_upload_range(self.ctx, 0, batch, pad_len, host.as_ptr()) };
        if pinned {
            let _ = unsafe { mesh_cuda_host_unregister(self.device, host.as_mut_ptr()) };
        }
        drop(host);
        if rc != 0 {
            bail!("CUDA upload range failed (rc={rc})");
        }
        let rounds = params.mix_rounds as u32;
        if !self.mix_range_interruptible(pad_len, rounds, 0, batch, stop, stale)? {
            return Ok(None);
        }
        if params.version >= 2
            && !self.mix_reverse_interruptible(pad_len, rounds, 0, batch, stop, stale)?
        {
            return Ok(None);
        }
        if work_aborted(stop, stale) {
            return Ok(None);
        }

        let n = batch as usize;
        let mut heads = vec![0u8; n.saturating_mul(32)];
        let rc = unsafe { mesh_cuda_ctx_download_heads(self.ctx, pad_len, batch, heads.as_mut_ptr()) };
        if rc != 0 {
            bail!("CUDA download heads failed (rc={rc})");
        }
        let do_wave = params.version >= 4;
        let mut programs = vec![0u64; n.saturating_mul(FUSION_LANES)];
        if do_wave {
            for i in 0..n {
                let head = &heads[i * 32..i * 32 + 32];
                let prog = fusion_program_words(params.fold_salt, head);
                programs[i * FUSION_LANES..(i + 1) * FUSION_LANES].copy_from_slice(&prog);
            }
        }
        let sample_stride = fold_sample_stride(pad_len);
        let sample_count = fold_sample_count(pad_len) as u32;
        let pitch = fold_samples_buf_len(pad_len);
        let mut samples = vec![0u8; pitch.saturating_mul(n)];
        let mut wave_acc = vec![0u8; n.saturating_mul(32)];
        let rc = unsafe {
            mesh_cuda_ctx_fold_extract(
                self.ctx,
                pad_len,
                batch,
                sample_stride,
                sample_count,
                programs.as_ptr(),
                samples.as_mut_ptr(),
                wave_acc.as_mut_ptr(),
                if do_wave { 1 } else { 0 },
            )
        };
        if rc != 0 {
            bail!("CUDA fold extract failed (rc={rc})");
        }
        if !self.fold_checked {
            let mut pad0 = vec![0u8; pad_len];
            let rc = unsafe {
                mesh_cuda_ctx_download_pad(self.ctx, 0, pad_len, pad0.as_mut_ptr())
            };
            if rc != 0 {
                bail!("CUDA fold audit download failed (rc={rc})");
            }
            let cpu = fold_pow(&pad0, params);
            let packed = &samples[..pitch];
            let mut acc = [0u8; 32];
            acc.copy_from_slice(&wave_acc[..32]);
            let extracted = fold_pow_from_device_extract(packed, &acc, pad_len, params);
            if cpu != extracted {
                bail!(
                    "CUDA fold extract mismatch vs CPU rematch (device digest wrong) — refusing to mine"
                );
            }
            self.fold_checked = true;
        }
        drop(_gpu);

        let mut found = u64::MAX;
        for i in 0..n {
            if work_aborted(stop, stale) {
                return Ok(None);
            }
            let packed = &samples[i * pitch..(i + 1) * pitch];
            let mut acc = [0u8; 32];
            acc.copy_from_slice(&wave_acc[i * 32..(i + 1) * 32]);
            let pow = fold_pow_from_device_extract(packed, &acc, pad_len, params);
            if pow.meets_difficulty(difficulty) {
                let nonce = start_nonce + i as u64;
                if found == u64::MAX || nonce < found {
                    found = nonce;
                }
            }
        }
        if found == u64::MAX {
            Ok(None)
        } else {
            Ok(Some(found))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MinerBackend {
    #[default]
    Auto,
    Nvidia,
    Amd,
    Cpu,
}

impl MinerBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Nvidia => "NVIDIA CUDA",
            Self::Amd => "AMD OpenCL",
            Self::Cpu => "CPU",
        }
    }

    pub const ALL: [MinerBackend; 4] = [Self::Auto, Self::Nvidia, Self::Amd, Self::Cpu];
}

/// A selectable compute device (CPU or a specific GPU).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ComputeDevice {
    Cpu,
    Cuda { index: i32 },
    OpenCl { index: i32 },
}

impl ComputeDevice {
    pub fn key(self) -> String {
        match self {
            Self::Cpu => "cpu".into(),
            Self::Cuda { index } => format!("cuda:{index}"),
            Self::OpenCl { index } => format!("opencl:{index}"),
        }
    }

    pub fn short_label(self) -> String {
        match self {
            Self::Cpu => "CPU".into(),
            Self::Cuda { index } => format!("CUDA {index}"),
            Self::OpenCl { index } => format!("OpenCL {index}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub id: ComputeDevice,
    pub name: String,
    /// "CPU" | "NVIDIA" | "AMD" | "OpenCL"
    pub family: &'static str,
    /// Reported device memory (0 for CPU / unknown).
    pub vram_bytes: u64,
}

/// Enumerate CPU + CUDA GPUs + OpenCL GPUs (skips NVIDIA OpenCL when CUDA is present).
pub fn enumerate_devices() -> Vec<DeviceInfo> {
    let mut out = vec![DeviceInfo {
        id: ComputeDevice::Cpu,
        name: "CPU (MeshHash mix)".into(),
        family: "CPU",
        vram_bytes: 0,
    }];

    let cuda_n = cuda_device_count();
    for i in 0..cuda_n {
        let vram = cuda_device_vram(i).unwrap_or(0);
        let base = cuda_device_name(i);
        let name = if vram > 0 {
            format!("{base} · {}", format_bytes(vram))
        } else {
            base
        };
        out.push(DeviceInfo {
            id: ComputeDevice::Cuda { index: i },
            name,
            family: "NVIDIA",
            vram_bytes: vram,
        });
    }

    for g in list_opencl_gpus() {
        if g.is_nvidia && cuda_n > 0 {
            continue;
        }
        let family = if g.is_amd {
            "AMD"
        } else if g.is_nvidia {
            "NVIDIA"
        } else {
            "OpenCL"
        };
        let vram = if g.vram_bytes > 0 {
            format!(" · {}", format_bytes(g.vram_bytes))
        } else {
            String::new()
        };
        let label = if g.is_amd {
            format!("AMD OpenCL — {}{vram}", g.name)
        } else {
            format!("{}{vram}", g.name)
        };
        out.push(DeviceInfo {
            id: ComputeDevice::OpenCl { index: g.index },
            name: label,
            family,
            vram_bytes: g.vram_bytes,
        });
    }

    out
}

/// Best GPU name + total VRAM across selected devices (for AI advertise).
pub fn ai_capacity_from_selection(
    catalog: &[DeviceInfo],
    selected: &std::collections::HashSet<ComputeDevice>,
) -> (String, u64) {
    let mut best_name = "miner-gui".to_string();
    let mut best_vram = 0u64;
    let mut total = 0u64;
    for d in catalog {
        if matches!(d.id, ComputeDevice::Cpu) || !selected.contains(&d.id) {
            continue;
        }
        total = total.saturating_add(d.vram_bytes);
        if d.vram_bytes >= best_vram {
            best_vram = d.vram_bytes;
            best_name = d.name.clone();
        }
    }
    if total == 0 {
        (best_name, 8 * 1024 * 1024 * 1024)
    } else {
        (best_name, total)
    }
}

fn format_bytes(n: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if n as f64 >= GIB {
        format!("{:.1} GiB", n as f64 / GIB)
    } else {
        format!("{:.0} MiB", n as f64 / MIB)
    }
}

fn cuda_device_vram(index: i32) -> Option<u64> {
    #[cfg(mesh_cuda)]
    {
        let mut v = 0u64;
        if unsafe { mesh_cuda_device_vram_bytes(index, &mut v) } == 0 && v > 0 {
            Some(v)
        } else {
            None
        }
    }
    #[cfg(not(mesh_cuda))]
    {
        let _ = index;
        None
    }
}

fn cuda_device_name(index: i32) -> String {
    #[cfg(mesh_cuda)]
    {
        let mut buf = vec![0u8; 192];
        let rc = unsafe { mesh_cuda_device_name(index, buf.as_mut_ptr(), buf.len() as i32) };
        if rc == 0 {
            let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let name = String::from_utf8_lossy(&buf[..nul]).to_string();
            if !name.is_empty() {
                return format!("NVIDIA CUDA — {name}");
            }
        }
        format!("NVIDIA CUDA device {index}")
    }
    #[cfg(not(mesh_cuda))]
    {
        let _ = index;
        "NVIDIA CUDA (not linked in this build)".into()
    }
}

pub fn cuda_device_count() -> i32 {
    #[cfg(mesh_cuda)]
    {
        let mut n = 0i32;
        let rc = unsafe { mesh_cuda_device_count(&mut n) };
        if rc == 0 && n > 0 {
            n
        } else {
            0
        }
    }
    #[cfg(not(mesh_cuda))]
    {
        0
    }
}

#[derive(Clone, Debug)]
pub struct MinerConfig {
    pub rpc: String,
    pub address: Address,
    pub batch: u32,
    pub max_nonces: u64,
    /// Legacy single-device index (used when `devices` is empty).
    pub device: i32,
    /// Legacy backend picker (used when `devices` is empty).
    pub backend: MinerBackend,
    /// Explicit multi-device selection. When non-empty, overrides backend/device.
    pub devices: Vec<ComputeDevice>,
    /// When set, PoW uses this HTTP pool instead of edge/seed RPC list.
    pub pool: Option<String>,
    /// Optional worker / rig name — pool credits as `address.worker`.
    pub worker_name: Option<String>,
}

impl MinerConfig {
    pub fn from_backend(
        rpc: String,
        address: Address,
        batch: u32,
        max_nonces: u64,
        device: i32,
        backend: MinerBackend,
    ) -> Self {
        Self {
            rpc,
            address,
            batch,
            max_nonces,
            device,
            backend,
            devices: Vec::new(),
            pool: None,
            worker_name: None,
        }
    }

    pub fn with_devices(
        rpc: String,
        address: Address,
        batch: u32,
        max_nonces: u64,
        devices: Vec<ComputeDevice>,
    ) -> Self {
        Self {
            rpc,
            address,
            batch,
            max_nonces,
            device: 0,
            backend: MinerBackend::Auto,
            devices,
            pool: None,
            worker_name: None,
        }
    }

    pub fn with_pool(mut self, pool: Option<String>) -> Self {
        self.pool = pool
            .map(|s| normalize_pool_url(&s))
            .filter(|s| !s.is_empty());
        self
    }

    pub fn with_worker_name(mut self, name: Option<String>) -> Self {
        self.worker_name = name
            .map(|s| sanitize_worker_name(&s))
            .filter(|s| !s.is_empty());
        self
    }
}

/// Safe worker / rig label for pool identity (`address.worker`).
pub fn sanitize_worker_name(raw: &str) -> String {
    let s: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(32)
        .collect();
    s.trim_matches('_').to_string()
}

/// MiningCore-style username: `walletaddress.workername` (worker defaults to `default`).
pub fn miner_identity(address: &Address, worker: Option<&str>) -> String {
    let w = worker
        .map(sanitize_worker_name)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".into());
    format!("{address}.{w}")
}

/// Pool GBT/submit use `?address=wallet.worker`; seed RPC must stay a bare mesh address.
fn gbt_address_param(rpc: &str, payout: &Address, miner_id: &str) -> String {
    if looks_like_pool_target(rpc) {
        miner_id.to_string()
    } else {
        payout.to_string()
    }
}

/// True when every URL looks like the Hashmonkeys MESH HTTP(S) pool (not seed/edge RPC).
pub fn looks_like_pool_target(raw: &str) -> bool {
    let urls = mesh_types::parse_rpc_list(raw);
    if urls.is_empty() {
        return false;
    }
    urls.iter().all(|u| {
        let n = normalize_pool_url(u).to_lowercase();
        if n.contains(":12500") || n.contains(":13500") {
            return true;
        }
        // Public HTTPS front on eu.hashmonkeys.cloud (443) — not seed/edge RPC ports.
        if n.contains("eu.hashmonkeys.cloud") {
            return !(n.contains(":18080")
                || n.contains(":18081")
                || n.contains(":18083")
                || n.contains(":39001")
                || n.contains(":39002")
                || n.contains(":39003"));
        }
        false
    })
}

/// MESH pool is HTTP GBT (Hashmonkeys port scheme e.g. :12500), not stratum+tcp.
/// Accepts pasted stratum URLs and rewrites them to `http://…`.
pub fn normalize_pool_url(raw: &str) -> String {
    let s = raw.trim().trim_end_matches('/');
    if s.is_empty() {
        return String::new();
    }
    let rest = s
        .strip_prefix("stratum+tcp://")
        .or_else(|| s.strip_prefix("stratum+ssl://"))
        .or_else(|| s.strip_prefix("stratum://"))
        .unwrap_or(s);
    let mut url = if rest.starts_with("http://") || rest.starts_with("https://") {
        rest.to_string()
    } else {
        format!("http://{rest}")
    };
    if let Some(idx) = url.find("://") {
        let after = &url[idx + 3..];
        if let Some(slash) = after.find('/') {
            url = format!("{}{}", &url[..idx + 3], &after[..slash]);
        }
    }
    url.trim_end_matches('/').to_string()
}

#[derive(Clone, Debug)]
pub enum MinerEvent {
    Status(String),
    /// Finished Fusion hashes / s. GPU path mirrors the same figure on both
    /// lanes (`cpu_hs == gpu_hs`). Never add them — use [`hashrate_fusion`].
    Hashrate { cpu_hs: f64, gpu_hs: f64 },
    BlockFound { height: u64, id: String },
    /// Verified AI / research job (GPU market).
    AiJobDone {
        job_id: String,
        kind: String,
        brain_epoch: Option<u64>,
    },
    Error(String),
    Stopped,
    AiStopped,
}

impl MinerEvent {
    /// Canonical Fusion H/s. GPU path already puts the same rate on both lanes.
    pub fn hashrate_fusion(cpu_hs: f64, gpu_hs: f64) -> f64 {
        meshhash_cpu::hashrate_fusion(cpu_hs, gpu_hs)
    }

    #[deprecated(note = "use hashrate_fusion — adding CPU+GPU double-counts the GPU path")]
    pub fn hashrate_total(cpu_hs: f64, gpu_hs: f64) -> f64 {
        Self::hashrate_fusion(cpu_hs, gpu_hs)
    }
}

pub use meshhash_cpu::format_hashrate;

pub fn cuda_available(device: i32) -> bool {
    #[cfg(mesh_cuda)]
    {
        let mut n = 0i32;
        let rc = unsafe { mesh_cuda_device_count(&mut n) };
        if rc != 0 || n <= 0 || device < 0 || device >= n {
            return false;
        }
        let set = unsafe { mesh_cuda_set_device(device) };
        set == 0
    }
    #[cfg(not(mesh_cuda))]
    {
        let _ = device;
        false
    }
}

pub fn amd_available(device: i32) -> bool {
    OpenClMixer::try_new(true, device).is_ok()
}

pub fn backend_status(backend: MinerBackend, device: i32) -> String {
    match backend {
        MinerBackend::Auto => {
            if cuda_available(device) {
                "Auto → NVIDIA CUDA ready".into()
            } else if opencl_gpu_available(true) {
                "Auto → AMD OpenCL ready".into()
            } else if opencl_gpu_available(false) {
                "Auto → OpenCL GPU ready".into()
            } else {
                "Auto → CPU mix".into()
            }
        }
        MinerBackend::Nvidia => {
            if cuda_available(device) {
                "NVIDIA CUDA ready".into()
            } else {
                "NVIDIA CUDA unavailable — select another device or rebuild with CUDA".into()
            }
        }
        MinerBackend::Amd => {
            if amd_available(device) {
                "AMD OpenCL ready".into()
            } else if opencl_gpu_available(false) {
                "AMD not found — other OpenCL GPU available".into()
            } else {
                "OpenCL unavailable — install AMD Adrenalin (or GPU OpenCL)".into()
            }
        }
        MinerBackend::Cpu => "CPU mix".into(),
    }
}

pub fn devices_status(devices: &[ComputeDevice]) -> String {
    if devices.is_empty() {
        return "No devices selected".into();
    }
    let names: Vec<String> = devices.iter().map(|d| d.short_label()).collect();
    format!("{} device(s): {}", devices.len(), names.join(" + "))
}

enum ActiveMix {
    #[cfg(mesh_cuda)]
    Cuda(CudaMixer),
    OpenCl(OpenClMixer),
    Cpu,
}

impl ActiveMix {
    fn is_cpu(&self) -> bool {
        matches!(self, ActiveMix::Cpu)
    }

    fn vram_bytes(&self) -> Option<u64> {
        match self {
            #[cfg(mesh_cuda)]
            ActiveMix::Cuda(c) => Some(c.vram_bytes()).filter(|&v| v > 0),
            ActiveMix::OpenCl(o) => Some(o.vram_bytes()).filter(|&v| v > 0),
            ActiveMix::Cpu => None,
        }
    }

    fn host_roundtrip_fold(&self) -> bool {
        matches!(self, ActiveMix::OpenCl(_))
    }

    fn mix_filled_pads(
        &mut self,
        host: Vec<u8>,
        difficulty: u32,
        params: &MeshHashParams,
        start_nonce: u64,
        batch: u32,
        stop: &AtomicBool,
        stale: &AtomicBool,
    ) -> Result<Option<u64>> {
        match self {
            #[cfg(mesh_cuda)]
            ActiveMix::Cuda(cuda) => {
                cuda.mix_filled_pads(host, difficulty, params, start_nonce, batch, stop, stale)
            }
            ActiveMix::OpenCl(ocl) => {
                ocl.mix_filled_pads(host, difficulty, params, start_nonce, batch, stop, stale)
            }
            ActiveMix::Cpu => bail!("CPU mix does not take prefilled pads"),
        }
    }
}

/// Resolve a selected device. GPU selections fail loudly (no silent CPU mix).
fn resolve_compute_device(dev: ComputeDevice) -> Result<(ActiveMix, String)> {
    match dev {
        ComputeDevice::Cpu => Ok((
            ActiveMix::Cpu,
            format!(
                "CPU MeshHash mix ({} threads)",
                cpu_parallelism()
            ),
        )),
        ComputeDevice::Cuda { index } => {
            #[cfg(mesh_cuda)]
            {
                let m = CudaMixer::try_new(index)
                    .with_context(|| format!("CUDA device {index} failed to init"))?;
                let name = m.name.clone();
                Ok((
                    ActiveMix::Cuda(m),
                    format!("CUDA MeshHash mix — {name}"),
                ))
            }
            #[cfg(not(mesh_cuda))]
            {
                let _ = index;
                bail!("This Miner build has no CUDA — rebuild with nvcc / CUDA toolkit");
            }
        }
        ComputeDevice::OpenCl { index } => {
            let m = OpenClMixer::try_new_any(index)
                .with_context(|| format!("OpenCL GPU index {index} failed to init"))?;
            let name = m.device_name().to_string();
            Ok((
                ActiveMix::OpenCl(m),
                format!("OpenCL MeshHash mix — {name}"),
            ))
        }
    }
}

fn resolve_mix(backend: MinerBackend, device: i32) -> Result<(ActiveMix, String)> {
    match backend {
        MinerBackend::Cpu => Ok((ActiveMix::Cpu, "Using CPU MeshHash mix".into())),
        MinerBackend::Nvidia => resolve_compute_device(ComputeDevice::Cuda { index: device }),
        MinerBackend::Amd => {
            let m = OpenClMixer::try_new(true, device).or_else(|_| OpenClMixer::try_new(false, device))
                .context("OpenCL / AMD GPU unavailable")?;
            let name = m.device_name().to_string();
            Ok((
                ActiveMix::OpenCl(m),
                format!("OpenCL MeshHash mix — {name}"),
            ))
        }
        MinerBackend::Auto => {
            if cuda_available(device) {
                resolve_compute_device(ComputeDevice::Cuda { index: device })
            } else if let Ok(m) = OpenClMixer::try_new(true, device) {
                let name = m.device_name().to_string();
                Ok((
                    ActiveMix::OpenCl(m),
                    format!("Auto: AMD OpenCL — {name}"),
                ))
            } else if let Ok(m) = OpenClMixer::try_new(false, device) {
                let name = m.device_name().to_string();
                Ok((ActiveMix::OpenCl(m), format!("Auto: OpenCL — {name}")))
            } else {
                Ok((ActiveMix::Cpu, "Auto: CPU MeshHash mix".into()))
            }
        }
    }
}

fn expand_workers(
    cfg: &MinerConfig,
) -> Result<(Vec<(ComputeDevice, ActiveMix, String)>, Vec<String>)> {
    // Build/31: MeshHash is CPU-verifiable. CPU and GPU may both search;
    // they share the contributor pot. GPU can still run AI jobs in parallel.
    if !cfg.devices.is_empty() {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut skips = Vec::new();
        let has_gpu_pick = cfg
            .devices
            .iter()
            .any(|d| !matches!(d, ComputeDevice::Cpu));
        for d in &cfg.devices {
            if !seen.insert(*d) {
                continue;
            }
            // GPU path already fills on CPU and seals Fusion. A second CPU miner
            // steals pad-fill cores and makes the all-in-one slower than GPU CLI.
            if has_gpu_pick && matches!(d, ComputeDevice::Cpu) {
                continue;
            }
            match resolve_compute_device(*d) {
                Ok((mix, status)) => out.push((*d, mix, status)),
                Err(e) => {
                    tracing::warn!(device = %d.short_label(), error = %e, "skip device");
                    skips.push(format!("{} failed — {e:#}", d.short_label()));
                }
            }
        }
        if out.is_empty() {
            bail!(skips
                .first()
                .cloned()
                .unwrap_or_else(|| "No mining devices selected".into()));
        }
        return Ok((out, skips));
    }
    match cfg.backend {
        MinerBackend::Auto => {
            if cuda_available(cfg.device) {
                return resolve_compute_device(ComputeDevice::Cuda { index: cfg.device }).map(
                    |(mix, status)| {
                        (
                            vec![(ComputeDevice::Cuda { index: cfg.device }, mix, status)],
                            Vec::new(),
                        )
                    },
                );
            }
            if let Ok((mix, status)) =
                resolve_compute_device(ComputeDevice::OpenCl { index: cfg.device })
            {
                return Ok((
                    vec![(ComputeDevice::OpenCl { index: cfg.device }, mix, status)],
                    Vec::new(),
                ));
            }
            let (mix, status) = resolve_compute_device(ComputeDevice::Cpu)?;
            return Ok((vec![(ComputeDevice::Cpu, mix, status)], Vec::new()));
        }
        MinerBackend::Nvidia | MinerBackend::Amd | MinerBackend::Cpu => {}
    }
    let (mix, status) = resolve_mix(cfg.backend, cfg.device)?;
    let id = match &mix {
        #[cfg(mesh_cuda)]
        ActiveMix::Cuda(_) => ComputeDevice::Cuda { index: cfg.device },
        ActiveMix::OpenCl(_) => ComputeDevice::OpenCl { index: cfg.device },
        ActiveMix::Cpu => ComputeDevice::Cpu,
    };
    Ok((vec![(id, mix, status)], Vec::new()))
}

enum WorkerEvent {
    Status(String),
    Hashrate { worker: usize, hs: f64 },
    BlockFound { height: u64, id: String },
    Error(String),
    Stopped { worker: usize },
}

/// Run continuous RPC mining until `stop` is set (one or more devices).
pub fn run_rpc_loop(cfg: MinerConfig, stop: Arc<AtomicBool>, tx: Sender<MinerEvent>) {
    // Prefer explicit pool override (legacy). Otherwise use the mine-target list as-is.
    let rpcs: Vec<String> = if let Some(pool) = cfg.pool.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        vec![normalize_pool_url(pool)]
    } else {
        let mut urls: Vec<String> = mesh_types::parse_rpc_list(&cfg.rpc)
            .into_iter()
            .map(|u| normalize_pool_url(&u))
            .filter(|u| !u.is_empty())
            .collect();
        if urls.is_empty() {
            urls = mesh_types::prefer_mine_rpc_urls();
            if let Some(extra) = discover_rpc_edges(urls.first().map(|s| s.as_str()).unwrap_or("")) {
                urls = mesh_types::public_pool_first(&mesh_types::merge_rpc_urls(&urls, &extra));
            }
        }
        urls
    };
    let worker_name = cfg.worker_name.clone();
    let (workers, skips) = match expand_workers(&cfg) {
        Ok(w) => w,
        Err(e) => {
            let _ = tx.send(MinerEvent::Error(format!("{e:#}")));
            let _ = tx.send(MinerEvent::Stopped);
            return;
        }
    };
    for s in skips {
        let _ = tx.send(MinerEvent::Error(s));
    }
    if workers.is_empty() {
        let _ = tx.send(MinerEvent::Error("No mining devices selected".into()));
        let _ = tx.send(MinerEvent::Stopped);
        return;
    }

    let has_cpu = workers.iter().any(|(_, mix, _)| mix.is_cpu());
    let has_gpu = workers.iter().any(|(_, mix, _)| !mix.is_cpu());
    set_cpu_mine_threads(has_cpu, has_gpu);
    if has_gpu {
        crate::gpu_gate::set_pow_holds_gpu(true);
    }
    struct CpuThreadGuard {
        gpu: bool,
    }
    impl Drop for CpuThreadGuard {
        fn drop(&mut self) {
            CPU_MINE_THREADS.store(0, Ordering::Relaxed);
            crate::host_pads::set_gpu_host_threads(0);
            if self.gpu {
                crate::gpu_gate::set_pow_holds_gpu(false);
            }
        }
    }
    let _cpu_thread_guard = CpuThreadGuard { gpu: has_gpu };
    if has_gpu {
        let _ = tx.send(MinerEvent::Status(format!(
            "GPU Fusion: {} CPU cores fill pads + seal. Same path as the GPU CLI (not a second CPU miner).",
            crate::host_pads::gpu_host_threads()
        )));
    } else if has_cpu {
        let _ = tx.send(MinerEvent::Status(format!(
            "CPU Fusion: {} cores (same as mesh-miner-cpu).",
            cpu_mine_threads()
        )));
    }

    let label = workers
        .iter()
        .map(|(d, _, _)| d.short_label())
        .collect::<Vec<_>>()
        .join(" + ");
    let _ = tx.send(MinerEvent::Status(format!("Mining with: {label}")));
    let _ = tx.send(MinerEvent::Status(format!(
        "Payout {} via {}",
        cfg.address,
        rpcs.join(" | ")
    )));

    if workers.len() == 1 {
        let (_dev, mix, status) = workers.into_iter().next().unwrap();
        let _ = tx.send(MinerEvent::Status(status));
        if let Some(v) = mix.vram_bytes() {
            let b = clamp_batch_xfer(cfg.batch, meshhash_cpu::SCRATCHPAD_SIZE, Some(v), false, None);
            let _ = tx.send(MinerEvent::Status(format!(
                "VRAM {} → up to {b} parallel pads (batch 0 = auto)",
                format_bytes(v)
            )));
        }
        let is_cpu = mix.is_cpu();
        run_single_worker(&rpcs, &cfg, mix, is_cpu, &stop, &tx);
        let _ = tx.send(MinerEvent::Stopped);
        return;
    }

    let (wtx, wrx) = mpsc::channel::<WorkerEvent>();
    let mut handles = Vec::new();
    let n = workers.len();
    let mut worker_is_cpu: HashMap<usize, bool> = HashMap::new();

    for (i, (_dev, mix, status)) in workers.into_iter().enumerate() {
        worker_is_cpu.insert(i, mix.is_cpu());
        let _ = tx.send(MinerEvent::Status(format!("[{i}] {status}")));
        let batch = cfg.batch.max(0); // 0 = auto
        if let Some(v) = mix.vram_bytes() {
            let b = clamp_batch_xfer(batch, meshhash_cpu::SCRATCHPAD_SIZE, Some(v), false, None);
            let _ = tx.send(MinerEvent::Status(format!(
                "[{i}] VRAM {} → up to {b} parallel pads (batch 0 = auto)",
                format_bytes(v)
            )));
        }
        let stop_c = stop.clone();
        let wtx_c = wtx.clone();
        let rpcs_c = rpcs.clone();
        let addr = cfg.address;
        let miner_id = miner_identity(&cfg.address, worker_name.as_deref());
        let max_nonces = cfg.max_nonces;
        handles.push(thread::spawn(move || {
            run_worker_loop(
                i,
                &rpcs_c,
                &addr,
                &miner_id,
                batch,
                max_nonces,
                mix,
                &stop_c,
                &wtx_c,
            );
            let _ = wtx_c.send(WorkerEvent::Stopped { worker: i });
        }));
    }
    drop(wtx);

    let mut rates: HashMap<usize, f64> = HashMap::new();
    let mut alive = n;
    while alive > 0 {
        match wrx.recv() {
            Ok(WorkerEvent::Status(s)) => {
                let _ = tx.send(MinerEvent::Status(s));
            }
            Ok(WorkerEvent::Hashrate { worker, hs }) => {
                rates.insert(worker, hs);
                let mut cpu_hs = 0.0;
                let mut gpu_hs = 0.0;
                for (w, r) in &rates {
                    if *worker_is_cpu.get(w).unwrap_or(&false) {
                        cpu_hs += *r;
                    } else {
                        gpu_hs += *r;
                    }
                }
                if cpu_hs == 0.0 && gpu_hs > 0.0 {
                    cpu_hs = gpu_hs;
                }
                let _ = tx.send(MinerEvent::Hashrate { cpu_hs, gpu_hs });
            }
            Ok(WorkerEvent::BlockFound { height, id }) => {
                let _ = tx.send(MinerEvent::BlockFound { height, id });
            }
            Ok(WorkerEvent::Error(e)) => {
                let _ = tx.send(MinerEvent::Error(e));
            }
            Ok(WorkerEvent::Stopped { .. }) => {
                alive = alive.saturating_sub(1);
            }
            Err(_) => break,
        }
    }

    for h in handles {
        let _ = h.join();
    }
    let _ = tx.send(MinerEvent::Stopped);
}

fn run_single_worker(
    rpcs: &[String],
    cfg: &MinerConfig,
    mut mix: ActiveMix,
    is_cpu: bool,
    stop: &AtomicBool,
    tx: &Sender<MinerEvent>,
) {
    let mut window = RateWindow::new();
    let mut rpc_i = 0usize;

    while !stop.load(Ordering::SeqCst) {
        let rpc = &rpcs[rpc_i % rpcs.len()];
        match mine_one_rpc(
            rpc,
            &cfg.address,
            &miner_identity(&cfg.address, cfg.worker_name.as_deref()),
            cfg.batch, // 0 = auto (CPU: threads×16)
            cfg.max_nonces,
            &mut mix,
            stop,
            Some(tx),
            |batch_n, _elapsed| {
                let hs = window.push(batch_n as u64);
                if window.should_send() && hs > 0.05 {
                    let (cpu_hs, gpu_hs) = if is_cpu {
                        (hs, 0.0)
                    } else {
                        // One Fusion digest is GPU wave + CPU seal.
                        (hs, hs)
                    };
                    let _ = tx.send(MinerEvent::Hashrate { cpu_hs, gpu_hs });
                }
            },
        ) {
            Ok(Some((height, id))) => {
                let _ = tx.send(MinerEvent::BlockFound { height, id });
            }
            Ok(None) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let pause = Duration::from_millis(if is_cpu { 5 } else { 8 });
                thread::sleep(pause);
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if is_race_noise(&msg) {
                    // Swallow — already handled as Ok(None) for most cases.
                } else {
                    let _ = tx.send(MinerEvent::Error(format!("{msg} ({rpc})")));
                    rpc_i = rpc_i.wrapping_add(1);
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn run_worker_loop(
    worker: usize,
    rpcs: &[String],
    payout: &Address,
    miner_id: &str,
    batch: u32,
    max_nonces: u64,
    mut mix: ActiveMix,
    stop: &AtomicBool,
    tx: &Sender<WorkerEvent>,
) {
    let mut window = RateWindow::new();
    let mut rpc_i = 0usize;

    while !stop.load(Ordering::SeqCst) {
        let rpc = &rpcs[rpc_i % rpcs.len()];
        match mine_one_rpc(
            rpc,
            payout,
            miner_id,
            batch,
            max_nonces,
            &mut mix,
            stop,
            None,
            |batch_n, _elapsed| {
                let hs = window.push(batch_n as u64);
                if window.should_send() && hs > 0.05 {
                    let _ = tx.send(WorkerEvent::Hashrate { worker, hs });
                }
            },
        ) {
            Ok(Some((height, id))) => {
                let _ = tx.send(WorkerEvent::BlockFound { height, id });
            }
            Ok(None) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(8));
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if !is_race_noise(&msg) {
                    let _ = tx.send(WorkerEvent::Error(format!("[{worker}] {msg} ({rpc})")));
                    rpc_i = rpc_i.wrapping_add(1);
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[derive(Deserialize)]
struct TemplateResp {
    difficulty: u32,
    #[serde(default)]
    soft_diff_hint: Option<u32>,
    light_pow: bool,
    #[serde(default)]
    pow_version: u8,
    block_hex: String,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    exam_scenario: String,
    #[serde(default)]
    exam_title: String,
    #[serde(default)]
    exam_payload_hex: String,
    #[serde(default)]
    exam_job_id: String,
}

#[derive(Deserialize)]
struct SubmitResp {
    accepted: bool,
    height: u64,
    id: String,
}

fn mine_one_rpc(
    rpc: &str,
    payout: &Address,
    miner_id: &str,
    batch: u32,
    max_nonces: u64,
    mix: &mut ActiveMix,
    stop: &AtomicBool,
    exam_tx: Option<&Sender<MinerEvent>>,
    mut on_batch: impl FnMut(u32, Duration),
) -> Result<Option<(u64, String)>> {
    let addr_q = gbt_address_param(rpc, payout, miner_id);
    let url = format!("{rpc}/v1/getblocktemplate?address={addr_q}");
    let mut get_req = ureq::get(&url).timeout(Duration::from_secs(30));
    get_req = get_req.set("X-Mesh-Miner", miner_id);
    let resp = get_req
        .call()
        .with_context(|| format!("GET {url}"))?;
    if !(200..300).contains(&resp.status()) {
        bail!("GET {url} -> {}", resp.status());
    }
    let tmpl: TemplateResp = resp.into_json()?;
    if let Some(tx) = exam_tx {
        if !tmpl.exam_payload_hex.is_empty() {
            let hint = crate::exam::ExamHint {
                height: tmpl.height,
                scenario: tmpl.exam_scenario.clone(),
                title: tmpl.exam_title.clone(),
                payload_hex: tmpl.exam_payload_hex.clone(),
                job_id: tmpl.exam_job_id.clone(),
            };
            let rpc_owned = rpc.to_string();
            let payout_owned = *payout;
            let tx = tx.clone();
            thread::spawn(move || {
                crate::exam::try_submit_exam(&rpc_owned, &payout_owned, &hint, &tx);
            });
        }
    }
    let soft = tmpl.soft_diff_hint.unwrap_or(tmpl.difficulty);
    if soft != tmpl.difficulty {
        tracing::info!(
            height = tmpl.height,
            consensus_diff = tmpl.difficulty,
            soft_diff_hint = soft,
            "research soft mining hint (validation still uses consensus_diff)"
        );
    }
    let bytes = hex::decode(&tmpl.block_hex)?;
    let mut block: Block = bincode::deserialize(&bytes)?;
    let pow_version = if tmpl.pow_version == 0 { 1 } else { tmpl.pow_version };
    let commitment = block.header.pre_pow_commitment();
    let (work_seed, params) = pow_search_inputs(
        &commitment,
        tmpl.light_pow,
        block.header.height,
        &block.header.prev_hash,
    );
    if params.version >= 5 && mix.is_cpu() {
        bail!(
            "Fusion sequential (pow v5): GPU must run the wave, then this CPU seals. Enable NVIDIA/AMD."
        );
    }
    let batch = clamp_batch_xfer(
        batch,
        params.scratchpad_size,
        mix.vram_bytes(),
        mix.host_roundtrip_fold(),
        Some(tmpl.difficulty),
    );
    tracing::debug!(
        batch,
        pad = params.scratchpad_size,
        vram = ?mix.vram_bytes(),
        diff = tmpl.difficulty,
        light = tmpl.light_pow,
        pow_version,
        "pow batch scaled to device VRAM and difficulty"
    );
    let job_height = block.header.height;
    let stale = Arc::new(AtomicBool::new(false));
    let watch = spawn_tip_watch(rpc.to_string(), job_height, stale.clone());
    let found = search_nonces(
        &work_seed,
        tmpl.difficulty,
        &params,
        batch,
        max_nonces,
        mix,
        stop,
        &stale,
        &mut on_batch,
    )?;
    watch.store(true, Ordering::Relaxed);
    let Some(nonce) = found else {
        return Ok(None);
    };
    if stale.load(Ordering::Relaxed) || tip_moved(rpc, job_height) {
        tracing::debug!(height = job_height, "tip moved before Fusion seal");
        return Ok(None);
    }
    block.header.nonce = nonce;
    // CPU seal: rematch the bound digest on this tip. That is the Fusion block.
    let pow = meshhash_cpu_with_params(&work_seed, nonce, &params);
    if !pow.meets_difficulty(tmpl.difficulty) {
        tracing::warn!(nonce, height = job_height, "GPU nonce failed CPU Fusion seal");
        return Ok(None);
    }
    if tip_moved(rpc, job_height) {
        tracing::debug!(height = job_height, "tip moved during Fusion seal");
        return Ok(None);
    }
    if mesh_types::exam_required_for_block(job_height) {
        if let Some(tx) = exam_tx {
            let hint = crate::exam::ExamHint {
                height: tmpl.height,
                scenario: tmpl.exam_scenario.clone(),
                title: tmpl.exam_title.clone(),
                payload_hex: tmpl.exam_payload_hex.clone(),
                job_id: tmpl.exam_job_id.clone(),
            };
            if !crate::exam::ensure_exam_match(rpc, payout, &hint, tx) {
                bail!("exam MATCH required before submitblock at height {job_height}");
            }
        }
    }
    let body = json!({ "block_hex": hex::encode(bincode::serialize(&block)?) });
    let mut req = ureq::post(&format!(
        "{rpc}/v1/submitblock?address={}",
        gbt_address_param(rpc, payout, miner_id)
    ))
    .timeout(Duration::from_secs(30));
    req = req.set("X-Mesh-Miner", miner_id);
    if let Ok(token) = std::env::var("MESH_RPC_TOKEN") {
        let t = token.trim();
        if !t.is_empty() {
            req = req.set("X-Mesh-Token", t);
        }
    }
    match req.send_json(body) {
        Ok(resp) => {
            let submit: SubmitResp = resp.into_json()?;
            if submit.accepted {
                Ok(Some((submit.height, submit.id)))
            } else {
                Ok(None)
            }
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            if is_stale_reject(code, &text) {
                // Another device already advanced the tip — normal race, not an error.
                Ok(None)
            } else {
                bail!(
                    "submitblock HTTP {code}: {}",
                    text.chars().take(120).collect::<String>()
                );
            }
        }
        Err(e) => Err(e.into()),
    }
}

fn is_stale_reject(code: u16, body: &str) -> bool {
    if code != 400 {
        return false;
    }
    let low = body.to_ascii_lowercase();
    low.contains("stale")
        || low.contains("tip moved")
        || low.contains("tip kept moving")
        || low.is_empty()
}

fn is_race_noise(msg: &str) -> bool {
    let low = msg.to_ascii_lowercase();
    (low.contains("submitblock") && low.contains("400"))
        || low.contains("stale block")
        || low.contains("tip moved")
}

fn search_nonces(
    commitment: &Hash,
    difficulty: u32,
    params: &MeshHashParams,
    batch: u32,
    max_nonces: u64,
    mix: &mut ActiveMix,
    stop: &AtomicBool,
    stale: &AtomicBool,
    on_batch: &mut impl FnMut(u32, Duration),
) -> Result<Option<u64>> {
    let batch = batch.max(1);
    if mix.is_cpu() {
        let mut start = 0u64;
        while start < max_nonces
            && !stop.load(Ordering::Relaxed)
            && !stale.load(Ordering::Relaxed)
        {
            let n = batch.min((max_nonces - start) as u32);
            let t0 = Instant::now();
            let found = search_batch_once(commitment, difficulty, params, start, n, mix, stop, stale)?;
            if !stale.load(Ordering::Relaxed) {
                on_batch(n, t0.elapsed());
            }
            if let Some(nonce) = found {
                return Ok(Some(nonce));
            }
            start += n as u64;
        }
        return Ok(None);
    }

    // GPU: fill batch N+1 on the reserved host cores while this batch mixes.
    let mut start = 0u64;
    let first_n = batch.min(max_nonces as u32);
    if first_n == 0 {
        return Ok(None);
    }
    let fill0 = Instant::now();
    let Some(mut host) =
        crate::host_pads::fill_pads_parallel(commitment, params, start, first_n, stop, stale)
    else {
        return Ok(None);
    };
    let first_fill = fill0.elapsed();
    while start < max_nonces
        && !stop.load(Ordering::Relaxed)
        && !stale.load(Ordering::Relaxed)
    {
        let n = batch.min((max_nonces - start) as u32);
        let next_start = start + n as u64;
        let next_n = if next_start < max_nonces {
            Some(batch.min((max_nonces - next_start) as u32))
        } else {
            None
        };
        let t0 = Instant::now();
        let mut next_host = None;
        let found = thread::scope(|scope| -> Result<Option<u64>> {
            let fill_h = next_n.map(|nn| {
                scope.spawn(move || {
                    crate::host_pads::fill_pads_parallel(
                        commitment,
                        params,
                        next_start,
                        nn,
                        stop,
                        stale,
                    )
                })
            });
            let found = mix.mix_filled_pads(host, difficulty, params, start, n, stop, stale)?;
            if let Some(h) = fill_h {
                next_host = h.join().unwrap_or(None);
            }
            Ok(found)
        })?;
        if !stale.load(Ordering::Relaxed) {
            let elapsed = if start == 0 {
                first_fill + t0.elapsed()
            } else {
                t0.elapsed()
            };
            on_batch(n, elapsed);
        }
        if let Some(nonce) = found {
            return Ok(Some(nonce));
        }
        start = next_start;
        match (next_n, next_host) {
            (None, _) => break,
            (Some(_), Some(h)) => host = h,
            (Some(_), None) => return Ok(None),
        }
    }
    Ok(None)
}

fn search_batch_once(
    commitment: &Hash,
    difficulty: u32,
    params: &MeshHashParams,
    start_nonce: u64,
    batch: u32,
    mix: &mut ActiveMix,
    stop: &AtomicBool,
    stale: &AtomicBool,
) -> Result<Option<u64>> {
    match mix {
        #[cfg(mesh_cuda)]
        ActiveMix::Cuda(cuda) => {
            cuda.search_batch(commitment, difficulty, params, start_nonce, batch, stop, stale)
        }
        ActiveMix::OpenCl(ocl) => {
            ocl.search_batch(commitment, difficulty, params, start_nonce, batch, stop, stale)
        }
        ActiveMix::Cpu => {
            // Leave cores for GPU pad-fill when both lanes run.
            let threads = cpu_mine_threads();
            let batch = batch as u64;
            if batch == 0 {
                return Ok(None);
            }
            let found = AtomicU64::new(u64::MAX);
            let pad_len = params.scratchpad_size;
            let params = params.clone();
            thread::scope(|scope| {
                let chunk = (batch as usize).div_ceil(threads).max(1);
                for t in 0..threads {
                    let start = start_nonce + (t * chunk) as u64;
                    let end = (start_nonce + batch).min(start + chunk as u64);
                    if start >= end {
                        continue;
                    }
                    let found = &found;
                    let stop = stop;
                    let stale = stale;
                    let commitment = *commitment;
                    let params = params.clone();
                    scope.spawn(move || {
                        let mut host_pad = vec![0u8; pad_len];
                        for nonce in start..end {
                            if stop.load(Ordering::Relaxed) || stale.load(Ordering::Relaxed) {
                                return;
                            }
                            if found.load(Ordering::Relaxed) != u64::MAX {
                                return;
                            }
                            fill_scratchpad_for_nonce(&commitment, nonce, &params, &mut host_pad);
                            mix_scratchpad_with_params(&mut host_pad, &params);
                            let pow = fold_pow(&host_pad, &params);
                            if pow.meets_difficulty(difficulty) {
                                let _ = found.fetch_update(
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                    |cur| {
                                        if cur == u64::MAX || nonce < cur {
                                            Some(nonce)
                                        } else {
                                            None
                                        }
                                    },
                                );
                                return;
                            }
                        }
                    });
                }
            });
            let n = found.load(Ordering::Relaxed);
            if n == u64::MAX {
                Ok(None)
            } else {
                Ok(Some(n))
            }
        }
    }
}

fn peek_tip_height(rpc: &str) -> Option<u64> {
    #[derive(Deserialize)]
    struct Tip {
        #[serde(default)]
        height: u64,
    }
    let mut urls = vec![format!("{}/v1/getnodeinfo", rpc.trim_end_matches('/'))];
    let seed = mesh_types::default_seed_rpc_url();
    let seed_url = format!("{}/v1/getnodeinfo", seed.trim_end_matches('/'));
    if !urls.iter().any(|u| u.eq_ignore_ascii_case(&seed_url)) {
        urls.push(seed_url);
    }
    for url in urls {
        let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(3)).call() else {
            continue;
        };
        if let Ok(tip) = resp.into_json::<Tip>() {
            return Some(tip.height);
        }
    }
    None
}

fn tip_moved(rpc: &str, job_height: u64) -> bool {
    peek_tip_height(rpc).is_some_and(|h| h >= job_height)
}

fn spawn_tip_watch(rpc: String, job_height: u64, stale: Arc<AtomicBool>) -> Arc<AtomicBool> {
    let done = Arc::new(AtomicBool::new(false));
    let watch_done = done.clone();
    thread::spawn(move || {
        while !watch_done.load(Ordering::Relaxed) {
            if tip_moved(&rpc, job_height) {
                stale.store(true, Ordering::Relaxed);
                break;
            }
            thread::sleep(Duration::from_millis(750));
        }
    });
    done
}

/// Pull `edges` from `/v1/getnodeinfo` when present.
fn discover_rpc_edges(rpc: &str) -> Option<Vec<String>> {
    if rpc.is_empty() {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct Info {
        #[serde(default)]
        edges: Vec<String>,
    }
    let url = format!("{}/v1/getnodeinfo", rpc.trim_end_matches('/'));
    let info: Info = ureq::get(&url)
        .timeout(Duration::from_secs(3))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    if info.edges.is_empty() {
        None
    } else {
        Some(info.edges)
    }
}

#[cfg(all(test, mesh_cuda))]
mod cuda_fold_tests {
    use super::*;
    use meshhash_cpu::MeshHashParams;

    #[test]
    fn device_extract_matches_cpu() {
        let mut cuda = CudaMixer::try_new(0).expect("CUDA device 0");
        let mut params = MeshHashParams::light();
        params.version = 4;
        params.fold_salt = 11;
        let seed = mesh_types::Hash::digest(b"cuda-fold-audit");
        let stop = AtomicBool::new(false);
        let stale = AtomicBool::new(false);
        let n = 4u32;
        let host = crate::host_pads::fill_pads_parallel(&seed, &params, 0, n, &stop, &stale)
            .expect("fill pads");
        cuda.mix_filled_pads(host, 64, &params, 0, n, &stop, &stale)
            .expect("CUDA mix + fold extract must match CPU rematch");
        assert!(cuda.fold_checked);
    }
}

#[cfg(test)]
mod batch_cap_tests {
    use super::*;

    #[test]
    fn low_difficulty_caps_gpu_auto_batch() {
        let vram = Some(12 * 1024 * 1024 * 1024u64);
        let pad = 16 * 1024 * 1024usize;
        let low = clamp_batch_xfer(0, pad, vram, false, Some(1));
        let mid = clamp_batch_xfer(0, pad, vram, false, Some(3));
        let uncapped = clamp_batch_xfer(0, pad, vram, false, None);
        assert_eq!(low, luck_batch_cap(1));
        assert_eq!(mid, luck_batch_cap(3));
        assert!(uncapped > low);
        assert!(low <= MAX_LUCK_BATCH);
        assert!(mid <= MAX_LUCK_BATCH);
    }

    #[test]
    fn luck_cap_grows_with_difficulty_then_stops() {
        assert_eq!(luck_batch_cap(1), 8);
        assert_eq!(luck_batch_cap(2), 16);
        assert_eq!(luck_batch_cap(3), 32);
        assert_eq!(luck_batch_cap(10), MAX_LUCK_BATCH);
    }
}
