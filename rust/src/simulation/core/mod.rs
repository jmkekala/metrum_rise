// SPDX-License-Identifier: GPL-2.0-only

//! Core simulation primitives: time, slots, scheduling, entity handles, request queues, timed
//! events and weather.

pub(crate) mod change_log;
pub mod config;
pub(crate) mod handles;
pub(crate) mod phases;
pub(crate) mod pure_job;
pub(crate) mod request_queue;
pub(crate) mod scheduler;
pub(crate) mod sparse_chunk_grid;
pub mod time;
pub(crate) mod timing_wheel;
pub mod weather;

/// Mixes a stable integer seed into a well-distributed 64-bit value (SplitMix64 finalizer).
#[inline(always)]
pub(crate) fn stable_hash64(seed: u64) -> u64 {
    let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}
