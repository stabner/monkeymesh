//! Shared multi-backend miner engine (NVIDIA CUDA / AMD OpenCL / CPU).

pub mod ai_worker;
pub mod engine;
pub mod exam;
pub mod gpu_gate;
pub mod host_pads;
pub mod opencl_mix;

pub use engine::{
    ai_capacity_from_selection, amd_available, backend_status, clamp_batch, cuda_available,
    cuda_device_count, devices_status, enumerate_devices, format_hashrate, run_rpc_loop,
    miner_identity, scratch_budget_bytes, ComputeDevice, DeviceInfo, MinerBackend, MinerConfig,
    MinerEvent,
};
