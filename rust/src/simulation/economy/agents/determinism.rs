// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic small-value selection helpers for agent lifecycle code.

/// Mixes a stable integer seed into a well-distributed 64-bit value.
#[inline(always)]
pub(super) fn stable_hash64(seed: u64) -> u64 {
    let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

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
