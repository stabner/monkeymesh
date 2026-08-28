//! CUDA MeshHash: pads live in VRAM; host only stages one pad at a time.
//! Fill stays CPU (Blake3 must match node verify); mix runs on device.

#include <cstdint>
#include <cstring>
#include <cuda_runtime.h>

extern "C" {

struct MeshCudaCtx {
    int device;
    uint8_t* dev;
    size_t capacity;
    // Mix `state` lives in a register on CPU; chunked kernels must carry it
    // separately. Reloading pad[0:8] each chunk resets the mix (wrong hashes).
    uint64_t* state;
    size_t state_slots;
    uint64_t* programs;
    size_t program_slots;
    uint8_t* samples;
    size_t samples_cap;
    uint8_t* wave_acc;
    size_t wave_slots;
};

__device__ void mesh_mix_one(
    uint8_t* pad,
    size_t pad_len,
    uint32_t start_round,
    uint32_t rounds,
    uint64_t* state_slot
) {
    uint64_t mask = (uint64_t)(pad_len - 8);
    uint64_t state;
    if (start_round == 0) {
        memcpy(&state, pad, 8);
    } else {
        state = *state_slot;
    }

    for (uint32_t k = 0; k < rounds; k++) {
        uint32_t i = start_round + k;
        uint64_t idx_a =
            (((state ^ ((uint64_t)i * 0x9E3779B97F4A7C15ULL)) & mask)) & ~7ULL;
        uint64_t a;
        memcpy(&a, pad + idx_a, 8);

        uint64_t idx_b = ((a << 17 | a >> (64 - 17)) ^ state) & mask;
        idx_b &= ~7ULL;
        uint64_t b;
        memcpy(&b, pad + idx_b, 8);

        uint32_t rot = (uint32_t)(state % 63) + 1;
        a = a + b;
        a = (a << rot) | (a >> (64 - rot));
        a ^= state * 0xD6E8FEB86659FD93ULL;

        memcpy(pad + idx_a, &a, 8);
        state = state + a + 0xA0761D6478BD642FULL;
    }
    *state_slot = state;
}

__device__ void mesh_mix_one_reverse(
    uint8_t* pad,
    size_t pad_len,
    uint32_t start_round,
    uint32_t rounds,
    uint32_t total_rounds,
    uint64_t* state_slot
) {
    uint64_t mask = (uint64_t)(pad_len - 8);
    uint64_t state;
    if (start_round == 0) {
        memcpy(&state, pad + (pad_len - 8), 8);
    } else {
        state = *state_slot;
    }

    for (uint32_t k = 0; k < rounds; k++) {
        uint32_t i = start_round + k;
        uint64_t rev_i = (uint64_t)(total_rounds - 1 - i);
        uint64_t idx_a =
            (((state ^ (rev_i * 0xC2B2AE3D27D4EB4FULL)) & mask)) & ~7ULL;
        uint64_t a;
        memcpy(&a, pad + idx_a, 8);

        uint64_t rotl7 = (state << 7) | (state >> (64 - 7));
        uint64_t idx_b = (((a >> 13) | (a << (64 - 13))) ^ rotl7) & mask;
        idx_b &= ~7ULL;
        uint64_t b;
        memcpy(&b, pad + idx_b, 8);

        uint32_t rot = (uint32_t)(state % 63) + 1;
        a = a * 0x94D049BB133111EBULL;
        a = (a >> rot) | (a << (64 - rot));
        a ^= b + state;

        memcpy(pad + idx_a, &a, 8);
        state = state + a + 0x85EBCA77C2B2AE63ULL;
    }
    *state_slot = state;
}

__global__ void mesh_mix_reverse_batch_k(
    uint8_t* pads,
    size_t pad_len,
    uint32_t start_round,
    uint32_t rounds,
    uint32_t total_rounds,
    uint32_t start_index,
    uint32_t count,
    uint64_t* states
) {
    uint32_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= count) return;
    uint32_t slot = start_index + idx;
    mesh_mix_one_reverse(
        pads + (size_t)slot * pad_len,
        pad_len,
        start_round,
        rounds,
        total_rounds,
        &states[slot]
    );
}

