// SPDX-License-Identifier: GPL-2.0-only

//! Fixed-order parallel aggregation for floating-point economy state.

use rayon::prelude::*;

/// Folds fixed input chunks in parallel, then merges them in input order.
///
/// The chunk size and indexed input order define the result, independently of Rayon
/// scheduling and worker count. Work is O(N); temporary storage is O(N / chunk_size)
/// accumulators, with no allocation per input item. Callers must provide a nonzero size.
pub(super) fn ordered_fold<I, T, Identity, Fold, Merge>(
    input: &[I],
    chunk_size: usize,
    identity: Identity,
    fold: Fold,
    merge: Merge,
) -> T
where
    I: Sync,
    T: Send,
    Identity: Fn() -> T + Send + Sync,
    Fold: Fn(&mut T, usize, &I) + Send + Sync,
    Merge: Fn(T, T) -> T,
{
    input
        .par_chunks(chunk_size)
        .enumerate()
        .map(|(chunk_idx, chunk)| {
            let mut total = identity();
            for (offset, item) in chunk.iter().enumerate() {
                fold(&mut total, chunk_idx * chunk_size + offset, item);
            }
            total
        })
        .collect::<Vec<_>>()
        .into_iter()
        .fold(identity(), merge)
}
