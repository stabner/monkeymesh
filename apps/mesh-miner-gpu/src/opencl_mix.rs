//! Runtime OpenCL MeshHash mix (AMD / any OpenCL GPU).
//! Pads stay in device memory; host stages one pad at a time.
//! Loads `OpenCL.dll` dynamically — no OpenCL SDK required at build time.

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};
use libloading::Library;
use mesh_types::Hash;
use meshhash_cpu::{finish_pow_mix, MeshHashParams};

type ClInt = i32;
type ClUint = u32;
type ClBitfield = u64;
type ClBool = ClUint;
type ClDeviceType = ClBitfield;
type ClPlatformId = *mut c_void;
type ClDeviceId = *mut c_void;
type ClContext = *mut c_void;
type ClCommandQueue = *mut c_void;
type ClMem = *mut c_void;
type ClProgram = *mut c_void;
type ClKernel = *mut c_void;

const CL_SUCCESS: ClInt = 0;
const CL_DEVICE_TYPE_GPU: ClDeviceType = 1 << 2;
const CL_DEVICE_NAME: ClUint = 0x102B;
const CL_DEVICE_VENDOR: ClUint = 0x102C;
const CL_DEVICE_GLOBAL_MEM_SIZE: ClUint = 0x101F;
const CL_MEM_READ_WRITE: ClBitfield = 1 << 0;
const CL_TRUE: ClBool = 1;
const CL_FALSE: ClBool = 0;

const KERNEL_SRC: &str = r#"
inline ulong rotl64(ulong x, uint n) {
    return (x << n) | (x >> (64u - n));
}

inline ulong rotr64(ulong x, uint n) {
    return (x >> n) | (x << (64u - n));
}

void mesh_mix_one(__global uchar *pad, ulong pad_len, uint rounds) {
    ulong mask = pad_len - 8;
    __global ulong *p64 = (__global ulong *)pad;
    ulong state = p64[0];

    for (uint i = 0; i < rounds; i++) {
        ulong idx_a = ((state ^ ((ulong)i * (ulong)0x9E3779B97F4A7C15UL)) & mask) & ~(ulong)7;
        ulong a = *(__global ulong *)(pad + idx_a);

        ulong idx_b = ((rotl64(a, 17) ^ state) & mask) & ~(ulong)7;
        ulong b = *(__global ulong *)(pad + idx_b);

        uint rot = (uint)(state % (ulong)63) + 1u;
        a = a + b;
        a = rotl64(a, rot);
        a ^= state * (ulong)0xD6E8FEB86659FD93UL;

        *(__global ulong *)(pad + idx_a) = a;
        state = state + a + (ulong)0xA0761D6478BD642FUL;
    }
}

void mesh_mix_one_reverse(__global uchar *pad, ulong pad_len, uint rounds) {
    ulong mask = pad_len - 8;
    ulong state = *(__global ulong *)(pad + (pad_len - 8));

    for (uint i = 0; i < rounds; i++) {
        ulong rev_i = (ulong)(rounds - 1u - i);
        ulong idx_a = ((state ^ (rev_i * (ulong)0xC2B2AE3D27D4EB4FUL)) & mask) & ~(ulong)7;
        ulong a = *(__global ulong *)(pad + idx_a);

        ulong idx_b = ((rotr64(a, 13) ^ rotl64(state, 7)) & mask) & ~(ulong)7;
        ulong b = *(__global ulong *)(pad + idx_b);

        uint rot = (uint)(state % (ulong)63) + 1u;
        a = a * (ulong)0x94D049BB133111EBUL;
        a = rotr64(a, rot);
        a ^= b + state;

        *(__global ulong *)(pad + idx_a) = a;
        state = state + a + (ulong)0x85EBCA77C2B2AE63UL;
    }
}

__kernel void mesh_mix_batch(__global uchar *pads, ulong pad_len, uint rounds, uint batch) {
    uint idx = get_global_id(0);
    if (idx >= batch) return;
    mesh_mix_one(pads + (ulong)idx * pad_len, pad_len, rounds);
}

