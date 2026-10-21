// SPDX-License-Identifier: GPL-2.0-only

//! Exact handoff of successful import-only staffing proofs between census phases.

use std::{collections::HashMap, sync::Arc};

use super::super::definitions::RuntimeEconomyCatalog;

/// Exact non-capital inputs used by an import-only staffing proof.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Key {
    /// Runtime profile identity.
    pub(super) profile: u16,
    /// Physical worker capacity used by the activity calculation.
    pub(super) physical: u32,
    /// Output area scale bits.
    pub(super) area: u32,
    /// Utility, tax, import, carrier, and payload price bits.
    pub(super) prices: [u32; 7],
    /// Export mode and multiplier bits.
    pub(super) exports: Option<u32>,
    /// Demand-limited activity bits.
    pub(super) market_activity: u32,
    /// Maximum desired worker count.
    pub(super) desired: u32,
}

/// One successful maximal import-only staffing proof.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Row {
    /// Start of this owner's output-share slice.
    pub(super) first: usize,
    /// Length of this owner's output-share slice.
    pub(super) len: usize,
    key: Key,
    activity: f32,
    headroom: [f64; 2],
    required: f64,
    valid: bool,
    #[cfg(test)]
    /// Number of cross-census replays observed for this row.
    pub(super) replays: usize,
}

impl Row {
    /// Records a successful maximal desired plan for later exact reuse.
    pub(super) fn remember(&mut self, key: Key, activity: f32, headroom: [f64; 2], required: f64) {
        self.key = key;
        self.activity = activity;
        self.headroom = headroom;
        self.required = required;
        self.valid = true;
    }

    /// Invalidates this proof while retaining its sparse slice metadata.
    pub(super) fn invalidate(&mut self) {
        self.valid = false;
    }
}

/// Borrowed exact proof view for one owner.
pub(super) struct Proof<'a> {
    row: &'a Row,
    shares: &'a [u32],
}

impl Proof<'_> {
    /// Reuses the proof when all non-capital inputs and share bits match.
    pub(super) fn reuse(
        &self,
        key: Key,
        shares: &[u32],
        capital: f64,
    ) -> Option<(u32, f32, [f64; 2], f64)> {
        if self.row.key != key
            || self.shares != shares
            || capital
                .partial_cmp(&self.row.required)
                .is_none_or(std::cmp::Ordering::is_lt)
        {
            return None;
        }
        let mut headroom = self.row.headroom;
        headroom[1] = capital - self.row.required;
        Some((key.desired, self.row.activity, headroom, self.row.required))
    }
}

/// Reusable per-owner import-only proofs and their flat output-share storage.
#[derive(Clone, Default)]
pub(super) struct Book {
    /// Per-owner proof rows; callers fill and invalidate these during census setup.
    pub(super) rows: Vec<Row>,
    /// Flat exact output-share bits, partitioned by each row's `first..first + len`.
    pub(super) shares: Vec<u32>,
    catalog: Option<Arc<()>>,
    scratch: Vec<(usize, Row)>,
}

impl Book {
    /// Resets proof validity and prepares sparse row/share storage for a census.
    pub(super) fn prepare(&mut self, offsets: &[usize], catalog: &RuntimeEconomyCatalog) {
        let count = offsets.len().saturating_sub(1);
        self.rows.resize_with(count, Row::default);
        self.rows.truncate(count);
        for (row, bounds) in self.rows.iter_mut().zip(offsets.windows(2)) {
            row.first = bounds[0];
            row.len = bounds[1].saturating_sub(bounds[0]);
            row.invalidate();
            #[cfg(test)]
            {
                row.replays = 0;
            }
        }
        self.shares.resize(offsets.last().copied().unwrap_or(0), 0);
        self.catalog = Some(catalog.cache_identity().clone());
    }

