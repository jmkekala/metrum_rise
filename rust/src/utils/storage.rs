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

/// Makes room for `len` rows in storage indexed by rows that follow the city (buildings,
/// parcels, planning owners and the journals sized by them), keeping spare capacity: when
/// fewer than `len / 8` spare rows would remain, the capacity grows to the larger of `len +
/// len / 4` and twice the old capacity, the amortized growth `reserve` gives. Sized exactly
/// at its first use, such storage otherwise reallocated, copying every row, in the planning
/// slot after one building or parcel was appended, every row array of a book at once
/// (`SIM-02-K`). An empty vector takes a fresh allocation, as [`reserve_cleared`] does.
/// Capacity never changes a result. O(1) without growth, O(len) to grow.
pub(crate) fn reserve_rows<T>(rows: &mut Vec<T>, len: usize) {
    let capacity = spare_capacity(len, rows.capacity());
    if capacity <= rows.capacity() {
        return;
    }
    if rows.is_empty() {
        *rows = Vec::with_capacity(capacity);
    } else {
        rows.reserve_exact(capacity - rows.len());
    }
}

/// The capacity to request for `required` rows of storage now holding `capacity`, with the
/// spare rows of [`reserve_rows`]: `capacity` itself while at least `required / 8` rows
/// would stay spare, else the larger of `required + required / 4` and `2 * capacity`. For
/// arenas grown by request (input tranches, receipts), whose capacity also sizes their
/// journals: requested exactly, the next hour's request a few rows larger doubled the arena and
/// every journal it sizes in one planning slot (`SIM-02-K`). O(1).
pub(crate) fn spare_capacity(required: usize, capacity: usize) -> usize {
    if capacity >= required + required / 8 {
        capacity
    } else {
        (required + required / 4).max(2 * capacity)
    }
}

/// Reserves at least `capacity` rows in `rows` (`SIM-02-K`): for storage copied from a book a
/// census filled, which [`Clone`] gives exactly its rows, so that the copy's next census finds
/// the room the original's held. Capacity never changes a result. O(1) without growth.
pub(crate) fn reserve_capacity<T>(rows: &mut Vec<T>, capacity: usize) {
    rows.reserve_exact(capacity.saturating_sub(rows.len()));
}

/// Rows appended at once from which [`resize_rows`] and [`grow_rows`] fill them on the pool.
const PARALLEL_FILL_ROWS: usize = 1 << 14;

/// [`Vec::resize`] to `len` rows, with the spare capacity of [`reserve_rows`].
pub(crate) fn resize_rows<T: Clone + Send>(rows: &mut Vec<T>, len: usize, value: T) {
    reserve_rows(rows, len);
    fill_rows(rows, len, value);
}

/// Grows `rows` to at least `len` rows, never shrinking it, with the spare capacity of
/// [`reserve_rows`] checked whether or not it grows: storage copied at its length (a loaded
/// book's copy) takes its spare rows at its next preparation.
pub(crate) fn grow_rows<T: Clone + Send>(rows: &mut Vec<T>, len: usize, value: T) {
    reserve_rows(rows, len.max(rows.len()));
    if rows.len() < len {
        fill_rows(rows, len, value);
    }
}

// `Vec::resize`, appending large fills on the pool: a planning pass truncates the owner rows it
// appended to its books and the next pass appends them again, about 190,000 rows an array at
// 100k residents, which one thread wrote in 0.1-0.4 ms an array (`SIM-02-K`). Every appended
// row is `value`, so the rows equal a serial fill's. O(appended rows / p).
fn fill_rows<T: Clone + Send>(rows: &mut Vec<T>, len: usize, value: T) {
    let appended = len.saturating_sub(rows.len());
    if appended >= PARALLEL_FILL_ROWS {
        use rayon::prelude::*;
        rows.par_extend(rayon::iter::repeat_n(value, appended));
    } else {
        rows.resize(len, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_storage_keeps_spare_capacity_and_grows_amortized() {
        let mut rows = vec![0u64; 800];
        assert_eq!(rows.capacity(), 800);
        resize_rows(&mut rows, 800, 0);
        assert_eq!((rows.len(), rows.capacity()), (800, 1600));
        // Appended rows land in spare capacity without reallocating.
        let ptr = rows.as_ptr();
        resize_rows(&mut rows, 1400, 7);
        assert_eq!((rows.as_ptr(), rows.capacity()), (ptr, 1600));
        assert!(rows[..800].iter().all(|&row| row == 0) && rows[800..].iter().all(|&row| row == 7));
        let mut empty: Vec<u64> = Vec::new();
        reserve_rows(&mut empty, 100);
        assert_eq!(empty.capacity(), 125);
    }

    #[test]
    fn large_row_growth_fills_on_the_pool_as_a_serial_fill_does() {
        let len = 3 * PARALLEL_FILL_ROWS + 5;
        let mut rows = vec![[1.0f64, 2.0]; 9];
        grow_rows(&mut rows, len, [0.0, -1.0]);
        let mut serial = vec![[1.0f64, 2.0]; 9];
        serial.resize(len, [0.0, -1.0]);
        assert_eq!(rows, serial);
        // A shrink and a regrowth, as a pass's truncation and the next pass's preparation.
        rows.truncate(9);
        resize_rows(&mut rows, len, [3.0, 4.0]);
        assert_eq!(rows.len(), len);
        assert!(rows[9..].iter().all(|&row| row == [3.0, 4.0]) && rows[..9] == serial[..9]);
        resize_rows(&mut rows, 4, [0.0, 0.0]);
        assert_eq!(rows, serial[..4]);
    }
}
