// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic small-value selection helpers for agent lifecycle code.

pub(super) use crate::simulation::core::stable_hash64;

/// Returns a deterministic index in `0..len` for a stable seed.
#[inline(always)]
pub(super) fn stable_index(seed: u64, len: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    (stable_hash64(seed) as usize) % len
}

/// Returns a deterministic float in `[0, 1)` for a stable seed.
#[inline(always)]
pub(super) fn stable_unit_f32(seed: u64) -> f32 {
    const SCALE: f32 = 1.0 / ((1_u32 << 24) as f32);
    ((stable_hash64(seed) >> 40) as u32) as f32 * SCALE
}