__device__ __forceinline__ uint64_t mesh_rotl(uint64_t x, uint32_t n) {
    n &= 63u;
    return (x << n) | (x >> (64u - n));
}

__device__ __forceinline__ uint64_t mesh_rotr(uint64_t x, uint32_t n) {
    n &= 63u;
    return (x >> n) | (x << (64u - n));
}

__device__ uint64_t mesh_fusion_alu(uint8_t op, uint64_t a, uint64_t b) {
    switch (op & 7) {
    case 0:
        return a + b;
    case 1:
        return a ^ b;
    case 2: {
        uint32_t r = (uint32_t)(b & 63ull);
        if (r == 0) r = 1;
        return mesh_rotl(a, r) ^ b;
    }
    case 3:
        return a * (b | 1ull);
    case 4:
        return (a & b) + (a | b);
    case 5:
        return a - mesh_rotl(b, 11);
    case 6: {
        uint32_t r = (uint32_t)(b & 63ull);
        if (r == 0) r = 1;
        return mesh_rotr(a, r) + b;
    }
    default:
        return a ^ (b * 0xD6E8FEB86659FD93ULL);
    }
}

__device__ void mesh_fusion_wave_one(
    const uint8_t* pad,
    size_t pad_len,
    const uint64_t* prog,
    uint8_t* acc_bytes
) {
    uint64_t mask = (uint64_t)(pad_len - 8);
    for (int i = 0; i < 32; i++) acc_bytes[i] = 0;
    for (uint32_t lane = 0; lane < 32; lane++) {
        uint64_t acc = prog[lane] ^ ((uint64_t)lane * 0xA0761D6478BD642FULL);
        for (uint32_t step = 0; step < 64; step++) {
            uint64_t idx =
                ((acc ^ prog[step % 32]) * 0x94D049BB133111EBULL) & mask;
            idx &= ~7ULL;
            uint64_t word;
            memcpy(&word, pad + idx, 8);
            uint8_t op = (uint8_t)((prog[lane] >> ((step & 7u) * 3u)) ^ (uint64_t)step);
            acc = mesh_fusion_alu(op, acc, word);
            uint64_t idx2 = (mesh_rotl(acc, 17) ^ (uint64_t)step) & mask;
            idx2 &= ~7ULL;
            uint64_t word2;
            memcpy(&word2, pad + idx2, 8);
            acc = mesh_rotl(acc + word2, 7);
        }
        uint8_t lane_bytes[8];
        memcpy(lane_bytes, &acc, 8);
        for (int i = 0; i < 8; i++) {
            acc_bytes[i] ^= lane_bytes[i];
            uint32_t j = (uint32_t)(i + (int)lane) % 32u;
            acc_bytes[j] = (uint8_t)(acc_bytes[j] + lane_bytes[i]);
        }
    }
}

__global__ void mesh_fold_extract_k(
    const uint8_t* pads,
    size_t pad_len,
    uint32_t count,
    const uint64_t* programs,
    uint8_t* samples_out,
    uint8_t* wave_acc_out,
    size_t sample_stride,
    uint32_t sample_count,
    int do_wave
) {
    uint32_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= count) return;
    const uint8_t* pad = pads + (size_t)idx * pad_len;
    uint8_t* dst = samples_out + (size_t)idx * (size_t)sample_count * 32ull;
    for (uint32_t s = 0; s < sample_count; s++) {
        size_t i = (size_t)s * sample_stride;
        if (i >= pad_len) break;
        size_t n = pad_len - i;
        if (n > 32) n = 32;
        memcpy(dst + (size_t)s * 32ull, pad + i, n);
        if (n < 32) {
            memset(dst + (size_t)s * 32ull + n, 0, 32 - n);
        }
    }
    if (do_wave) {
        mesh_fusion_wave_one(
            pad,
            pad_len,
            programs + (size_t)idx * 32ull,
            wave_acc_out + (size_t)idx * 32ull
        );
    }
}

