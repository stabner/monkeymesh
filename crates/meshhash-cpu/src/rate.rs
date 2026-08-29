//! Shared Fusion hashrate window + display.
//!
//! Count **finished Fusion hashes** (one nonce = one digest). GPU path fill+mix+seal
//! is one hash. Do not add CPU and GPU tiles when the engine mirrors the same rate.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

const HASH_WINDOW_SECS: f64 = 12.0;
const HASH_EMA_ALPHA: f64 = 0.20;
const HASH_REPORT_MS: u64 = 500;
/// First sample is `n / ~0` floored to 0.2s — a VRAM-sized wave looks like 100 kH/s.
/// Wait until wall time covers a real Fusion cycle (fill+mix) before publishing.
const MIN_SPAN_SECS: f64 = 3.0;

/// Rolling 12s window + EMA so a CUDA wave does not print a new H/s every batch.
#[derive(Debug)]
pub struct RateWindow {
    samples: VecDeque<(Instant, u64)>,
    ema: f64,
    last_send: Instant,
}

impl Default for RateWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl RateWindow {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            ema: 0.0,
            last_send: Instant::now() - Duration::from_secs(1),
        }
    }

    pub fn push(&mut self, n: u64) -> f64 {
        let now = Instant::now();
        if n > 0 {
            self.samples.push_back((now, n));
        }
        let cutoff = now - Duration::from_secs_f64(HASH_WINDOW_SECS);
        while self.samples.front().is_some_and(|(t, _)| *t < cutoff) {
            self.samples.pop_front();
        }
        let Some((t0, _)) = self.samples.front() else {
            return self.ema;
        };
        let span = now.duration_since(*t0).as_secs_f64();
        if span < MIN_SPAN_SECS {
            return self.ema;
        }
        let hashes: u64 = self.samples.iter().map(|(_, n)| *n).sum();
        let inst = hashes as f64 / span;
        self.ema = if self.ema <= 0.05 {
            inst
        } else {
            HASH_EMA_ALPHA * inst + (1.0 - HASH_EMA_ALPHA) * self.ema
        };
        self.ema
    }

    pub fn should_send(&mut self) -> bool {
        if self.last_send.elapsed() >= Duration::from_millis(HASH_REPORT_MS) {
            self.last_send = Instant::now();
            true
        } else {
            false
        }
    }
}

/// GPU Fusion path reports the same figure on both lanes. Never add those tiles.
pub fn hashrate_fusion(cpu_hs: f64, gpu_hs: f64) -> f64 {
    if gpu_hs > 0.0 {
        gpu_hs
    } else {
        cpu_hs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn first_sample_does_not_publish_inflated_rate() {
        let mut w = RateWindow::new();
        assert!(w.push(20_000) < 0.05, "single sample must not seed EMA");
    }

    #[test]
    fn window_uses_wall_time_not_batch_size() {
        let mut w = RateWindow::new();
        w.push(8);
        thread::sleep(Duration::from_millis(50));
        w.push(8);
        assert!(w.push(0) < 0.05, "still warming");
    }
}

pub fn format_hashrate(hs: f64) -> String {
    if hs >= 1_000_000.0 {
        format!("{:.2} MH/s", hs / 1_000_000.0)
    } else if hs >= 1_000.0 {
        format!("{:.2} kH/s", hs / 1_000.0)
    } else {
        format!("{:.1} H/s", hs)
    }
}
