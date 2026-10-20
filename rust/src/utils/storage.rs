// SPDX-License-Identifier: GPL-2.0-only

//! Reusable storage growth without copying discarded rows (`SIM-02-K`).

/// [`Vec::reserve`] for storage whose rows were just cleared: an empty vector with too little
/// capacity takes a fresh allocation of the capacity `reserve` would give, where `reserve`
/// would reallocate and copy the whole old allocation although no row survives. A non-empty
/// vector reserves as usual. O(1) for an empty vector, plus the allocation.
pub(crate) fn reserve_cleared<T>(rows: &mut Vec<T>, additional: usize) {
    if rows.is_empty() && rows.capacity() < additional {
        // `reserve`'s amortized growth for an empty vector: at least double the capacity and
        // at least its smallest non-zero capacity, so capacities stay those `reserve` gives.
        let smallest = match std::mem::size_of::<T>() {
            1 => 8,
            size if size <= 1024 => 4,
            _ => 1,
        };
        *rows = Vec::with_capacity(additional.max(rows.capacity() * 2).max(smallest));
    } else {
        rows.reserve(additional);
    }
}