__global__ void mesh_mix_batch_k(
    uint8_t* pads,
    size_t pad_len,
    uint32_t start_round,
    uint32_t rounds,
    uint32_t start_index,
    uint32_t count,
    uint64_t* states
) {
    uint32_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= count) return;
    uint32_t slot = start_index + idx;
    mesh_mix_one(
        pads + (size_t)slot * pad_len,
        pad_len,
        start_round,
        rounds,
        &states[slot]
    );
}

MeshCudaCtx* mesh_cuda_ctx_create(int device_id) {
    int n = 0;
    if (cudaGetDeviceCount(&n) != cudaSuccess || device_id < 0 || device_id >= n) {
        return nullptr;
    }
    if (cudaSetDevice(device_id) != cudaSuccess) {
        return nullptr;
    }
    auto* ctx = new MeshCudaCtx();
    ctx->device = device_id;
    ctx->dev = nullptr;
    ctx->capacity = 0;
    ctx->state = nullptr;
    ctx->state_slots = 0;
    ctx->programs = nullptr;
    ctx->program_slots = 0;
    ctx->samples = nullptr;
    ctx->samples_cap = 0;
    ctx->wave_acc = nullptr;
    ctx->wave_slots = 0;
    return ctx;
}

void mesh_cuda_ctx_destroy(MeshCudaCtx* ctx) {
    if (!ctx) return;
    cudaSetDevice(ctx->device);
    if (ctx->dev) {
        cudaFree(ctx->dev);
    }
    if (ctx->state) {
        cudaFree(ctx->state);
    }
    if (ctx->programs) {
        cudaFree(ctx->programs);
    }
    if (ctx->samples) {
        cudaFree(ctx->samples);
    }
    if (ctx->wave_acc) {
        cudaFree(ctx->wave_acc);
    }
    delete ctx;
}

int mesh_cuda_ctx_ensure(MeshCudaCtx* ctx, size_t bytes) {
    if (!ctx || bytes == 0) return -1;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    if (!(bytes <= ctx->capacity && ctx->dev)) {
        if (ctx->dev) {
            cudaFree(ctx->dev);
            ctx->dev = nullptr;
            ctx->capacity = 0;
        }
        err = cudaMalloc(&ctx->dev, bytes);
        if (err != cudaSuccess) return (int)err;
        ctx->capacity = bytes;
    }
    // Worst-case pad count (min pad 64 B) so chunked mix can store `state`.
    size_t slots = bytes / 64;
    if (slots < 1) slots = 1;
    if (slots > ctx->state_slots || !ctx->state) {
        if (ctx->state) {
            cudaFree(ctx->state);
            ctx->state = nullptr;
            ctx->state_slots = 0;
        }
        err = cudaMalloc(&ctx->state, slots * sizeof(uint64_t));
        if (err != cudaSuccess) return (int)err;
        ctx->state_slots = slots;
    }
    return 0;
}

int mesh_cuda_ctx_upload_pad(
    MeshCudaCtx* ctx,
    uint32_t index,
    size_t pad_len,
    const uint8_t* host_pad
) {
    if (!ctx || !host_pad || !ctx->dev || pad_len < 64) return -1;
    size_t off = (size_t)index * pad_len;
    if (off + pad_len > ctx->capacity) return -2;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    err = cudaMemcpy(ctx->dev + off, host_pad, pad_len, cudaMemcpyHostToDevice);
    return err == cudaSuccess ? 0 : (int)err;
}

