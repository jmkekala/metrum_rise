//! Runtime dispatch helpers for the agent tick.

use rayon::prelude::*;

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