    /// Returns a proof only for the catalog instance that created it.
    pub(super) fn get(&self, owner: usize, catalog: &RuntimeEconomyCatalog) -> Option<Proof<'_>> {
        let identity = self.catalog.as_ref()?;
        let row = self.rows.get(owner)?;
        if !Arc::ptr_eq(identity, catalog.cache_identity()) || !row.valid {
            return None;
        }
        let end = row.first.checked_add(row.len)?;
        Some(Proof {
            row,
            shares: self.shares.get(row.first..end)?,
        })
    }

    /// Invalidates all proofs while retaining allocations.
    pub(super) fn clear(&mut self) {
        for row in &mut self.rows {
            row.invalidate();
        }
        self.catalog = None;
    }

    /// Invalidates one owner while retaining its row and share slice.
    pub(super) fn invalidate(&mut self, owner: usize) {
        if let Some(row) = self.rows.get_mut(owner) {
            row.invalidate();
        }
    }

    /// Moves proof rows between owner indices while preserving their flat share offsets.
    pub(super) fn remap(&mut self, mapping: &HashMap<usize, usize>) {
        if self.rows.is_empty() {
            return;
        }
        let new_len = mapping
            .values()
            .copied()
            .max()
            .map_or(self.rows.len(), |index| self.rows.len().max(index + 1));
        self.rows.resize_with(new_len, Row::default);
        self.scratch.clear();
        self.scratch.reserve(mapping.len());
        for (&old, &new) in mapping {
            if let Some(row) = self.rows.get(old).copied() {
                self.scratch.push((new, row));
            }
        }
        for &old in mapping.keys() {
            if let Some(row) = self.rows.get_mut(old) {
                row.invalidate();
            }
        }
        for &(new, row) in &self.scratch {
            if let Some(destination) = self.rows.get_mut(new) {
                *destination = row;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(desired: u32) -> Key {
        Key {
            profile: 1,
            physical: 8,
            area: 1.0_f32.to_bits(),
            prices: [2; 7],
            exports: Some(1.5_f32.to_bits()),
            market_activity: 0.75_f32.to_bits(),
            desired,
        }
    }

    #[test]
    fn reuse_recomputes_capital_headroom_and_threshold() {
        let mut row = Row {
            first: 0,
            len: 2,
            ..Default::default()
        };
        row.remember(key(4), 0.5, [3.0, 8.0], 10.0);
        let shares = [11, 12];
        let proof = Proof {
            row: &row,
            shares: &shares,
        };
        assert!(proof.reuse(key(4), &shares, 9.99).is_none());
        let result = proof.reuse(key(4), &shares, 13.0).unwrap();
        assert_eq!(result.0, 4);
        assert_eq!(result.2, [3.0, 3.0]);
        assert_eq!(result.3, 10.0);
    }

    #[test]
    fn changed_key_or_shares_invalidates_reuse() {
        let mut row = Row {
            first: 0,
            len: 2,
            ..Default::default()
        };
        row.remember(key(4), 0.5, [3.0, 8.0], 10.0);
        let shares = [11, 12];
        let proof = Proof {
            row: &row,
            shares: &shares,
        };
        assert!(proof.reuse(key(5), &shares, 10.0).is_none());
        assert!(proof.reuse(key(4), &[11, 13], 10.0).is_none());
    }

    #[test]
    fn remap_moves_rows_and_invalidates_unmapped_rows() {
        let mut book = Book {
            rows: vec![
                Row {
                    first: 0,
                    len: 1,
                    ..Default::default()
                },
                Row {
                    first: 1,
                    len: 1,
                    ..Default::default()
                },
            ],
            shares: vec![1, 2],
            ..Default::default()
        };
        book.rows[0].remember(key(2), 0.5, [1.0, 1.0], 2.0);
        book.rows[1].remember(key(3), 0.5, [1.0, 1.0], 2.0);
        let mapping = HashMap::from([(1, 0)]);
        book.remap(&mapping);
        assert!(book.rows[0].valid);
        assert_eq!(book.rows[0].key.desired, 3);
        assert!(!book.rows[1].valid);
    }

    #[test]
    fn remap_preserves_unmapped_rows_and_grows_destinations() {
        let mut book = Book {
            rows: vec![
                Row {
                    first: 0,
                    len: 1,
                    ..Default::default()
                },
                Row {
                    first: 1,
                    len: 1,
                    ..Default::default()
                },
            ],
            shares: vec![1, 2],
            ..Default::default()
        };
        book.rows[0].remember(key(2), 0.5, [1.0, 1.0], 2.0);
        book.rows[1].remember(key(3), 0.5, [1.0, 1.0], 2.0);
        let mapping = HashMap::from([(0, 4)]);
        book.remap(&mapping);
        assert!(!book.rows[0].valid);
        assert!(book.rows[1].valid);
        assert!(book.rows[4].valid);
        assert_eq!(book.rows[4].key.desired, 2);
    }

    #[test]
    fn empty_unprepared_book_stays_empty() {
        let mut book = Book::default();
        book.remap(&HashMap::from([(0, 3)]));
        assert!(book.rows.is_empty());
    }

    #[test]
    fn nan_capital_cannot_reuse() {
        let mut row = Row {
            first: 0,
            len: 1,
            ..Default::default()
        };
        row.remember(key(1), 0.5, [1.0, 1.0], 2.0);
        let shares = [1];
        let proof = Proof {
            row: &row,
            shares: &shares,
        };
        assert!(proof.reuse(key(1), &shares, f64::NAN).is_none());
    }

    #[test]
    fn get_requires_catalog_identity_but_accepts_clone() {
        let catalog = RuntimeEconomyCatalog::default();
        let clone = catalog.clone();
        let other = RuntimeEconomyCatalog::default();
        let mut book = Book::default();
        book.prepare(&[0, 1], &catalog);
        book.shares[0] = 7;
        book.rows[0].remember(key(1), 0.5, [1.0, 1.0], 2.0);
        assert!(book.get(0, &clone).is_some());
        assert!(book.get(0, &other).is_none());
        assert_eq!(book.rows[0].replays, 0);
    }
}
