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

/// Agents per chunk of the movement pass's interleaved shards.
const MOVEMENT_CHUNK: usize = 256;

/// Dispatches the agents in shards that each own a route scratch.
///
/// Rayon may schedule a shard on any worker, so the scratch is owned by the shard rather than
/// by a thread-local cache. A shard takes every `shards`-th chunk of [`MOVEMENT_CHUNK`] agents
/// (`SIM-02-K` part 4): moving agents cluster in index ranges, so contiguous shards ran at
/// 1.8× their mean, while interleaved ones each see the whole city's mix. Results never depend
/// on the layout: a scratch only memoizes route storage. The serial branch retains the
/// existing small-population exception.
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

    let chunk_count = n.div_ceil(MOVEMENT_CHUNK);
    let shard_count = shards.len().min(chunk_count).max(1);
    shards[..shard_count]
        .par_iter_mut()
        .enumerate()
        .for_each(|(shard_idx, scratch)| {
            TripQueryScratch::with_owned(scratch, || visit_shard(shard_idx, shard_count, n, &f));
        });
}

// Visits shard `shard_idx`'s chunks in index order.
fn visit_shard(shard_idx: usize, shard_count: usize, n: usize, f: &impl Fn(usize)) {
    for start in (shard_idx * MOVEMENT_CHUNK..n).step_by(shard_count * MOVEMENT_CHUNK) {
        (start..(start + MOVEMENT_CHUNK).min(n)).for_each(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn owned_query_dispatch_visits_each_index_once_at_partition_boundaries() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap();
        pool.install(|| {
            let mut shards: Vec<_> = (0..3).map(|_| TripQueryScratch::default()).collect();
            let mut serial = TripQueryScratch::default();
            for n in [0, 1, 499, 500, 501, 767, 768, 769, 1003] {
                let visits: Vec<_> = (0..n).map(|_| AtomicUsize::new(0)).collect();
                dispatch_agents_with_owned_scratch(n, &mut shards, &mut serial, |index| {
                    visits[index].fetch_add(1, Ordering::Relaxed);
                });
                assert!(
                    visits
                        .iter()
                        .all(|count| count.load(Ordering::Relaxed) == 1)
                );
            }
        });
    }
}
