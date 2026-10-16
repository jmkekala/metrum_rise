// SPDX-License-Identifier: GPL-2.0-only

//! Per-entity phases (`SIM-02`, `docs/simulation.md#phases`).
//!
//! Every handle has a daily phase in `0..PHASES_PER_DAY`, a stable hash of the handle, so an
//! entity keeps its phase for its whole life and a loaded game recomputes the same phases from
//! the saved handle tables. [`PhaseBuckets`] lists the live handles of each daily phase; an
//! hourly phase is the daily phase modulo [`PHASES_PER_HOUR`], so an hourly visit reads the 24
//! daily buckets that share it. Insert and remove are O(1).

use super::handles::Handle;
use super::stable_hash64;

/// Daily phases: one per slot of a game day at 16 slots per minute.
pub(crate) const PHASES_PER_DAY: u32 = 24 * 60 * 16;
/// Hourly phases: one per slot of a game hour.
pub(crate) const PHASES_PER_HOUR: u32 = 60 * 16;

// Marks a handle slot that holds no bucket entry.
const ABSENT: u32 = u32::MAX;

/// Stable daily phase of `handle`.
pub(crate) fn daily_phase<K>(handle: Handle<K>) -> u32 {
    let key = (u64::from(handle.index()) << 32) | u64::from(handle.generation());
    (stable_hash64(key) % u64::from(PHASES_PER_DAY)) as u32
}

/// Live handles of one entity kind, bucketed by daily phase.
pub(crate) struct PhaseBuckets<K> {
    buckets: Vec<Vec<Handle<K>>>,
    // Position of each handle slot's entry in its bucket.
    positions: Vec<u32>,
    len: usize,
}

impl<K> Clone for PhaseBuckets<K> {
    fn clone(&self) -> Self {
        Self {
            buckets: self.buckets.clone(),
            positions: self.positions.clone(),
            len: self.len,
        }
    }
}

impl<K> std::fmt::Debug for PhaseBuckets<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhaseBuckets")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

// The bucket array is allocated on the first insert, so empty stores stay cheap.
impl<K> Default for PhaseBuckets<K> {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            positions: Vec::new(),
            len: 0,
        }
    }
}

// The readers below serve `EntityPhase` systems from `SIM-02-F`.
#[cfg_attr(not(test), allow(dead_code))]
impl<K> PhaseBuckets<K> {
    /// Live handles.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Adds a newly allocated handle. O(1) amortized.
    pub(crate) fn insert(&mut self, handle: Handle<K>) {
        let slot = handle.index() as usize;
        if self.positions.len() <= slot {
            self.positions.resize(slot + 1, ABSENT);
        }
        debug_assert_eq!(self.positions[slot], ABSENT, "handle slot bucketed twice");
        if self.buckets.is_empty() {
            self.buckets = (0..PHASES_PER_DAY).map(|_| Vec::new()).collect();
        }
        let bucket = &mut self.buckets[daily_phase(handle) as usize];
        self.positions[slot] = bucket.len() as u32;
        bucket.push(handle);
        self.len += 1;
    }

    /// Drops a freed handle. O(1).
    pub(crate) fn remove(&mut self, handle: Handle<K>) {
        let slot = handle.index() as usize;
        let position = std::mem::replace(&mut self.positions[slot], ABSENT);
        debug_assert_ne!(position, ABSENT, "removing an unbucketed handle");
        let bucket = &mut self.buckets[daily_phase(handle) as usize];
        bucket.swap_remove(position as usize);
        if let Some(moved) = bucket.get(position as usize) {
            self.positions[moved.index() as usize] = position;
        }
        self.len -= 1;
    }

    /// Forgets every handle.
    pub(crate) fn clear(&mut self) {
        for bucket in &mut self.buckets {
            bucket.clear();
        }
        self.positions.clear();
        self.len = 0;
    }

    /// Handles whose daily phase is `phase`, in ascending handle order, written to `out`.
    pub(crate) fn daily(&self, phase: u32, out: &mut Vec<Handle<K>>) {
        out.clear();
        if let Some(bucket) = self.buckets.get(phase as usize) {
            out.extend_from_slice(bucket);
        }
        out.sort_unstable();
    }

    /// Handles whose hourly phase is `phase`, in ascending handle order, written to `out`.
    pub(crate) fn hourly(&self, phase: u32, out: &mut Vec<Handle<K>>) {
        out.clear();
        if self.buckets.is_empty() {
            return;
        }
        let mut daily = phase;
        while daily < PHASES_PER_DAY {
            out.extend_from_slice(&self.buckets[daily as usize]);
            daily += PHASES_PER_HOUR;
        }
        out.sort_unstable();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Thing;

    #[test]
    fn every_handle_is_visited_once_per_day_and_once_per_hour_in_handle_order() {
        let mut buckets = PhaseBuckets::<Thing>::default();
        let handles: Vec<_> = (0..500).map(|i| Handle::from_parts(i, i % 3)).collect();
        for &handle in &handles {
            buckets.insert(handle);
        }
        for &handle in handles.iter().step_by(7) {
            buckets.remove(handle);
        }
        let live: Vec<_> = handles
            .iter()
            .copied()
            .enumerate()
            .filter(|(i, _)| i % 7 != 0)
            .map(|(_, handle)| handle)
            .collect();
        assert_eq!(buckets.len(), live.len());
        let mut out = Vec::new();
        let mut daily = Vec::new();
        for phase in 0..PHASES_PER_DAY {
            buckets.daily(phase, &mut out);
            assert!(out.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(out.iter().all(|&handle| daily_phase(handle) == phase));
            daily.extend_from_slice(&out);
        }
        daily.sort_unstable();
        assert_eq!(daily, live);
        let mut hourly = Vec::new();
        for phase in 0..PHASES_PER_HOUR {
            buckets.hourly(phase, &mut out);
            assert!(out.windows(2).all(|pair| pair[0] < pair[1]));
            hourly.extend_from_slice(&out);
        }
        hourly.sort_unstable();
        assert_eq!(hourly, live);
    }
}
