// Shared-brain v2 CUDA train — Q16.16 mlp512, bit-exact with Rust CPU contract.
// Layer parallelism: one thread per output row; input reduce is sequential (order-matched).

#include <cuda_runtime.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

static const int FRAC = 16;
static const int32_t ONE = 1 << 16;
static const int INPUT = 784;
static const int H1 = 512;
static const int H2 = 128;
static const int OUT = 10;
static const int N_W1 = H1 * INPUT;
static const int N_B1 = H1;
static const int N_W2 = H2 * H1;
static const int N_B2 = H2;
static const int N_W3 = OUT * H2;
static const int N_B3 = OUT;
static const int N_WEIGHTS = N_W1 + N_B1 + N_W2 + N_B2 + N_W3 + N_B3;

__device__ __forceinline__ int32_t qmul(int32_t a, int32_t b) {
    return (int32_t)(((int64_t)a * (int64_t)b) >> FRAC);
}

__device__ __forceinline__ int32_t qadd(int32_t a, int32_t b) {
    int64_t s = (int64_t)a + (int64_t)b;
    if (s > 2147483647LL) return 2147483647;
    if (s < -2147483648LL) return (int32_t)(-2147483648LL);
    return (int32_t)s;
}

__device__ __forceinline__ int32_t qsub(int32_t a, int32_t b) {
    int64_t s = (int64_t)a - (int64_t)b;
    if (s > 2147483647LL) return 2147483647;
    if (s < -2147483648LL) return (int32_t)(-2147483648LL);
    return (int32_t)s;
}

__device__ __forceinline__ int32_t qrelu(int32_t x) { return x > 0 ? x : 0; }

__device__ __forceinline__ int32_t clamp_q(int32_t x) {
    int32_t lo = -ONE * 8;
    int32_t hi = ONE * 8;
    if (x < lo) return lo;
    if (x > hi) return hi;
    return x;
}

struct MlpDev {
    int32_t* w1; // N_W1
    int32_t* b1;
    int32_t* w2;
    int32_t* b2;
    int32_t* w3;
    int32_t* b3;
};

// Single-thread train step (bit-identical to Rust train_step_correct).
// Gradients applied in-place (output→input) so we avoid multi-MB device stacks.
__device__ int32_t train_step_seq(MlpDev m, const int32_t* x, uint8_t y, int32_t lr) {
    int32_t h1_pre[H1];
    int32_t h1[H1];
    for (int j = 0; j < H1; j++) {
        int32_t s = m.b1[j];
        int row = j * INPUT;
        for (int i = 0; i < INPUT; i++) {
            s = qadd(s, qmul(m.w1[row + i], x[i]));
        }
        h1_pre[j] = s;
        h1[j] = qrelu(s);
    }
    int32_t h2_pre[H2];
    int32_t h2[H2];
    for (int j = 0; j < H2; j++) {
        int32_t s = m.b2[j];
        int row = j * H1;
        for (int i = 0; i < H1; i++) {
            s = qadd(s, qmul(m.w2[row + i], h1[i]));
        }
        h2_pre[j] = s;
        h2[j] = qrelu(s);
    }
    int32_t logits[OUT];
    for (int c = 0; c < OUT; c++) {
        int32_t s = m.b3[c];
        int row = c * H2;
        for (int j = 0; j < H2; j++) {
            s = qadd(s, qmul(m.w3[row + j], h2[j]));
        }
        logits[c] = clamp_q(s);
    }

    int32_t loss = 0;
    int32_t d_logits[OUT];
    for (int c = 0; c < OUT; c++) {
        int32_t target = (c == (int)y) ? ONE : 0;
        int32_t err = qsub(logits[c], target);
        loss = qadd(loss, qmul(err, err));
        d_logits[c] = err;
    }

    int32_t d_h2[H2];
    for (int j = 0; j < H2; j++) d_h2[j] = 0;
    for (int c = 0; c < OUT; c++) {
        int row = c * H2;
        int32_t g = d_logits[c];
        m.b3[c] = clamp_q(qsub(m.b3[c], qmul(lr, g)));
        for (int j = 0; j < H2; j++) {
            int32_t dw = qmul(g, h2[j]);
            d_h2[j] = qadd(d_h2[j], qmul(g, m.w3[row + j]));
            m.w3[row + j] = clamp_q(qsub(m.w3[row + j], qmul(lr, dw)));
        }
    }
    for (int j = 0; j < H2; j++) {
        if (h2_pre[j] <= 0) d_h2[j] = 0;
    }

    int32_t d_h1[H1];
    for (int i = 0; i < H1; i++) d_h1[i] = 0;
    for (int j = 0; j < H2; j++) {
        int row = j * H1;
        int32_t g = d_h2[j];
        m.b2[j] = clamp_q(qsub(m.b2[j], qmul(lr, g)));
        for (int i = 0; i < H1; i++) {
            int32_t dw = qmul(g, h1[i]);
            d_h1[i] = qadd(d_h1[i], qmul(g, m.w2[row + i]));
            m.w2[row + i] = clamp_q(qsub(m.w2[row + i], qmul(lr, dw)));
        }
    }
    for (int i = 0; i < H1; i++) {
        if (h1_pre[i] <= 0) d_h1[i] = 0;
    }

    for (int j = 0; j < H1; j++) {
        int row = j * INPUT;
        int32_t g = d_h1[j];
        m.b1[j] = clamp_q(qsub(m.b1[j], qmul(lr, g)));
        for (int i = 0; i < INPUT; i++) {
            int32_t dw = qmul(g, x[i]);
            m.w1[row + i] = clamp_q(qsub(m.w1[row + i], qmul(lr, dw)));
        }
    }

    return loss;
}