int mesh_cuda_ctx_mix_range(
    MeshCudaCtx* ctx,
    size_t pad_len,
    uint32_t start_round,
    uint32_t rounds,
    uint32_t start_index,
    uint32_t count
) {
    if (!ctx || !ctx->dev || !ctx->state || count == 0 || pad_len < 64) return -1;
    size_t need = pad_len * ((size_t)start_index + (size_t)count);
    if (need > ctx->capacity) return -2;
    if ((size_t)start_index + (size_t)count > ctx->state_slots) return -3;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    uint32_t threads = 128;
    uint32_t blocks = (count + threads - 1) / threads;
    mesh_mix_batch_k<<<blocks, threads>>>(
        ctx->dev, pad_len, start_round, rounds, start_index, count, ctx->state
    );
    err = cudaGetLastError();
    return err == cudaSuccess ? 0 : (int)err;
}

int mesh_cuda_ctx_mix_reverse_range(
    MeshCudaCtx* ctx,
    size_t pad_len,
    uint32_t start_round,
    uint32_t rounds,
    uint32_t total_rounds,
    uint32_t start_index,
    uint32_t count
) {
    if (!ctx || !ctx->dev || !ctx->state || count == 0 || pad_len < 64) return -1;
    if (total_rounds == 0 || start_round + rounds > total_rounds) return -4;
    size_t need = pad_len * ((size_t)start_index + (size_t)count);
    if (need > ctx->capacity) return -2;
    if ((size_t)start_index + (size_t)count > ctx->state_slots) return -3;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    uint32_t threads = 128;
    uint32_t blocks = (count + threads - 1) / threads;
    mesh_mix_reverse_batch_k<<<blocks, threads>>>(
        ctx->dev, pad_len, start_round, rounds, total_rounds, start_index, count, ctx->state
    );
    err = cudaGetLastError();
    return err == cudaSuccess ? 0 : (int)err;
}

int mesh_cuda_ctx_upload_range(
    MeshCudaCtx* ctx,
    uint32_t start_index,
    uint32_t count,
    size_t pad_len,
    const uint8_t* host
) {
    if (!ctx || !host || !ctx->dev || count == 0 || pad_len < 64) return -1;
    size_t off = (size_t)start_index * pad_len;
    size_t bytes = (size_t)count * pad_len;
    if (off + bytes > ctx->capacity) return -2;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    err = cudaMemcpy(ctx->dev + off, host, bytes, cudaMemcpyHostToDevice);
    return err == cudaSuccess ? 0 : (int)err;
}

int mesh_cuda_ctx_download_range(
    MeshCudaCtx* ctx,
    uint32_t start_index,
    uint32_t count,
    size_t pad_len,
    uint8_t* host
) {
    if (!ctx || !host || !ctx->dev || count == 0 || pad_len < 64) return -1;
    size_t off = (size_t)start_index * pad_len;
    size_t bytes = (size_t)count * pad_len;
    if (off + bytes > ctx->capacity) return -2;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    err = cudaMemcpy(host, ctx->dev + off, bytes, cudaMemcpyDeviceToHost);
    return err == cudaSuccess ? 0 : (int)err;
}

int mesh_cuda_ctx_mix(MeshCudaCtx* ctx, size_t pad_len, uint32_t rounds, uint32_t batch) {
    return mesh_cuda_ctx_mix_range(ctx, pad_len, 0, rounds, 0, batch);
}

int mesh_cuda_ctx_synchronize(MeshCudaCtx* ctx) {
    if (!ctx) return -1;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    err = cudaDeviceSynchronize();
    return err == cudaSuccess ? 0 : (int)err;
}

int mesh_cuda_host_register(int device_id, void* p, size_t n) {
    if (!p || n == 0) return -1;
    cudaError_t err = cudaSetDevice(device_id);
    if (err != cudaSuccess) return (int)err;
    err = cudaHostRegister(p, n, cudaHostRegisterDefault);
    return err == cudaSuccess ? 0 : (int)err;
}

int mesh_cuda_host_unregister(int device_id, void* p) {
    if (!p) return -1;
    cudaError_t err = cudaSetDevice(device_id);
    if (err != cudaSuccess) return (int)err;
    err = cudaHostUnregister(p);
    return err == cudaSuccess ? 0 : (int)err;
}

