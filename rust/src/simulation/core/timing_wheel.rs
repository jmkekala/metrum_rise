// SPDX-License-Identifier: GPL-2.0-only

//! Timed events keyed by absolute slot (`SIM-02`, `docs/simulation.md#deadlines-and-timed-events`).
//!
//! Two levels: a ring with one bucket per slot covering the next [`NEAR_SLOTS`] slots, and an
//! ordered overflow map for events further ahead, which move into the ring as it advances.
//! Insert and pop are O(1) amortized; an overflow insert costs O(log F) for F distant slots.
//! Events of one slot fire in insertion order: every event carries a sequence number from one
//! counter, which is saved with the wheel.

use std::collections::BTreeMap;

/// Slots covered by the ring: one game day at 16 slots per minute.
pub(crate) const NEAR_SLOTS: u64 = 24 * 60 * 16;

/// One scheduled event with its due slot and insertion sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimedEvent<E> {
    pub(crate) slot: u64,
    pub(crate) seq: u64,
    pub(crate) event: E,
}

/// Events due at absolute slots at or after [`TimingWheel::next_slot`].
#[derive(Clone, Debug)]
pub(crate) struct TimingWheel<E> {
    // `ring[slot % NEAR_SLOTS]` holds events for slots in `next_slot..next_slot + NEAR_SLOTS`.
    ring: Vec<Vec<TimedEvent<E>>>,
    far: BTreeMap<u64, Vec<TimedEvent<E>>>,
    // First slot not yet popped.
    next_slot: u64,
    next_seq: u64,
    len: usize,
}

impl<E: Copy> TimingWheel<E> {
    /// An empty wheel whose first poppable slot is `next_slot`.
    pub(crate) fn new(next_slot: u64) -> Self {
        Self {
            ring: (0..NEAR_SLOTS).map(|_| Vec::new()).collect(),
            far: BTreeMap::new(),
            next_slot,
            next_seq: 0,
            len: 0,
        }
    }

    /// Scheduled events not yet popped.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// First slot [`Self::pop_slot`] has not yet emptied.
    pub(crate) fn next_slot(&self) -> u64 {
        self.next_slot
    }

