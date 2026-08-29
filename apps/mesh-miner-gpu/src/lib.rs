//! Shared multi-backend miner engine (NVIDIA CUDA / AMD OpenCL / CPU).

pub mod ai_worker;
pub mod engine;
pub mod exam;
pub mod gpu_gate;
pub mod host_pads;
pub mod opencl_mix;

pub use ai_worker::{run_ai_loop, AiCapacity};
pub use engine::{
    ai_capacity_from_selection, amd_available, backend_status, clamp_batch, cuda_available,
    cuda_device_count, devices_status, enumerate_devices, format_hashrate, looks_like_pool_target,
    miner_identity, run_rpc_loop, scratch_budget_bytes, ComputeDevice, DeviceInfo, MinerBackend,
    MinerConfig, MinerEvent,
};
