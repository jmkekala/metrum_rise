// SPDX-License-Identifier: GPL-2.0-only

//! Revision-stamped storage-index change logs for incrementally maintained derived tables.

/// Indices changed at each revision of an owning store. A consumer built at revision `r`
/// may update only the logged indices while the log is complete since `r`; otherwise it
/// rebuilds. Changes that cannot name their indices clear the log. Storage is bounded:
/// a full log is cleared, so lagging consumers rebuild once. Record and query are
/// O(1) amortized and O(log entries + changes) respectively.
#[derive(Clone, Debug, Default)]
pub(crate) struct IndexChangeLog {
    // Every revision after `floor` is recorded below, in ascending revision order.
    floor: u64,
    entries: Vec<(u64, usize)>,
}

// Bounds retained history; consumers more than this many changes behind rebuild instead.
const MAX_ENTRIES: usize = 1 << 16;

impl IndexChangeLog {
    /// Reserves bounded recording space outside mutation loops, without discarding history.
    pub(crate) fn prepare_recording(&mut self, entries: usize) {
        let target = entries.min(MAX_ENTRIES);
        if self.entries.capacity() < target {
            self.entries
                .reserve(target.saturating_sub(self.entries.len()));
        }
    }

    /// Records one row without allocation. Exhausted/unprepared storage marks the revision
    /// incomplete, so consumers rebuild instead of accepting a partial change set.
    pub(crate) fn record_prepared(&mut self, revision: u64, index: usize) {
        if self.entries.len() >= self.entries.capacity().min(MAX_ENTRIES) {
            self.clear(revision);
            return;
        }
        self.entries.push((revision, index));
    }

    /// Records the indices changed by the owner's step to `revision`.
    pub(crate) fn record(&mut self, revision: u64, indices: impl IntoIterator<Item = usize>) {
        if self.entries.len() >= MAX_ENTRIES {
            self.clear(revision.wrapping_sub(1));
        }
        self.entries
            .extend(indices.into_iter().map(|index| (revision, index)));
    }

    /// Marks a change to `revision` whose affected indices are unknown.
    pub(crate) fn clear(&mut self, revision: u64) {
        self.floor = revision;
        self.entries.clear();
    }

    /// Indices changed after `since` up to the current revision, sorted and deduplicated
    /// into `out`. Returns false when the log cannot prove completeness since `since`.
    pub(crate) fn changes_since(&self, since: u64, current: u64, out: &mut Vec<usize>) -> bool {
        out.clear();
        if since == current {
            return true;
        }
        if since < self.floor || since > current {
            return false;
        }
        let first = self
            .entries
            .partition_point(|&(revision, _)| revision <= since);
        out.extend(self.entries[first..].iter().map(|&(_, index)| index));
        out.sort_unstable();
        out.dedup();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_recording_never_grows_and_overflow_requires_a_rebuild() {
        let mut log = IndexChangeLog::default();
        let mut out = Vec::new();
        log.record_prepared(1, 2);
        assert_eq!(log.entries.capacity(), 0);
        assert!(!log.changes_since(0, 1, &mut out));
        log.prepare_recording(8);
        let capacity = log.entries.capacity();
        for revision in 2..=capacity as u64 + 1 {
            log.record_prepared(revision, 3);
        }
        assert!(log.changes_since(1, capacity as u64 + 1, &mut out));
        assert_eq!(out, [3]);
        let overflow = capacity as u64 + 2;
        log.record_prepared(overflow, 4);
        assert_eq!(log.entries.capacity(), capacity);
        assert!(!log.changes_since(1, overflow, &mut out));
        log.record_prepared(overflow + 1, 5);
        assert!(log.changes_since(overflow, overflow + 1, &mut out));
        assert_eq!(out, [5]);
    }

    #[test]
    fn changes_follow_recorded_revisions_and_unknown_changes_force_rebuilds() {
        let mut log = IndexChangeLog::default();
        let mut out = Vec::new();
        assert!(log.changes_since(0, 0, &mut out) && out.is_empty());
        log.record(1, [4]);
        log.record(2, [7, 4]);
        assert!(log.changes_since(0, 2, &mut out));
        assert_eq!(out, [4, 7]);
        assert!(log.changes_since(1, 2, &mut out));
        assert_eq!(out, [4, 7]);
        assert!(log.changes_since(2, 2, &mut out) && out.is_empty());
        log.clear(3);
        assert!(!log.changes_since(2, 3, &mut out));
        assert!(log.changes_since(3, 3, &mut out) && out.is_empty());
        log.record(4, [1]);
        assert!(log.changes_since(3, 4, &mut out));
        assert_eq!(out, [1]);
        assert!(
            !log.changes_since(5, 4, &mut out),
            "a cursor ahead of the store is foreign"
        );
    }

    #[test]
    fn full_logs_are_bounded_and_lagging_consumers_rebuild() {
        let mut log = IndexChangeLog::default();
        let mut out = Vec::new();
        for revision in 1..=MAX_ENTRIES as u64 + 1 {
            log.record(revision, [revision as usize]);
        }
        assert!(log.entries.len() <= MAX_ENTRIES);
        assert!(!log.changes_since(0, MAX_ENTRIES as u64 + 1, &mut out));
        assert!(log.changes_since(MAX_ENTRIES as u64, MAX_ENTRIES as u64 + 1, &mut out));
        assert_eq!(out, [MAX_ENTRIES + 1]);
    }
}