int mesh_cuda_ctx_download_heads(
    MeshCudaCtx* ctx,
    size_t pad_len,
    uint32_t count,
    uint8_t* host
) {
    if (!ctx || !host || !ctx->dev || count == 0 || pad_len < 32) return -1;
    size_t need = pad_len * (size_t)count;
    if (need > ctx->capacity) return -2;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    err = cudaMemcpy2D(
        host,
        32,
        ctx->dev,
        pad_len,
        32,
        count,
        cudaMemcpyDeviceToHost
    );
    return err == cudaSuccess ? 0 : (int)err;
}

static int mesh_cuda_ctx_ensure_fold(
    MeshCudaCtx* ctx,
    uint32_t count,
    size_t samples_bytes
) {
    cudaError_t err;
    size_t prog_slots = (size_t)count * 32ull;
    if (prog_slots > ctx->program_slots || !ctx->programs) {
        if (ctx->programs) {
            cudaFree(ctx->programs);
            ctx->programs = nullptr;
            ctx->program_slots = 0;
        }
        err = cudaMalloc(&ctx->programs, prog_slots * sizeof(uint64_t));
        if (err != cudaSuccess) return (int)err;
        ctx->program_slots = prog_slots;
    }
    if (samples_bytes > ctx->samples_cap || !ctx->samples) {
        if (ctx->samples) {
            cudaFree(ctx->samples);
            ctx->samples = nullptr;
            ctx->samples_cap = 0;
        }
        err = cudaMalloc(&ctx->samples, samples_bytes);
        if (err != cudaSuccess) return (int)err;
        ctx->samples_cap = samples_bytes;
    }
    size_t wave_slots = (size_t)count;
    if (wave_slots > ctx->wave_slots || !ctx->wave_acc) {
        if (ctx->wave_acc) {
            cudaFree(ctx->wave_acc);
            ctx->wave_acc = nullptr;
            ctx->wave_slots = 0;
        }
        err = cudaMalloc(&ctx->wave_acc, wave_slots * 32ull);
        if (err != cudaSuccess) return (int)err;
        ctx->wave_slots = wave_slots;
    }
    return 0;
}

int mesh_cuda_ctx_fold_extract(
    MeshCudaCtx* ctx,
    size_t pad_len,
    uint32_t count,
    size_t sample_stride,
    uint32_t sample_count,
    const uint64_t* host_programs,
    uint8_t* host_samples,
    uint8_t* host_wave_acc,
    int do_wave
) {
    if (!ctx || !ctx->dev || count == 0 || pad_len < 64 || sample_stride == 0 || sample_count == 0) {
        return -1;
    }
    if (do_wave && !host_programs) return -1;
    if (!host_samples) return -1;
    if (do_wave && !host_wave_acc) return -1;
    size_t samples_bytes = (size_t)count * (size_t)sample_count * 32ull;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    int rc = mesh_cuda_ctx_ensure_fold(ctx, count, samples_bytes);
    if (rc != 0) return rc;
    if (do_wave) {
        err = cudaMemcpy(
            ctx->programs,
            host_programs,
            (size_t)count * 32ull * sizeof(uint64_t),
            cudaMemcpyHostToDevice
        );
        if (err != cudaSuccess) return (int)err;
    }
    uint32_t threads = 128;
    uint32_t blocks = (count + threads - 1) / threads;
    mesh_fold_extract_k<<<blocks, threads>>>(
        ctx->dev,
        pad_len,
        count,
        ctx->programs,
        ctx->samples,
        ctx->wave_acc,
        sample_stride,
        sample_count,
        do_wave
    );
    err = cudaGetLastError();
    if (err != cudaSuccess) return (int)err;
    err = cudaMemcpy(host_samples, ctx->samples, samples_bytes, cudaMemcpyDeviceToHost);
    if (err != cudaSuccess) return (int)err;
    if (do_wave) {
        err = cudaMemcpy(host_wave_acc, ctx->wave_acc, (size_t)count * 32ull, cudaMemcpyDeviceToHost);
        if (err != cudaSuccess) return (int)err;
    }
    return 0;
}

