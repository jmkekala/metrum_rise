// SPDX-License-Identifier: GPL-2.0-only

//! Reusable exact visit labels for employment and shopping candidate scans.

/// Worker-local exact membership labels shared by household candidate searches.
/// Cold storage is O(entries); warm reset and membership checks allocate nothing.
pub(super) struct CandidateVisits {
    seen_entries: Vec<u32>,
    seen_epoch: u32,
}

impl CandidateVisits {
    /// Borrows this worker's scratch for one non-nested candidate query.
    pub(super) fn with<T>(query: impl FnOnce(&mut Self) -> T) -> T {
        thread_local! {
            // Rayon may create many tasks per worker. Keep visited labels with the worker,
            // advancing their query epoch instead of allocating for each task partition.
            static CANDIDATE_VISITS: std::cell::RefCell<CandidateVisits> =
                std::cell::RefCell::new(CandidateVisits::new());
        }
        CANDIDATE_VISITS.with_borrow_mut(query)
    }

    fn new() -> Self {
        Self {
            seen_entries: Vec::new(),
            seen_epoch: 0,
        }
    }

    /// Starts an independent entry numbering, growing only when the largest index grows.
    pub(super) fn begin_query(&mut self, entry_count: usize) {
        if self.seen_entries.len() < entry_count {
            self.seen_entries.resize(entry_count, 0);
        }
        self.seen_epoch = self.seen_epoch.wrapping_add(1);
        if self.seen_epoch == 0 {
            self.seen_entries.fill(0);
            self.seen_epoch = 1;
        }
    }

    /// Checks membership without consuming a scan-limit slot.
    pub(super) fn contains(&self, entry_idx: usize) -> bool {
        self.seen_entries.get(entry_idx).is_some_and(|&epoch| epoch == self.seen_epoch)
    }

    /// Marks an in-range entry and returns whether this is its first visit.
    pub(super) fn mark_seen(&mut self, entry_idx: usize) -> bool {
        if entry_idx >= self.seen_entries.len() {
            return false;
        }
        if self.seen_entries[entry_idx] == self.seen_epoch {
            return false;
        }
        self.seen_entries[entry_idx] = self.seen_epoch;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_multimode_visits_match_original_linear_duplicate_checks() {
        let mut visits = CandidateVisits::new();
        for entries in [0, 1, 24, 127, 599] {
            let streams: [Vec<usize>; 2] = std::array::from_fn(|mode|
                (0..entries * 3).map(|i| (i * 37 + mode * 17) % entries).collect());
            for limit in [0, 1, 24, 384] {
                let mut expected = Vec::new();
                for stream in &streams {
                    for &entry in stream {
                        if expected.contains(&entry) { continue; }
                        if expected.len() == limit { break; }
                        expected.push(entry);
                    }
                }
                visits.begin_query(entries);
                let mut actual = Vec::new();
                for stream in &streams {
                    for &entry in stream {
                        if visits.contains(entry) { continue; }
                        if actual.len() == limit { break; }
                        assert!(visits.mark_seen(entry));
                        actual.push(entry);
                    }
                }
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    #[ignore = "matched release timing of bounded household candidate deduplication"]
    fn benchmark_bounded_candidate_visits() {
        use std::{hint::black_box, time::Instant};
        let mut visits = CandidateVisits::new();
        let mut seen = Vec::with_capacity(384);
        visits.begin_query(384);
        let mut samples = [Vec::new(), Vec::new()];
        for iteration in 0..44 {
            for candidate in [iteration % 2, 1 - iteration % 2] {
                let start = Instant::now();
                let mut accepted = 0;
                for _ in 0..128 {
                    if candidate == 1 { visits.begin_query(384); } else { seen.clear(); }
                    for index in 0..768 {
                        let entry = black_box(index % 384);
                        let fresh = if candidate == 1 {
                            if visits.contains(entry) { false } else { visits.mark_seen(entry) }
                        } else if seen.contains(&entry) { false } else { seen.push(entry); true };
                        accepted += usize::from(fresh);
                    }
                }
                assert_eq!(accepted, 128 * 384);
                samples[candidate].push(start.elapsed().as_secs_f64() * 1e6 / 128.0);
            }
        }
        eprintln!("CANDIDATE_VISITS_BENCH {}", serde_json::json!({
            "linear_us": samples[0], "labels_us": samples[1], "accepted_equal": true }));
    }

    #[test]
    fn candidate_scan_reuses_labels_between_tasks_and_after_epoch_wrap() {
        let capacity = CandidateVisits::with(|scratch| {
            scratch.begin_query(32);
            assert!(scratch.mark_seen(7));
            assert!(!scratch.mark_seen(7));
            scratch.seen_entries.capacity()
        });
        for _ in 0..16 {
            CandidateVisits::with(|scratch| {
                scratch.begin_query(32);
                assert!(scratch.mark_seen(7));
                assert!(!scratch.mark_seen(7));
                assert_eq!(scratch.seen_entries.capacity(), capacity);
            });
        }
        CandidateVisits::with(|scratch| {
            scratch.seen_epoch = u32::MAX;
            scratch.begin_query(32);
            assert_eq!(scratch.seen_epoch, 1);
            assert!(scratch.mark_seen(7));
            assert!(!scratch.mark_seen(7));
            assert_eq!(scratch.seen_entries.capacity(), capacity);
        });
    }

}
