// SPDX-License-Identifier: GPL-2.0-only

//! The prepare queue (`RENDER-32-D`, `docs/terrain.md` "Streaming core"): which chunk products
//! are being prepared on the background pool and which request of each chunk is live.
//!
//! A layer starts a job for a chunk with [`PrepareQueue::start`], which hands out a serial and
//! refuses while [`PrepareQueue::max_in_flight`] jobs are out. A job stays in flight from its
//! start until its result is delivered or dropped ([`PrepareQueue::finish`]), so the results
//! waiting for the renderer's poll are bounded with the jobs. A newer start for a chunk, a
//! cancel or a clear makes the older request stale: its result is dropped when it lands, never
//! delivered. The order jobs complete in decides only when a product is delivered, never which
//! product a chunk gets. Every operation is `O(1)`.

use std::collections::HashMap;
use std::hash::Hash;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// Chunks being prepared: the live request of each and the jobs in flight.
pub(crate) struct PrepareQueue<K> {
    live: Map<K, u64>,
    in_flight: usize,
    max_in_flight: usize,
    next_serial: u64,
}

impl<K: Copy + Eq + Hash> PrepareQueue<K> {
    /// A queue that lets at most `max_in_flight` jobs out at once.
    pub(crate) fn new(max_in_flight: usize) -> Self {
        Self {
            live: Map::default(),
            in_flight: 0,
            max_in_flight,
            next_serial: 1,
        }
    }

    /// Starts a job for `key`, superseding its live request if any. `None` while the in-flight
    /// bound is reached; the caller asks again later.
    pub(crate) fn start(&mut self, key: K) -> Option<u64> {
        if self.in_flight >= self.max_in_flight {
            return None;
        }
        let serial = self.next_serial;
        self.next_serial += 1;
        self.in_flight += 1;
        self.live.insert(key, serial);
        Some(serial)
    }

    /// A job's result leaves: returns whether it is `key`'s live request (deliver it; the key
    /// then has none) or stale (drop it).
    pub(crate) fn finish(&mut self, key: K, serial: u64) -> bool {
        self.in_flight = self.in_flight.saturating_sub(1);
        if self.live.get(&key) == Some(&serial) {
            self.live.remove(&key);
            true
        } else {
            false
        }
    }

    /// Makes `key`'s live request stale.
    pub(crate) fn cancel(&mut self, key: K) {
        self.live.remove(&key);
    }

    /// Makes every live request stale (a renderer reset or a world load); jobs in flight run
    /// on and their results are dropped.
    pub(crate) fn clear(&mut self) {
        self.live.clear();
    }

    /// Jobs started and not yet finished.
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// The in-flight bound.
    #[cfg(test)]
    pub(crate) fn max_in_flight(&self) -> usize {
        self.max_in_flight
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_supersede_cancel_and_clear() {
        let mut queue = PrepareQueue::new(2);
        let a = queue.start(1).unwrap();
        let b = queue.start(1).unwrap();
        assert_eq!(queue.start(2), None, "the bound refuses a third job");
        assert!(!queue.finish(1, a), "a superseded request is stale");
        assert!(queue.finish(1, b), "the newest request is live");
        assert!(!queue.finish(1, b), "a delivered request is not live twice");
        assert_eq!(queue.in_flight(), 0);
        let c = queue.start(3).unwrap();
        queue.cancel(3);
        assert!(!queue.finish(3, c), "a cancelled request is stale");
        let d = queue.start(4).unwrap();
        queue.clear();
        assert!(!queue.finish(4, d), "a cleared request is stale");
        assert_eq!(queue.max_in_flight(), 2);
    }
}
