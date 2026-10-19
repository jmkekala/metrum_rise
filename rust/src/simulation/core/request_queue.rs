// SPDX-License-Identifier: GPL-2.0-only

//! Per-slot request queues of entity handles (`SIM-02-J`,
//! `docs/simulation.md#request-queues`).
//!
//! An entity queues `(slot, handle)` when its phase or deadline comes up, and the matcher serves
//! the oldest request first. Phases visit their handles in ascending order, so the requests of
//! one slot already stand in handle order. An entity is queued at most once: a mark per handle
//! slot, holding the queued handle's generation, is set by a push and cleared by the pop that
//! serves it. The marks are derived from the queue, so a loaded queue rebuilds them and saves
//! hold only the requests, in order.

use std::collections::VecDeque;
use std::fmt;

use super::handles::Handle;

// Marks a handle slot with no queued request. Generations never reach it: a slot would have to
// be freed four billion times.
const NOT_QUEUED: u32 = u32::MAX;

/// Requests of entities of kind `K`, oldest first, at most one per entity.
pub(crate) struct RequestQueue<K> {
    requests: VecDeque<(u64, Handle<K>)>,
    // Generation of the handle queued from each handle slot, or `NOT_QUEUED`.
    queued: Vec<u32>,
}

// Manual impls: derives would require the marker type itself to implement each trait.
impl<K> Clone for RequestQueue<K> {
    fn clone(&self) -> Self {
        Self {
            requests: self.requests.clone(),
            queued: self.queued.clone(),
        }
    }
}

impl<K> Default for RequestQueue<K> {
    fn default() -> Self {
        Self {
            requests: VecDeque::new(),
            queued: Vec::new(),
        }
    }
}

impl<K> fmt::Debug for RequestQueue<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.requests.iter()).finish()
    }
}

impl<K> PartialEq for RequestQueue<K> {
    fn eq(&self, other: &Self) -> bool {
        self.requests == other.requests
    }
}

impl<K> RequestQueue<K> {
    /// Rebuilds a saved queue from its requests in order; a repeated entity keeps its first
    /// request. O(requests).
    pub(crate) fn restore(requests: impl IntoIterator<Item = (u64, Handle<K>)>) -> Self {
        let mut queue = Self::default();
        for (slot, handle) in requests {
            queue.push(slot, handle);
        }
        queue
    }

    /// Queues `handle`'s request from `slot` unless it already waits. Returns whether it was
    /// queued. O(1) amortized.
    pub(crate) fn push(&mut self, slot: u64, handle: Handle<K>) -> bool {
        if handle.is_none() || self.is_queued(handle) {
            return false;
        }
        let index = handle.index() as usize;
        if self.queued.len() <= index {
            self.queued.resize(index + 1, NOT_QUEUED);
        }
        self.queued[index] = handle.generation();
        self.requests.push_back((slot, handle));
        true
    }

    /// Takes the oldest request: its slot and entity. O(1). A request of an entity that left
    /// can wait before a request of the entity that reused its handle slot; taking it leaves
    /// the newer entity's mark in place.
    pub(crate) fn pop(&mut self) -> Option<(u64, Handle<K>)> {
        let (slot, handle) = self.requests.pop_front()?;
        let mark = &mut self.queued[handle.index() as usize];
        if *mark == handle.generation() {
            *mark = NOT_QUEUED;
        }
        Some((slot, handle))
    }

    /// The oldest request, without taking it.
    pub(crate) fn front(&self) -> Option<(u64, Handle<K>)> {
        self.requests.front().copied()
    }

    /// Whether every mark is the one the queue's requests give, as `restore` rebuilds them.
    #[cfg(test)]
    pub(crate) fn marks_follow_queue(&self) -> bool {
        let rebuilt = Self::restore(self.iter());
        let used = |marks: &[u32]| {
            marks
                .iter()
                .rposition(|&mark| mark != NOT_QUEUED)
                .map_or(0, |i| i + 1)
        };
        self.queued[..used(&self.queued)] == rebuilt.queued[..used(&rebuilt.queued)]
    }

    /// Whether `handle` has a request waiting. O(1).
    pub(crate) fn is_queued(&self, handle: Handle<K>) -> bool {
        self.queued.get(handle.index() as usize) == Some(&handle.generation())
    }

    /// Requests waiting.
    pub(crate) fn len(&self) -> usize {
        self.requests.len()
    }

    /// Whether no request waits.
    pub(crate) fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// The waiting requests, oldest first.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (u64, Handle<K>)> + '_ {
        self.requests.iter().copied()
    }

    /// Drops every request, keeping the allocations.
    pub(crate) fn clear(&mut self) {
        self.requests.clear();
        self.queued.fill(NOT_QUEUED);
    }
}

/// Service of one queue over an hour: requests taken, the most in one slot, the slots that
/// left requests waiting and the longest such backlog.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct QueueStats {
    pub(crate) taken: u32,
    busiest_slot: u32,
    carried_slots: u32,
    longest_backlog: u32,
}

impl QueueStats {
    /// Adds one slot that took `taken` requests and left `backlog` waiting.
    pub(crate) fn record_slot(&mut self, taken: usize, backlog: usize) {
        self.taken = self.taken.saturating_add(taken as u32);
        self.busiest_slot = self.busiest_slot.max(taken as u32);
        if backlog > 0 {
            self.carried_slots = self.carried_slots.saturating_add(1);
            self.longest_backlog = self.longest_backlog.max(backlog as u32);
        }
    }
}

impl fmt::Display for QueueStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "taken={} busiest_slot={} carried_slots={} longest_backlog={}",
            self.taken, self.busiest_slot, self.carried_slots, self.longest_backlog
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Thing;

    #[test]
    fn requests_leave_oldest_first_once_per_entity_and_restore_their_marks() {
        let mut queue = RequestQueue::<Thing>::default();
        let first = Handle::from_parts(3, 0);
        let second = Handle::from_parts(1, 2);
        assert!(queue.push(10, first));
        assert!(queue.push(10, second));
        assert!(!queue.push(11, first), "an entity waits once");
        assert!(
            queue.push(11, Handle::from_parts(3, 1)),
            "a reused slot's new entity is another request"
        );
        assert!(!queue.push(12, Handle::NONE));
        let restored = RequestQueue::restore(queue.iter());
        assert_eq!(restored, queue);
        assert!(restored.is_queued(second));
        assert_eq!(queue.pop(), Some((10, first)));
        assert!(queue.push(12, first), "a served entity can queue again");
        assert_eq!(
            queue.iter().collect::<Vec<_>>(),
            vec![(10, second), (11, Handle::from_parts(3, 1)), (12, first)]
        );
        queue.clear();
        assert!(queue.is_empty() && !queue.is_queued(second));
    }

    #[test]
    fn taking_a_left_entitys_request_keeps_the_mark_of_the_slots_new_entity() {
        let mut queue = RequestQueue::<Thing>::default();
        let (left, reused) = (Handle::from_parts(3, 0), Handle::from_parts(3, 1));
        assert!(queue.push(5, left));
        assert!(queue.push(6, reused));
        assert_eq!(queue.pop(), Some((5, left)));
        assert!(queue.is_queued(reused));
        assert!(!queue.push(7, reused), "the new entity still waits once");
        assert!(queue.marks_follow_queue());
        assert_eq!(queue.pop(), Some((6, reused)));
        assert!(!queue.is_queued(reused) && queue.marks_follow_queue());
    }
}
