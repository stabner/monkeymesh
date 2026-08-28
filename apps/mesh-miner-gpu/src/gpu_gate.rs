//! One card, two users: Fusion mix and CUDA brain train take turns.
//! PoW holds the card with a blocking lock. AI train waits for the card — it does not
//! fall back to CPU (miner CPU is for Fusion fill + the exam sidecar; the seed rematches).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

static GPU: Mutex<()> = Mutex::new(());
/// Set while a Fusion GPU worker is in its mine loop. AI must not take the card.
static POW_HOLDS: AtomicBool = AtomicBool::new(false);

pub fn set_pow_holds_gpu(on: bool) {
    POW_HOLDS.store(on, Ordering::Relaxed);
}

pub fn pow_holds_gpu() -> bool {
    POW_HOLDS.load(Ordering::Relaxed)
}

pub fn lock_gpu() -> MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

/// Non-blocking: `None` when Fusion mix already owns the card.
pub fn try_lock_gpu() -> Option<MutexGuard<'static, ()>> {
    match GPU.try_lock() {
        Ok(g) => Some(g),
        Err(TryLockError::WouldBlock) => None,
        Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
    }
}