__kernel void mesh_mix_reverse_batch(__global uchar *pads, ulong pad_len, uint rounds, uint batch) {
    uint idx = get_global_id(0);
    if (idx >= batch) return;
    mesh_mix_one_reverse(pads + (ulong)idx * pad_len, pad_len, rounds);
}
"#;

struct Api {
    _lib: Library,
    get_platform_ids: unsafe extern "system" fn(ClUint, *mut ClPlatformId, *mut ClUint) -> ClInt,
    get_device_ids: unsafe extern "system" fn(
        ClPlatformId,
        ClDeviceType,
        ClUint,
        *mut ClDeviceId,
        *mut ClUint,
    ) -> ClInt,
    get_device_info: unsafe extern "system" fn(
        ClDeviceId,
        ClUint,
        usize,
        *mut c_void,
        *mut usize,
    ) -> ClInt,
    create_context: unsafe extern "system" fn(
        *const isize,
        ClUint,
        *const ClDeviceId,
        Option<unsafe extern "system" fn(*const c_char, *const c_void, usize, *mut c_void)>,
        *mut c_void,
        *mut ClInt,
    ) -> ClContext,
    create_command_queue: unsafe extern "system" fn(
        ClContext,
        ClDeviceId,
        ClBitfield,
        *mut ClInt,
    ) -> ClCommandQueue,
    create_buffer: unsafe extern "system" fn(
        ClContext,
        ClBitfield,
        usize,
        *mut c_void,
        *mut ClInt,
    ) -> ClMem,
    create_program_with_source: unsafe extern "system" fn(
        ClContext,
        ClUint,
        *mut *const c_char,
        *const usize,
        *mut ClInt,
    ) -> ClProgram,
    build_program: unsafe extern "system" fn(
        ClProgram,
        ClUint,
        *const ClDeviceId,
        *const c_char,
        Option<unsafe extern "system" fn(ClProgram, *mut c_void)>,
        *mut c_void,
    ) -> ClInt,
    create_kernel: unsafe extern "system" fn(ClProgram, *const c_char, *mut ClInt) -> ClKernel,
    set_kernel_arg: unsafe extern "system" fn(ClKernel, ClUint, usize, *const c_void) -> ClInt,
    enqueue_ndrange: unsafe extern "system" fn(
        ClCommandQueue,
        ClKernel,
        ClUint,
        *const usize,
        *const usize,
        *const usize,
        ClUint,
        *const *mut c_void,
        *mut *mut c_void,
    ) -> ClInt,
    enqueue_write: unsafe extern "system" fn(
        ClCommandQueue,
        ClMem,
        ClBool,
        usize,
        usize,
        *const c_void,
        ClUint,
        *const *mut c_void,
        *mut *mut c_void,
    ) -> ClInt,
    enqueue_read: unsafe extern "system" fn(
        ClCommandQueue,
        ClMem,
        ClBool,
        usize,
        usize,
        *mut c_void,
        ClUint,
        *const *mut c_void,
        *mut *mut c_void,
    ) -> ClInt,
    finish: unsafe extern "system" fn(ClCommandQueue) -> ClInt,
    release_mem: unsafe extern "system" fn(ClMem) -> ClInt,
    release_kernel: unsafe extern "system" fn(ClKernel) -> ClInt,
    release_program: unsafe extern "system" fn(ClProgram) -> ClInt,
    release_queue: unsafe extern "system" fn(ClCommandQueue) -> ClInt,
    release_context: unsafe extern "system" fn(ClContext) -> ClInt,
}