int mesh_cuda_ctx_download_pad(
    MeshCudaCtx* ctx,
    uint32_t index,
    size_t pad_len,
    uint8_t* host_pad
) {
    if (!ctx || !host_pad || !ctx->dev || pad_len < 64) return -1;
    size_t off = (size_t)index * pad_len;
    if (off + pad_len > ctx->capacity) return -2;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    err = cudaMemcpy(host_pad, ctx->dev + off, pad_len, cudaMemcpyDeviceToHost);
    return err == cudaSuccess ? 0 : (int)err;
}

// Legacy bulk API (large host buffer) — kept for older call sites.
int mesh_cuda_ctx_mix_batch(
    MeshCudaCtx* ctx,
    uint8_t* host_pads,
    size_t pad_len,
    uint32_t rounds,
    uint32_t batch
) {
    if (!ctx || !host_pads || batch == 0 || pad_len < 64) return -1;
    size_t bytes = pad_len * (size_t)batch;
    int rc = mesh_cuda_ctx_ensure(ctx, bytes);
    if (rc != 0) return rc;
    cudaError_t err = cudaSetDevice(ctx->device);
    if (err != cudaSuccess) return (int)err;
    err = cudaMemcpy(ctx->dev, host_pads, bytes, cudaMemcpyHostToDevice);
    if (err != cudaSuccess) return (int)err;
    rc = mesh_cuda_ctx_mix(ctx, pad_len, rounds, batch);
    if (rc != 0) return rc;
    rc = mesh_cuda_ctx_synchronize(ctx);
    if (rc != 0) return rc;
    err = cudaMemcpy(host_pads, ctx->dev, bytes, cudaMemcpyDeviceToHost);
    return err == cudaSuccess ? 0 : (int)err;
}

int mesh_cuda_mix_batch(uint8_t* host_pads, size_t pad_len, uint32_t rounds, uint32_t batch) {
    MeshCudaCtx* ctx = mesh_cuda_ctx_create(0);
    if (!ctx) return -1;
    int rc = mesh_cuda_ctx_mix_batch(ctx, host_pads, pad_len, rounds, batch);
    mesh_cuda_ctx_destroy(ctx);
    return rc;
}

int mesh_cuda_device_count(int* out) {
    return (int)cudaGetDeviceCount(out);
}

int mesh_cuda_set_device(int id) {
    return (int)cudaSetDevice(id);
}

int mesh_cuda_device_name(int device_id, char* out, int out_len) {
    if (!out || out_len < 2) return -1;
    out[0] = '\0';
    cudaDeviceProp prop;
    cudaError_t err = cudaGetDeviceProperties(&prop, device_id);
    if (err != cudaSuccess) return (int)err;
    int major = prop.major;
    int minor = prop.minor;
    const char* name = prop.name;
    int i = 0;
    while (name[i] && i < out_len - 16) {
        out[i] = name[i];
        i++;
    }
    const char* suffix_pre = " (sm_";
    for (int j = 0; suffix_pre[j] && i < out_len - 8; j++) out[i++] = suffix_pre[j];
    if (i < out_len - 4) {
        out[i++] = (char)('0' + (major % 10));
        out[i++] = (char)('0' + (minor % 10));
        out[i++] = ')';
    }
    out[i < out_len ? i : out_len - 1] = '\0';
    return 0;
}

int mesh_cuda_device_vram_bytes(int device_id, unsigned long long* out) {
    if (!out) return -1;
    *out = 0;
    cudaDeviceProp prop;
    cudaError_t err = cudaGetDeviceProperties(&prop, device_id);
    if (err != cudaSuccess) return (int)err;
    *out = (unsigned long long)prop.totalGlobalMem;
    return 0;
}

} // extern C
