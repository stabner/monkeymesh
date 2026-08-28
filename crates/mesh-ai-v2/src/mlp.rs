//! mlp512: 784 → 512 → 128 → 10 in Q16.16.

use crate::q16::{clamp_q, q_from_milli, qadd, qmul, qrelu, qsub, seed_q, ONE};

pub const INPUT: usize = 784;
pub const H1: usize = 512;
pub const H2: usize = 128;
pub const OUT: usize = 10;

pub const GENESIS_BRAIN_SEED: u64 = 0x4D45_5348_4252_5632; // "MESHBRV2"
pub const WEIGHTS_MAGIC: &[u8] = b"MESHBRAINv2";

pub const N_W1: usize = H1 * INPUT;
pub const N_B1: usize = H1;
pub const N_W2: usize = H2 * H1;
pub const N_B2: usize = H2;
pub const N_W3: usize = OUT * H2;
pub const N_B3: usize = OUT;
pub const N_WEIGHTS: usize = N_W1 + N_B1 + N_W2 + N_B2 + N_W3 + N_B3;
/// magic + u32 count + i32 weights
pub const WEIGHTS_BLOB_LEN: usize = WEIGHTS_MAGIC.len() + 4 + N_WEIGHTS * 4;

pub struct Mlp {
    pub w1: Vec<i32>,
    pub b1: Vec<i32>,
    pub w2: Vec<i32>,
    pub b2: Vec<i32>,
    pub w3: Vec<i32>,
    pub b3: Vec<i32>,
}

impl Mlp {
    pub fn genesis(seed: u64) -> Self {
        let scale1 = ONE / 64;
        let scale2 = ONE / 32;
        let scale3 = ONE / 16;
        let mut w1 = vec![0i32; N_W1];
        for (i, w) in w1.iter_mut().enumerate() {
            *w = seed_q(seed, 1, i as u64, scale1);
        }
        let mut b1 = vec![0i32; N_B1];
        for (i, b) in b1.iter_mut().enumerate() {
            *b = seed_q(seed, 2, i as u64, scale1 / 4);
        }
        let mut w2 = vec![0i32; N_W2];
        for (i, w) in w2.iter_mut().enumerate() {
            *w = seed_q(seed, 3, i as u64, scale2);
        }
        let mut b2 = vec![0i32; N_B2];
        for (i, b) in b2.iter_mut().enumerate() {
            *b = seed_q(seed, 4, i as u64, scale2 / 4);
        }
        let mut w3 = vec![0i32; N_W3];
        for (i, w) in w3.iter_mut().enumerate() {
            *w = seed_q(seed, 5, i as u64, scale3);
        }
        let mut b3 = vec![0i32; N_B3];
        for (i, b) in b3.iter_mut().enumerate() {
            *b = seed_q(seed, 6, i as u64, scale3 / 4);
        }
        Self {
            w1,
            b1,
            w2,
            b2,
            w3,
            b3,
        }
    }

    pub fn from_weights(blob: &[u8]) -> Result<Self, ()> {
        if blob.len() < WEIGHTS_BLOB_LEN || &blob[..WEIGHTS_MAGIC.len()] != WEIGHTS_MAGIC {
            return Err(());
        }
        let mut o = WEIGHTS_MAGIC.len();
        let n = u32::from_le_bytes(blob[o..o + 4].try_into().unwrap()) as usize;
        o += 4;
        if n != N_WEIGHTS || blob.len() < o + n * 4 {
            return Err(());
        }
        let mut read = |len: usize| -> Vec<i32> {
            let mut v = Vec::with_capacity(len);
            for _ in 0..len {
                let bits = i32::from_le_bytes(blob[o..o + 4].try_into().unwrap());
                o += 4;
                v.push(bits);
            }
            v
        };
        Ok(Self {
            w1: read(N_W1),
            b1: read(N_B1),
            w2: read(N_W2),
            b2: read(N_B2),
            w3: read(N_W3),
            b3: read(N_B3),
        })
    }