impl Api {
    unsafe fn load() -> Result<Self> {
        let lib = Library::new("OpenCL.dll")
            .or_else(|_| Library::new("opencl.dll"))
            .context(
                "OpenCL.dll not found (install a GPU driver with OpenCL — AMD Adrenalin / NVIDIA)",
            )?;
        Ok(Self {
            get_platform_ids: *lib.get(b"clGetPlatformIDs")?,
            get_device_ids: *lib.get(b"clGetDeviceIDs")?,
            get_device_info: *lib.get(b"clGetDeviceInfo")?,
            create_context: *lib.get(b"clCreateContext")?,
            create_command_queue: *lib.get(b"clCreateCommandQueue")?,
            create_buffer: *lib.get(b"clCreateBuffer")?,
            create_program_with_source: *lib.get(b"clCreateProgramWithSource")?,
            build_program: *lib.get(b"clBuildProgram")?,
            create_kernel: *lib.get(b"clCreateKernel")?,
            set_kernel_arg: *lib.get(b"clSetKernelArg")?,
            enqueue_ndrange: *lib.get(b"clEnqueueNDRangeKernel")?,
            enqueue_write: *lib.get(b"clEnqueueWriteBuffer")?,
            enqueue_read: *lib.get(b"clEnqueueReadBuffer")?,
            finish: *lib.get(b"clFinish")?,
            release_mem: *lib.get(b"clReleaseMemObject")?,
            release_kernel: *lib.get(b"clReleaseKernel")?,
            release_program: *lib.get(b"clReleaseProgram")?,
            release_queue: *lib.get(b"clReleaseCommandQueue")?,
            release_context: *lib.get(b"clReleaseContext")?,
            _lib: lib,
        })
    }
}

pub struct OpenClMixer {
    api: Api,
    device_name: String,
    vram_bytes: u64,
    context: ClContext,
    queue: ClCommandQueue,
    program: ClProgram,
    kernel: ClKernel,
    kernel_rev: ClKernel,
    /// Reused device buffer (grows on demand).
    buf: ClMem,
    buf_bytes: usize,
}

unsafe impl Send for OpenClMixer {}

impl Drop for OpenClMixer {
    fn drop(&mut self) {
        unsafe {
            if !self.buf.is_null() {
                let _ = (self.api.release_mem)(self.buf);
            }
            let _ = (self.api.release_kernel)(self.kernel);
            let _ = (self.api.release_kernel)(self.kernel_rev);
            let _ = (self.api.release_program)(self.program);
            let _ = (self.api.release_queue)(self.queue);
            let _ = (self.api.release_context)(self.context);
        }
    }
}