__device__ int32_t predict(MlpDev m, const int32_t* x) {
    int32_t h1[H1];
    for (int j = 0; j < H1; j++) {
        int32_t s = m.b1[j];
        int row = j * INPUT;
        for (int i = 0; i < INPUT; i++) s = qadd(s, qmul(m.w1[row + i], x[i]));
        h1[j] = qrelu(s);
    }
    int32_t h2[H2];
    for (int j = 0; j < H2; j++) {
        int32_t s = m.b2[j];
        int row = j * H1;
        for (int i = 0; i < H1; i++) s = qadd(s, qmul(m.w2[row + i], h1[i]));
        h2[j] = qrelu(s);
    }
    int best = 0;
    int32_t best_v = (int32_t)0x80000000;
    for (int c = 0; c < OUT; c++) {
        int32_t s = m.b3[c];
        int row = c * H2;
        for (int j = 0; j < H2; j++) s = qadd(s, qmul(m.w3[row + j], h2[j]));
        if (s > best_v) {
            best_v = s;
            best = c;
        }
    }
    return best;
}

__global__ void run_job_k(
    int32_t* weights,
    const uint8_t* mnist,
    uint32_t steps,
    int32_t lr,
    uint32_t samples,
    uint32_t offset,
    int32_t* loss_out,
    int32_t* acc_out,
    int32_t* scratch_x // INPUT * microbatch workspace (unused for seq; keeps VRAM warm)
) {
    if (threadIdx.x != 0 || blockIdx.x != 0) return;
    (void)scratch_x;

    MlpDev m;
    m.w1 = weights;
    m.b1 = weights + N_W1;
    m.w2 = m.b1 + N_B1;
    m.b2 = m.w2 + N_W2;
    m.w3 = m.b2 + N_B2;
    m.b3 = m.w3 + N_W3;

    int32_t last_loss = 0;
    int32_t x[INPUT];
    for (uint32_t step = 0; step < steps; step++) {
        uint32_t idx = offset + (step % samples);
        const uint8_t* base = mnist + 18 + idx * (1 + INPUT);
        uint8_t y = base[0];
        for (int i = 0; i < INPUT; i++) {
            int64_t p = (int64_t)base[1 + i];
            x[i] = (int32_t)((p * (int64_t)ONE + 127) / 255);
        }
        last_loss = train_step_seq(m, x, y, lr);
    }

    uint32_t eval_n = samples < 64 ? samples : 64;
    int32_t correct = 0;
    for (uint32_t i = 0; i < eval_n; i++) {
        uint32_t idx = offset + i;
        const uint8_t* base = mnist + 18 + idx * (1 + INPUT);
        uint8_t y = base[0];
        for (int j = 0; j < INPUT; j++) {
            int64_t p = (int64_t)base[1 + j];
            x[j] = (int32_t)((p * (int64_t)ONE + 127) / 255);
        }
        if (predict(m, x) == (int)y) correct++;
    }
    *loss_out = last_loss;
    *acc_out = (int32_t)(((int64_t)correct * (int64_t)ONE) / (int64_t)eval_n);
}

