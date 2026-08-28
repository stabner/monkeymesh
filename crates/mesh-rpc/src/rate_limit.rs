//! Simple sliding-window rate limits for the embedded AI board.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct AiRateLimiter {
    inner: Mutex<HashMap<String, Window>>,
}

struct Window {
    start: Instant,
    count: u32,
}

impl Default for AiRateLimiter {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl AiRateLimiter {
    /// Returns Ok(()) if under limit, Err(retry_after_ms) if limited.
    pub fn check(&self, key: &str, limit: u32, window: Duration) -> Result<(), u64> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        // Opportunistic prune when map grows large.
        if map.len() > 8_000 {
            map.retain(|_, w| now.duration_since(w.start) < window.saturating_mul(2));
        }
        let entry = map.entry(key.to_string()).or_insert(Window {
            start: now,
            count: 0,
        });
        if now.duration_since(entry.start) >= window {
            entry.start = now;
            entry.count = 0;
        }
        if entry.count >= limit {
            let elapsed = now.duration_since(entry.start);
            let retry = window.saturating_sub(elapsed).as_millis() as u64;
            return Err(retry.max(50));
        }
        entry.count = entry.count.saturating_add(1);
        Ok(())
    }

    /// Check several keys; first failure wins (no partial consume on later keys).
    pub fn check_all(&self, checks: &[(&str, u32)], window: Duration) -> Result<(), u64> {
        for (key, limit) in checks {
            self.check(key, *limit, window)?;
        }
        Ok(())
    }
}
