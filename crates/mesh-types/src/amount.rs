use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

use crate::DECIMALS;

/// Amount in atomic units (1 MESH = 10^8).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Amount(pub u64);

#[derive(Debug, Error)]
pub enum AmountError {
    #[error("amount overflow")]
    Overflow,
    #[error("insufficient funds")]
    Insufficient,
}

impl Amount {
    pub const ZERO: Self = Self(0);

    pub fn from_atomic(v: u64) -> Self {
        Self(v)
    }

    pub fn from_mesh(mesh: u64) -> Option<Self> {
        mesh.checked_mul(10u64.pow(DECIMALS)).map(Self)
    }

    /// Parse `"12"`, `"12.5"`, or `"12.50000000"` into atomic units.
    pub fn parse_mesh(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let (whole_s, frac_s) = match s.split_once('.') {
            Some((w, f)) => (w, f),
            None => (s, ""),
        };
        if frac_s.len() > DECIMALS as usize {
            return None;
        }
        let whole: u64 = if whole_s.is_empty() {
            0
        } else {
            whole_s.parse().ok()?
        };
        let mut frac_buf = frac_s.to_string();
        while frac_buf.len() < DECIMALS as usize {
            frac_buf.push('0');
        }
        let frac: u64 = if frac_buf.is_empty() {
            0
        } else {
            frac_buf.parse().ok()?
        };
        let atomic = whole.checked_mul(10u64.pow(DECIMALS))?.checked_add(frac)?;
        Some(Self(atomic))
    }

    pub fn atomic(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, AmountError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(AmountError::Overflow)
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, AmountError> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or(AmountError::Insufficient)
    }

    /// Split a block reward by market basis points (out of 10_000).
    pub fn split_bps(self, bps: u16) -> Self {
        Self(self.0.saturating_mul(bps as u64) / 10_000)
    }
}

impl fmt::Debug for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Amount({})", self.0)
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.0 / 10u64.pow(DECIMALS);
        let frac = self.0 % 10u64.pow(DECIMALS);
        write!(f, "{whole}.{frac:08} MESH")
    }
}
