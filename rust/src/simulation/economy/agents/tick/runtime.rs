// SPDX-License-Identifier: GPL-2.0-only

//! Runtime dispatch helpers for the agent tick.

use rayon::prelude::*;

use super::TripQueryScratch;

/// Agent count where the tick switches from sequential loops to Rayon.
///
/// Below this threshold Rayon's worker threads can spin-wait for roughly 1 ms
/// after each call looking for more work. At 60 Hz with multiple parallel
/// sections per tick, that idle spin can cost more than the work itself in
/// small test cities.
pub(super) const PAR_THRESHOLD: usize = 500;

/// Dispatches `f` over `0..n`, using Rayon only when the agent count is large enough.
pub(super) fn dispatch_agents<F: Fn(usize) + Send + Sync>(n: usize, f: F) {
    if n >= PAR_THRESHOLD {
        (0..n).into_par_iter().for_each(f);
    } else {
        (0..n).for_each(f);
    }
}

/// Dispatches fixed contiguous agent slices while assigning each slice an owned route scratch.
///
/// Rayon may schedule a slice on any worker, so the scratch is owned by the slice rather than
/// by a thread-local cache. The serial branch retains the existing small-population exception.
pub(super) fn dispatch_agents_with_owned_scratch<F: Fn(usize) + Send + Sync>(
    n: usize,
    shards: &mut [TripQueryScratch],
    serial: &mut TripQueryScratch,
    f: F,
) {
    if n < PAR_THRESHOLD {
        TripQueryScratch::with_owned(serial, || (0..n).for_each(f));
        return;
    }

    let shard_count = shards.len().min(n).max(1);
    let chunk = n.div_ceil(shard_count);
    shards[..shard_count]
        .par_iter_mut()
        .enumerate()
        .for_each(|(shard_idx, scratch)| {
            let start = shard_idx * chunk;
            let end = (start + chunk).min(n);
            if start < end {
                TripQueryScratch::with_owned(scratch, || (start..end).for_each(&f));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn owned_query_dispatch_visits_each_index_once_at_partition_boundaries() {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(3).build().unwrap();
        pool.install(|| {
            let mut shards: Vec<_> = (0..3).map(|_| TripQueryScratch::default()).collect();
            let mut serial = TripQueryScratch::default();
            for n in [0, 1, 499, 500, 501, 1003] {
                let visits: Vec<_> = (0..n).map(|_| AtomicUsize::new(0)).collect();
                dispatch_agents_with_owned_scratch(n, &mut shards, &mut serial, |index| {
                    visits[index].fetch_add(1, Ordering::Relaxed);
                });
                assert!(visits.iter().all(|count| count.load(Ordering::Relaxed) == 1));
            }
        });
    }
}