extern "C" int mesh_brain_v2_cuda_available(void) {
    int n = 0;
    if (cudaGetDeviceCount(&n) != cudaSuccess) return 0;
    return n > 0 ? 1 : 0;
}

extern "C" int mesh_brain_v2_run_job(
    int32_t* weights_q16,
    uint32_t n_weights,
    const uint8_t* mnist,
    uint32_t mnist_len,
    uint32_t steps,
    uint32_t lr_milli,
    uint32_t samples,
    uint32_t offset,
    uint64_t workspace_bytes,
    int32_t* loss_q16_out,
    int32_t* acc_q16_out
) {
    if (!weights_q16 || !mnist || !loss_q16_out || !acc_q16_out) return -1;
    if (n_weights != (uint32_t)N_WEIGHTS) return -2;
    if (mnist_len < 18 + 4096u * (1 + INPUT)) return -3;

    // lr = (milli * ONE + 500) / 1000
    int32_t lr = (int32_t)((((int64_t)lr_milli * (int64_t)ONE) + 500) / 1000);

    size_t wbytes = (size_t)N_WEIGHTS * sizeof(int32_t);
    size_t scratch = workspace_bytes ? (size_t)workspace_bytes : (size_t)(64ull << 20);
    // Cap scratch so we don't OOM tiny cards; floor for activations.
    if (scratch < (size_t)(4ull << 20)) scratch = (size_t)(4ull << 20);
    if (scratch > (size_t)(512ull << 20)) scratch = (size_t)(512ull << 20);

    int32_t *d_w = nullptr, *d_loss = nullptr, *d_acc = nullptr, *d_scratch = nullptr;
    uint8_t* d_mnist = nullptr;
    int rc = -10;

    if (cudaMalloc(&d_w, wbytes) != cudaSuccess) goto done;
    if (cudaMalloc(&d_mnist, mnist_len) != cudaSuccess) goto done;
    if (cudaMalloc(&d_loss, sizeof(int32_t)) != cudaSuccess) goto done;
    if (cudaMalloc(&d_acc, sizeof(int32_t)) != cudaSuccess) goto done;
    if (cudaMalloc(&d_scratch, scratch) != cudaSuccess) goto done;

    if (cudaMemcpy(d_w, weights_q16, wbytes, cudaMemcpyHostToDevice) != cudaSuccess) goto done;
    if (cudaMemcpy(d_mnist, mnist, mnist_len, cudaMemcpyHostToDevice) != cudaSuccess) goto done;
    cudaMemset(d_scratch, 0, scratch);

    run_job_k<<<1, 1>>>(d_w, d_mnist, steps, lr, samples, offset, d_loss, d_acc, d_scratch);
    if (cudaDeviceSynchronize() != cudaSuccess) goto done;

    if (cudaMemcpy(weights_q16, d_w, wbytes, cudaMemcpyDeviceToHost) != cudaSuccess) goto done;
    if (cudaMemcpy(loss_q16_out, d_loss, sizeof(int32_t), cudaMemcpyDeviceToHost) != cudaSuccess)
        goto done;
    if (cudaMemcpy(acc_q16_out, d_acc, sizeof(int32_t), cudaMemcpyDeviceToHost) != cudaSuccess)
        goto done;
    rc = 0;

done:
    if (d_w) cudaFree(d_w);
    if (d_mnist) cudaFree(d_mnist);
    if (d_loss) cudaFree(d_loss);
    if (d_acc) cudaFree(d_acc);
    if (d_scratch) cudaFree(d_scratch);
    return rc;
}