impl OpenClMixer {
    /// Prefer AMD GPU devices when `prefer_amd` is true; otherwise any GPU.
    pub fn try_new(prefer_amd: bool, device_index: i32) -> Result<Self> {
        unsafe {
            let api = Api::load()?;
            let mut nplat = 0u32;
            check((api.get_platform_ids)(0, ptr::null_mut(), &mut nplat), "clGetPlatformIDs")?;
            if nplat == 0 {
                bail!("no OpenCL platforms");
            }
            let mut plats = vec![ptr::null_mut(); nplat as usize];
            check(
                (api.get_platform_ids)(nplat, plats.as_mut_ptr(), ptr::null_mut()),
                "clGetPlatformIDs",
            )?;

            let mut candidates: Vec<(ClDeviceId, String, bool)> = Vec::new();
            for p in plats {
                let mut ndev = 0u32;
                let rc = (api.get_device_ids)(p, CL_DEVICE_TYPE_GPU, 0, ptr::null_mut(), &mut ndev);
                if rc != CL_SUCCESS || ndev == 0 {
                    continue;
                }
                let mut devs = vec![ptr::null_mut(); ndev as usize];
                if (api.get_device_ids)(
                    p,
                    CL_DEVICE_TYPE_GPU,
                    ndev,
                    devs.as_mut_ptr(),
                    ptr::null_mut(),
                ) != CL_SUCCESS
                {
                    continue;
                }
                for d in devs {
                    let name = device_str(&api, d, CL_DEVICE_NAME).unwrap_or_else(|_| "?".into());
                    let vendor =
                        device_str(&api, d, CL_DEVICE_VENDOR).unwrap_or_else(|_| "?".into());
                    let low_v = vendor.to_ascii_lowercase();
                    let low_n = name.to_ascii_lowercase();
                    let is_amd = low_v.contains("advanced micro")
                        || low_v.contains("amd")
                        || low_n.contains("radeon")
                        || low_n.contains("amd");
                    candidates.push((d, format!("{name} ({vendor})"), is_amd));
                }
            }
            if candidates.is_empty() {
                bail!("no OpenCL GPU devices found");
            }

            let filtered: Vec<_> = if prefer_amd {
                let amd: Vec<_> = candidates.iter().filter(|c| c.2).cloned().collect();
                if amd.is_empty() {
                    candidates
                } else {
                    amd
                }
            } else {
                candidates
            };

            let idx = device_index.max(0) as usize % filtered.len();
            let (device, device_name, _) = filtered[idx].clone();
            let vram_bytes = device_u64(&api, device, CL_DEVICE_GLOBAL_MEM_SIZE).unwrap_or(0);

            let mut err = 0i32;
            let context =
                (api.create_context)(ptr::null(), 1, &device, None, ptr::null_mut(), &mut err);
            check(err, "clCreateContext")?;
            let queue = (api.create_command_queue)(context, device, 0, &mut err);
            check(err, "clCreateCommandQueue")?;

            let src = CString::new(KERNEL_SRC)?;
            let mut src_ptr = src.as_ptr();
            let program =
                (api.create_program_with_source)(context, 1, &mut src_ptr, ptr::null(), &mut err);
            check(err, "clCreateProgramWithSource")?;
            let build =
                (api.build_program)(program, 1, &device, ptr::null(), None, ptr::null_mut());
            check(build, "clBuildProgram")?;

            let kname = CString::new("mesh_mix_batch")?;
            let kernel = (api.create_kernel)(program, kname.as_ptr(), &mut err);
            check(err, "clCreateKernel")?;
            let krev = CString::new("mesh_mix_reverse_batch")?;
            let kernel_rev = (api.create_kernel)(program, krev.as_ptr(), &mut err);
            check(err, "clCreateKernel reverse")?;

            Ok(Self {
                api,
                device_name,
                vram_bytes,
                context,
                queue,
                program,
                kernel,
                kernel_rev,
                buf: ptr::null_mut(),
                buf_bytes: 0,
            })
        }
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn vram_bytes(&self) -> u64 {
        self.vram_bytes
    }

    fn ensure_buf(&mut self, bytes: usize) -> Result<()> {
        if bytes == 0 {
            bail!("OpenCL ensure 0 bytes");
        }
        if bytes <= self.buf_bytes && !self.buf.is_null() {
            return Ok(());
        }
        unsafe {
            if !self.buf.is_null() {
                let _ = (self.api.release_mem)(self.buf);
                self.buf = ptr::null_mut();
                self.buf_bytes = 0;
            }
            let mut err = 0i32;
            let mem = (self.api.create_buffer)(
                self.context,
                CL_MEM_READ_WRITE,
                bytes,
                ptr::null_mut(),
                &mut err,
            );
            check(err, "clCreateBuffer")?;
            self.buf = mem;
            self.buf_bytes = bytes;
        }
        Ok(())
    }

    unsafe fn enqueue_mix(&self, kernel: ClKernel, pad_len: usize, rounds: u32, batch: u32) -> Result<()> {
        let pad_len_u = pad_len as u64;
        check(
            (self.api.set_kernel_arg)(
                kernel,
                0,
                std::mem::size_of::<ClMem>(),
                &self.buf as *const _ as *const c_void,
            ),
            "clSetKernelArg0",
        )?;
        check(
            (self.api.set_kernel_arg)(
                kernel,
                1,
                std::mem::size_of::<u64>(),
                &pad_len_u as *const _ as *const c_void,
            ),
            "clSetKernelArg1",
        )?;
        check(
            (self.api.set_kernel_arg)(
                kernel,
                2,
                std::mem::size_of::<u32>(),
                &rounds as *const _ as *const c_void,
            ),
            "clSetKernelArg2",
        )?;
        check(
            (self.api.set_kernel_arg)(
                kernel,
                3,
                std::mem::size_of::<u32>(),
                &batch as *const _ as *const c_void,
            ),
            "clSetKernelArg3",
        )?;
        let global = batch as usize;
        check(
            (self.api.enqueue_ndrange)(
                self.queue,
                kernel,
                1,
                ptr::null(),
                &global,
                ptr::null(),
                0,
                ptr::null(),
                ptr::null_mut(),
            ),
            "clEnqueueNDRangeKernel",
        )
    }

    /// Parallel CPU fill → GPU forward (+ reverse) → parallel fold.
    pub fn search_batch(
        &mut self,
        commitment: &Hash,
        difficulty: u32,
        params: &MeshHashParams,
        start_nonce: u64,
        batch: u32,
        stop: &AtomicBool,
    ) -> Result<Option<u64>> {
        if batch == 0 || params.scratchpad_size < 64 {
            bail!("bad OpenCL search args");
        }
        let Some(host) =
            crate::host_pads::fill_pads_parallel(commitment, params, start_nonce, batch, stop)
        else {
            return Ok(None);
        };
        self.mix_filled_pads(host, difficulty, params, start_nonce, batch, stop)
    }

    pub fn mix_filled_pads(
        &mut self,
        mut host: Vec<u8>,
        difficulty: u32,
        params: &MeshHashParams,
        start_nonce: u64,
        batch: u32,
        stop: &AtomicBool,
    ) -> Result<Option<u64>> {
        if batch == 0 || params.scratchpad_size < 64 {
            bail!("bad OpenCL search args");
        }
        let pad_len = params.scratchpad_size;
        let rounds = params.mix_rounds as u32;
        let bytes = pad_len.saturating_mul(batch as usize);
        if host.len() < bytes {
            bail!("OpenCL host pad buffer short ({} < {})", host.len(), bytes);
        }
        self.ensure_buf(bytes)?;

        unsafe {
            check(
                (self.api.enqueue_write)(
                    self.queue,
                    self.buf,
                    CL_TRUE,
                    0,
                    bytes,
                    host.as_ptr() as *const c_void,
                    0,
                    ptr::null(),
                    ptr::null_mut(),
                ),
                "clEnqueueWriteBuffer pads",
            )?;
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            self.enqueue_mix(self.kernel, pad_len, rounds, batch)?;
            let _ = (self.api.finish)(self.queue);
            if params.version >= 2 {
                if stop.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                self.enqueue_mix(self.kernel_rev, pad_len, rounds, batch)?;
                let _ = (self.api.finish)(self.queue);
            }
            check(
                (self.api.enqueue_read)(
                    self.queue,
                    self.buf,
                    CL_TRUE,
                    0,
                    bytes,
                    host.as_mut_ptr() as *mut c_void,
                    0,
                    ptr::null(),
                    ptr::null_mut(),
                ),
                "clEnqueueReadBuffer pads",
            )?;
        }
        if params.version < 2 {
            for pad in host.chunks_exact_mut(pad_len) {
                finish_pow_mix(pad, params);
            }
        }
        Ok(crate::host_pads::fold_pads_parallel(
            &mut host,
            params,
            start_nonce,
            difficulty,
            stop,
        ))
    }
}

fn check(rc: ClInt, what: &str) -> Result<()> {
    if rc == CL_SUCCESS {
        Ok(())
    } else {
        bail!("{what} failed: OpenCL error {rc}")
    }
}

unsafe fn device_str(api: &Api, device: ClDeviceId, param: ClUint) -> Result<String> {
    let mut size = 0usize;
    check(
        (api.get_device_info)(device, param, 0, ptr::null_mut(), &mut size),
        "clGetDeviceInfo size",
    )?;
    let mut buf = vec![0u8; size.max(1)];
    check(
        (api.get_device_info)(
            device,
            param,
            buf.len(),
            buf.as_mut_ptr() as *mut c_void,
            ptr::null_mut(),
        ),
        "clGetDeviceInfo",
    )?;
    let cstr = CStr::from_ptr(buf.as_ptr() as *const c_char);
    Ok(cstr.to_string_lossy().trim_end_matches('\0').to_string())
}

unsafe fn device_u64(api: &Api, device: ClDeviceId, param: ClUint) -> Result<u64> {
    let mut val = 0u64;
    check(
        (api.get_device_info)(
            device,
            param,
            std::mem::size_of::<u64>(),
            &mut val as *mut _ as *mut c_void,
            ptr::null_mut(),
        ),
        "clGetDeviceInfo u64",
    )?;
    Ok(val)
}

/// Probe whether OpenCL GPUs exist (optionally AMD-preferring).
pub fn opencl_gpu_available(prefer_amd: bool) -> bool {
    OpenClMixer::try_new(prefer_amd, 0).is_ok()
}

#[derive(Clone, Debug)]
pub struct OpenClGpuInfo {
    pub index: i32,
    pub name: String,
    pub vendor: String,
    pub is_amd: bool,
    pub is_nvidia: bool,
    pub vram_bytes: u64,
}

/// List all OpenCL GPU devices (stable index for `OpenClMixer::try_new_any`).
pub fn list_opencl_gpus() -> Vec<OpenClGpuInfo> {
    unsafe {
        let Ok(api) = Api::load() else {
            return Vec::new();
        };
        let mut nplat = 0u32;
        if (api.get_platform_ids)(0, ptr::null_mut(), &mut nplat) != CL_SUCCESS || nplat == 0 {
            return Vec::new();
        }
        let mut plats = vec![ptr::null_mut(); nplat as usize];
        if (api.get_platform_ids)(nplat, plats.as_mut_ptr(), ptr::null_mut()) != CL_SUCCESS {
            return Vec::new();
        }

        let mut out = Vec::new();
        let mut index = 0i32;
        for p in plats {
            let mut ndev = 0u32;
            let rc = (api.get_device_ids)(p, CL_DEVICE_TYPE_GPU, 0, ptr::null_mut(), &mut ndev);
            if rc != CL_SUCCESS || ndev == 0 {
                continue;
            }
            let mut devs = vec![ptr::null_mut(); ndev as usize];
            if (api.get_device_ids)(
                p,
                CL_DEVICE_TYPE_GPU,
                ndev,
                devs.as_mut_ptr(),
                ptr::null_mut(),
            ) != CL_SUCCESS
            {
                continue;
            }
            for d in devs {
                let name = device_str(&api, d, CL_DEVICE_NAME).unwrap_or_else(|_| "?".into());
                let vendor = device_str(&api, d, CL_DEVICE_VENDOR).unwrap_or_else(|_| "?".into());
                let low_v = vendor.to_ascii_lowercase();
                let low_n = name.to_ascii_lowercase();
                let is_amd = low_v.contains("advanced micro")
                    || low_v.contains("amd")
                    || low_n.contains("radeon")
                    || low_n.contains("amd");
                let is_nvidia = low_v.contains("nvidia") || low_n.contains("nvidia");
                let vram_bytes = device_u64(&api, d, CL_DEVICE_GLOBAL_MEM_SIZE).unwrap_or(0);
                out.push(OpenClGpuInfo {
                    index,
                    name: format!("{name} ({vendor})"),
                    vendor,
                    is_amd,
                    is_nvidia,
                    vram_bytes,
                });
                index += 1;
            }
        }
        out
    }
}

impl OpenClMixer {
    /// Open a mixer for the `index`-th GPU from [`list_opencl_gpus`] (no AMD filter).
    pub fn try_new_any(device_index: i32) -> Result<Self> {
        Self::try_new(false, device_index)
    }
}