    pub fn to_weights(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(WEIGHTS_BLOB_LEN);
        out.extend_from_slice(WEIGHTS_MAGIC);
        out.extend_from_slice(&(N_WEIGHTS as u32).to_le_bytes());
        for w in self
            .w1
            .iter()
            .chain(self.b1.iter())
            .chain(self.w2.iter())
            .chain(self.b2.iter())
            .chain(self.w3.iter())
            .chain(self.b3.iter())
        {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    /// One SGD step; returns MSE loss in Q16.
    pub fn train_step_v2(&mut self, x: &[i32; INPUT], y: u8, lr: i32) -> i32 {
        train_step_correct(self, x, y, lr)
    }

    pub fn eval_accuracy(&self, xs: &[[i32; INPUT]], ys: &[u8]) -> i32 {
        if xs.is_empty() {
            return 0;
        }
        let mut correct = 0i32;
        for (x, &y) in xs.iter().zip(ys.iter()) {
            let mut h1 = [0i32; H1];
            for j in 0..H1 {
                let mut s = self.b1[j];
                let row = j * INPUT;
                for i in 0..INPUT {
                    s = qadd(s, qmul(self.w1[row + i], x[i]));
                }
                h1[j] = qrelu(s);
            }
            let mut h2 = [0i32; H2];
            for j in 0..H2 {
                let mut s = self.b2[j];
                let row = j * H1;
                for i in 0..H1 {
                    s = qadd(s, qmul(self.w2[row + i], h1[i]));
                }
                h2[j] = qrelu(s);
            }
            let mut best = 0usize;
            let mut best_v = i32::MIN;
            for c in 0..OUT {
                let mut s = self.b3[c];
                let row = c * H2;
                for j in 0..H2 {
                    s = qadd(s, qmul(self.w3[row + j], h2[j]));
                }
                if s > best_v {
                    best_v = s;
                    best = c;
                }
            }
            if best == y as usize {
                correct += 1;
            }
        }
        ((correct as i64 * ONE as i64) / xs.len() as i64) as i32
    }
}

fn train_step_correct(m: &mut Mlp, x: &[i32; INPUT], y: u8, lr: i32) -> i32 {
    let mut h1_pre = [0i32; H1];
    let mut h1 = [0i32; H1];
    for j in 0..H1 {
        let mut s = m.b1[j];
        let row = j * INPUT;
        for i in 0..INPUT {
            s = qadd(s, qmul(m.w1[row + i], x[i]));
        }
        h1_pre[j] = s;
        h1[j] = qrelu(s);
    }
    let mut h2_pre = [0i32; H2];
    let mut h2 = [0i32; H2];
    for j in 0..H2 {
        let mut s = m.b2[j];
        let row = j * H1;
        for i in 0..H1 {
            s = qadd(s, qmul(m.w2[row + i], h1[i]));
        }
        h2_pre[j] = s;
        h2[j] = qrelu(s);
    }
    let mut logits = [0i32; OUT];
    for c in 0..OUT {
        let mut s = m.b3[c];
        let row = c * H2;
        for j in 0..H2 {
            s = qadd(s, qmul(m.w3[row + j], h2[j]));
        }
        logits[c] = clamp_q(s);
    }

    let mut loss = 0i32;
    let mut d_logits = [0i32; OUT];
    for c in 0..OUT {
        let target = if c == y as usize { ONE } else { 0 };
        let err = qsub(logits[c], target);
        loss = qadd(loss, qmul(err, err));
        d_logits[c] = err;
    }

    let mut d_h2 = [0i32; H2];
    let mut dw3 = vec![0i32; N_W3];
    let mut db3 = [0i32; OUT];
    for c in 0..OUT {
        let row = c * H2;
        let g = d_logits[c];
        db3[c] = g;
        for j in 0..H2 {
            dw3[row + j] = qmul(g, h2[j]);
            d_h2[j] = qadd(d_h2[j], qmul(g, m.w3[row + j]));
        }
    }
    for j in 0..H2 {
        if h2_pre[j] <= 0 {
            d_h2[j] = 0;
        }
    }

    let mut d_h1 = [0i32; H1];
    let mut dw2 = vec![0i32; N_W2];
    let mut db2 = [0i32; H2];
    for j in 0..H2 {
        let row = j * H1;
        let g = d_h2[j];
        db2[j] = g;
        for i in 0..H1 {
            dw2[row + i] = qmul(g, h1[i]);
            d_h1[i] = qadd(d_h1[i], qmul(g, m.w2[row + i]));
        }
    }
    for i in 0..H1 {
        if h1_pre[i] <= 0 {
            d_h1[i] = 0;
        }
    }

    let mut dw1 = vec![0i32; N_W1];
    let mut db1 = [0i32; H1];
    for j in 0..H1 {
        let row = j * INPUT;
        let g = d_h1[j];
        db1[j] = g;
        for i in 0..INPUT {
            dw1[row + i] = qmul(g, x[i]);
        }
    }

    for i in 0..N_W1 {
        m.w1[i] = clamp_q(qsub(m.w1[i], qmul(lr, dw1[i])));
    }
    for i in 0..H1 {
        m.b1[i] = clamp_q(qsub(m.b1[i], qmul(lr, db1[i])));
    }
    for i in 0..N_W2 {
        m.w2[i] = clamp_q(qsub(m.w2[i], qmul(lr, dw2[i])));
    }
    for i in 0..H2 {
        m.b2[i] = clamp_q(qsub(m.b2[i], qmul(lr, db2[i])));
    }
    for i in 0..N_W3 {
        m.w3[i] = clamp_q(qsub(m.w3[i], qmul(lr, dw3[i])));
    }
    for i in 0..OUT {
        m.b3[i] = clamp_q(qsub(m.b3[i], qmul(lr, db3[i])));
    }

    loss
}

pub fn genesis_weights(seed: u64) -> Vec<u8> {
    Mlp::genesis(seed).to_weights()
}

pub fn weights_digest(weights: &[u8]) -> [u8; 32] {
    *blake3::hash(weights).as_bytes()
}

pub fn sample_to_q(pixels: &[u8]) -> [i32; INPUT] {
    let mut x = [0i32; INPUT];
    for i in 0..INPUT {
        let p = pixels[i] as i64;
        x[i] = ((p * ONE as i64 + 127) / 255) as i32;
    }
    x
}

pub fn lr_from_milli(lr_milli: u32) -> i32 {
    q_from_milli(lr_milli)
}