    /// Next insertion sequence number, saved with the wheel.
    pub(crate) fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Schedules `event` at `slot`; a slot already popped fires at the next popped slot.
    pub(crate) fn schedule(&mut self, slot: u64, event: E) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.insert(TimedEvent {
            slot: slot.max(self.next_slot),
            seq,
            event,
        });
    }

    fn insert(&mut self, timed: TimedEvent<E>) {
        self.len += 1;
        if timed.slot < self.next_slot + NEAR_SLOTS {
            self.ring[(timed.slot % NEAR_SLOTS) as usize].push(timed);
        } else {
            self.far.entry(timed.slot).or_default().push(timed);
        }
    }

    /// Moves every event due at `slot` into `out` in insertion order and advances past it.
    /// Slots must be popped in ascending order; skipped slots are popped first.
    pub(crate) fn pop_slot(&mut self, slot: u64, out: &mut Vec<TimedEvent<E>>) {
        while self.next_slot <= slot {
            let current = self.next_slot;
            let bucket = &mut self.ring[(current % NEAR_SLOTS) as usize];
            self.len -= bucket.len();
            out.append(bucket);
            self.next_slot += 1;
            // The slot entering the ring's far edge takes its overflow events.
            let entering = current + NEAR_SLOTS;
            if let Some(events) = self.far.remove(&entering) {
                self.ring[(entering % NEAR_SLOTS) as usize].extend(events);
            }
        }
    }

    /// Every pending event in firing order, for saving and inspection. O(N + near ring).
    pub(crate) fn pending(&self) -> Vec<TimedEvent<E>> {
        let mut events = Vec::with_capacity(self.len);
        for offset in 0..NEAR_SLOTS {
            let slot = self.next_slot + offset;
            events.extend(&self.ring[(slot % NEAR_SLOTS) as usize]);
        }
        for bucket in self.far.values() {
            events.extend(bucket);
        }
        events
    }

    /// Rebuilds a saved wheel; `events` must be in firing order with sequences below `next_seq`.
    pub(crate) fn restore(
        next_slot: u64,
        next_seq: u64,
        events: impl IntoIterator<Item = TimedEvent<E>>,
    ) -> Result<Self, String> {
        let mut wheel = Self::new(next_slot);
        wheel.next_seq = next_seq;
        let mut previous: Option<(u64, u64)> = None;
        for timed in events {
            if timed.slot < next_slot || timed.seq >= next_seq {
                return Err(format!(
                    "timed event at slot {} sequence {} is outside the saved wheel",
                    timed.slot, timed.seq
                ));
            }
            if previous.is_some_and(|key| key >= (timed.slot, timed.seq)) {
                return Err("timed events are not in firing order".into());
            }
            previous = Some((timed.slot, timed.seq));
            wheel.insert(timed);
        }
        Ok(wheel)
    }

    /// Keeps only the events `keep` accepts, in firing order. O(N + near ring); for rare
    /// wholesale changes such as undo, never per slot.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&E) -> bool) {
        let mut removed = 0;
        for bucket in self.ring.iter_mut().chain(self.far.values_mut()) {
            let before = bucket.len();
            bucket.retain(|timed| keep(&timed.event));
            removed += before - bucket.len();
        }
        self.far.retain(|_, bucket| !bucket.is_empty());
        self.len -= removed;
    }

    /// Removes every event, keeping the slot position and sequence counter.
    pub(crate) fn clear(&mut self) {
        for bucket in &mut self.ring {
            bucket.clear();
        }
        self.far.clear();
        self.len = 0;
    }

    /// Restarts an empty wheel at `next_slot`, for a world that is replaced wholesale.
    pub(crate) fn reset(&mut self, next_slot: u64) {
        self.clear();
        self.next_slot = next_slot;
        self.next_seq = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pop(wheel: &mut TimingWheel<u32>, slot: u64) -> Vec<(u64, u32)> {
        let mut out = Vec::new();
        wheel.pop_slot(slot, &mut out);
        out.into_iter().map(|e| (e.slot, e.event)).collect()
    }

    #[test]
    fn events_fire_at_their_slot_in_insertion_order_across_both_levels() {
        let mut wheel = TimingWheel::new(10);
        let far = 10 + NEAR_SLOTS * 2 + 5;
        wheel.schedule(far, 1);
        wheel.schedule(12, 2);
        wheel.schedule(12, 3);
        wheel.schedule(far, 4);
        wheel.schedule(3, 5); // already past: fires at the next popped slot
        assert_eq!(wheel.len(), 5);
        assert_eq!(pop(&mut wheel, 11), [(10, 5)]);
        assert_eq!(pop(&mut wheel, 12), [(12, 2), (12, 3)]);
        // The far events reach the ring as it advances and keep their order.
        wheel.schedule(far, 6);
        assert_eq!(pop(&mut wheel, far - 1), []);
        assert_eq!(pop(&mut wheel, far), [(far, 1), (far, 4), (far, 6)]);
        assert_eq!(wheel.len(), 0);
    }

    #[test]
    fn saved_wheels_restore_identically_and_reject_corruption() {
        let mut wheel = TimingWheel::new(100);
        for (slot, event) in [(150, 1), (100 + NEAR_SLOTS + 7, 2), (150, 3), (101, 4)] {
            wheel.schedule(slot, event);
        }
        let mut out = Vec::new();
        wheel.pop_slot(100, &mut out);
        let pending = wheel.pending();
        assert_eq!(
            pending.iter().map(|e| e.event).collect::<Vec<_>>(),
            [4, 1, 3, 2]
        );
        let mut restored =
            TimingWheel::restore(wheel.next_slot(), wheel.next_seq(), pending.clone()).unwrap();
        assert_eq!(restored.pending(), pending);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        wheel.pop_slot(100 + NEAR_SLOTS + 7, &mut a);
        restored.pop_slot(100 + NEAR_SLOTS + 7, &mut b);
        assert_eq!(a, b);
        let mut reversed = pending.clone();
        reversed.reverse();
        assert!(TimingWheel::restore(101, 4, reversed).is_err());
        assert!(TimingWheel::restore(151, 4, pending).is_err());
    }
}
