// SPDX-License-Identifier: GPL-2.0-only

//! Rings and travel buckets (`RENDER-32-F`, `docs/terrain.md` "Streaming core" 5.1, 5.3): the
//! two ways the core keeps a distance rule over a chunk grid up to date as the view moves,
//! without walking every chunk.
//!
//! - [`RingSet`] keeps the state of a uniform grid's cells as a function of one evaluation point
//!   (a chunk's tier: in the near ring, the far ring, out). The state of a cell can change only
//!   where its centre distance crosses one of the rule's thresholds, so a move of `m` metres
//!   re-evaluates the cells whose centre lies within `m` (plus each band's own reach) of a
//!   threshold: an annulus per band, `O(ring boundary cells)` per band for a move of a cell or
//!   two. A first point or a jump re-evaluates the members and the cells inside the outer band
//!   instead, `O(new set + old set)`. The caller evaluates its exact rule; the ring only chooses
//!   the candidates, so the states are exactly the rule's at the point, whatever the rounding.
//! - [`TravelBuckets`] holds keys whose rule can first change after the view travels some
//!   distance (the graded tiles' T26 scheme): a key evaluated at total travel `t`, whose
//!   distance is `margin` from the nearest threshold, waits in the bucket of `t + margin`; a move
//!   hands back the keys of the buckets the travel reached. `O(keys due + buckets passed)` a
//!   move, nothing on a still frame.
//!
//! Neither allocates once its storage has grown.

use std::collections::HashMap;
use std::hash::Hash;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// A cell of a grid: column and row.
pub(crate) type Cell = [i32; 2];

/// Metres added to every annulus on both sides: the callers' distances are single precision
/// over a few kilometres, whose rounding is millimetres.
pub(crate) const RING_SLACK_M: f64 = 0.05;

/// A uniform square grid: cell `[i, j]` has its centre at
/// `origin + (i + 0.5, j + 0.5) x cell` (metres, double precision; candidates only).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Lattice {
    /// The corner of cell `[0, 0]`.
    pub(crate) origin: [f64; 2],
    /// The cell's side.
    pub(crate) cell: f64,
}

impl Lattice {
    /// The centre of `cell`.
    #[cfg(test)]
    pub(crate) fn centre(&self, cell: Cell) -> [f64; 2] {
        [
            self.origin[0] + (f64::from(cell[0]) + 0.5) * self.cell,
            self.origin[1] + (f64::from(cell[1]) + 0.5) * self.cell,
        ]
    }

    // The cell indices along one axis whose centres lie in `[low, high]`, as an inclusive range
    // (empty when `first > last`).
    fn span(&self, axis: usize, low: f64, high: f64) -> (i64, i64) {
        let first = ((low - self.origin[axis]) / self.cell - 0.5).ceil();
        let last = ((high - self.origin[axis]) / self.cell - 0.5).floor();
        (clamp_index(first), clamp_index(last))
    }

    /// Visits every cell whose centre lies at a distance in `[inner, outer]` from `point` (a
    /// disc for `inner <= 0`). Row by row, two runs a row, so the cost is the cells visited plus
    /// the rows. Returns the cells visited.
    pub(crate) fn for_each_in_annulus(
        &self,
        point: [f64; 2],
        inner: f64,
        outer: f64,
        mut visit: impl FnMut(Cell),
    ) -> usize {
        // A NaN radius visits nothing.
        if outer.is_nan() || outer < 0.0 || outer < inner {
            return 0;
        }
        let mut visited = 0;
        let (row_first, row_last) = self.span(1, point[1] - outer, point[1] + outer);
        for row in row_first..=row_last {
            let z = self.origin[1] + (row as f64 + 0.5) * self.cell;
            let dz = (z - point[1]).abs();
            if dz > outer {
                continue;
            }
            let half_outer = (outer * outer - dz * dz).sqrt();
            let (first, last) = self.span(0, point[0] - half_outer, point[0] + half_outer);
            let row = row as i32;
            let mut run = |from: i64, to: i64| {
                for column in from..=to {
                    visit([column as i32, row]);
                }
                visited += (to - from + 1).max(0) as usize;
            };
            if inner <= dz {
                run(first, last);
                continue;
            }
            // The run whose centres lie strictly inside the inner circle is skipped; a centre on
            // the circle (to rounding) stays in, the callers' slack covers the rest.
            let half_inner = (inner * inner - dz * dz).sqrt();
            let (mut hole_first, mut hole_last) =
                self.span(0, point[0] - half_inner, point[0] + half_inner);
            let at = |column: i64| self.origin[0] + (column as f64 + 0.5) * self.cell;
            if hole_first <= hole_last && distance([at(hole_first), z], point) >= inner {
                hole_first += 1;
            }
            if hole_first <= hole_last && distance([at(hole_last), z], point) >= inner {
                hole_last -= 1;
            }
            if hole_first > hole_last {
                run(first, last);
                continue;
            }
            run(first, last.min(hole_first - 1));
            run(first.max(hole_last + 1), last);
        }
        visited
    }
}

fn clamp_index(value: f64) -> i64 {
    value.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i64
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dz = a[1] - b[1];
    (dx * dx + dz * dz).sqrt()
}

/// One threshold of a ring rule: a cell's state can change only while its centre distance is
/// within `reach` of `radius` (`reach` covers a rule that measures from another point of the
/// cell than its centre).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RingBand {
    /// The threshold, metres from the point.
    pub(crate) radius: f64,
    /// How far from the threshold a cell's centre may be when the rule crosses it.
    pub(crate) reach: f64,
}

/// The state of a grid's cells as a function of one evaluation point, kept incrementally. Cells
/// in the default state are not stored.
pub(crate) struct RingSet<T> {
    lattice: Lattice,
    members: Map<Cell, T>,
    point: Option<[f64; 2]>,
    visited: usize,
    // Members' cells, copied for a full update.
    scratch: Vec<Cell>,
}

impl<T: Copy + PartialEq + Default> RingSet<T> {
    /// An empty set over `lattice`.
    pub(crate) fn new(lattice: Lattice) -> Self {
        Self {
            lattice,
            members: Map::default(),
            point: None,
            visited: 0,
            scratch: Vec::new(),
        }
    }

    /// The grid.
    pub(crate) fn lattice(&self) -> Lattice {
        self.lattice
    }

    /// The cells in a state other than the default.
    pub(crate) fn members(&self) -> impl Iterator<Item = (Cell, T)> + '_ {
        self.members.iter().map(|(cell, state)| (*cell, *state))
    }

    /// The cells the last update and refreshes since visited.
    pub(crate) fn visited(&self) -> usize {
        self.visited
    }

    /// Starts counting visits anew (once a frame).
    pub(crate) fn reset_visited(&mut self) {
        self.visited = 0;
    }

    /// The last evaluation point.
    pub(crate) fn point(&self) -> Option<[f64; 2]> {
        self.point
    }

    /// Forgets every state and the point.
    pub(crate) fn clear(&mut self) {
        self.members.clear();
        self.point = None;
    }

    /// Moves the evaluation point to `point` and re-evaluates every cell whose state can have
    /// changed: by the annulus of each band widened by the move, or, for the first point, a
    /// move past half the outer band (`full_move`) or `full`, the members and every cell inside
    /// the outer band. `evaluate` is the exact rule at `point`; `changed(cell, old, new)` is
    /// called for each change. `O(boundary cells)` per band for a short move.
    pub(crate) fn update(
        &mut self,
        point: [f64; 2],
        bands: &[RingBand],
        full: bool,
        mut evaluate: impl FnMut(Cell) -> T,
        mut changed: impl FnMut(Cell, T, T),
    ) {
        let moved = self.point.map(|last| distance(last, point));
        self.point = Some(point);
        let outer = bands
            .iter()
            .map(|band| band.radius + band.reach)
            .fold(0.0, f64::max);
        if !full && moved == Some(0.0) {
            return;
        }
        let full = full || moved.is_none_or(|moved| moved > outer * 0.5);
        let lattice = self.lattice;
        if full {
            self.scratch.clear();
            self.scratch.extend(self.members.keys().copied());
            let mut scratch = std::mem::take(&mut self.scratch);
            for &cell in &scratch {
                self.visit(cell, &mut evaluate, &mut changed);
            }
            scratch.clear();
            self.scratch = scratch;
            let members = &mut self.members;
            self.visited += lattice.for_each_in_annulus(point, -1.0, outer + RING_SLACK_M, |cell| {
                visit_cell(members, cell, &mut evaluate, &mut changed)
            });
            return;
        }
        let moved = moved.unwrap_or(0.0);
        for band in bands {
            let width = band.reach + moved + RING_SLACK_M;
            let members = &mut self.members;
            self.visited += lattice.for_each_in_annulus(
                point,
                band.radius - width,
                band.radius + width,
                |cell| visit_cell(members, cell, &mut evaluate, &mut changed),
            );
        }
    }

    /// Re-evaluates one cell at the current point (an input of the rule other than the point
    /// changed for it).
    pub(crate) fn refresh(
        &mut self,
        cell: Cell,
        mut evaluate: impl FnMut(Cell) -> T,
        mut changed: impl FnMut(Cell, T, T),
    ) {
        self.visited += 1;
        visit_cell(&mut self.members, cell, &mut evaluate, &mut changed);
    }

    fn visit(
        &mut self,
        cell: Cell,
        evaluate: &mut impl FnMut(Cell) -> T,
        changed: &mut impl FnMut(Cell, T, T),
    ) {
        self.visited += 1;
        visit_cell(&mut self.members, cell, evaluate, changed);
    }
}

fn visit_cell<T: Copy + PartialEq + Default>(
    members: &mut Map<Cell, T>,
    cell: Cell,
    evaluate: &mut impl FnMut(Cell) -> T,
    changed: &mut impl FnMut(Cell, T, T),
) {
    let old = members.get(&cell).copied().unwrap_or_default();
    let new = evaluate(cell);
    if new == old {
        return;
    }
    if new == T::default() {
        members.remove(&cell);
    } else {
        members.insert(cell, new);
    }
    changed(cell, old, new);
}

/// Keys waiting for the view's travel to reach the distance at which their rule can first change.
pub(crate) struct TravelBuckets<K> {
    bucket_m: f64,
    travel: f64,
    first: i64,
    // Key -> its bucket and its index there (swap-remove keeps the indices exact).
    entries: Map<K, (i64, u32)>,
    buckets: Map<i64, Vec<K>>,
    spare: Vec<Vec<K>>,
}

impl<K: Copy + Eq + Hash> TravelBuckets<K> {
    /// Empty buckets `bucket_m` metres of travel wide.
    pub(crate) fn new(bucket_m: f64) -> Self {
        Self {
            bucket_m,
            travel: 0.0,
            first: 0,
            entries: Map::default(),
            buckets: Map::default(),
            spare: Vec::new(),
        }
    }

    /// Keys waiting.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether `key` waits.
    #[cfg(test)]
    pub(crate) fn contains(&self, key: K) -> bool {
        self.entries.contains_key(&key)
    }

    /// Puts `key` (again) in the bucket of the travel `margin` metres from now: it is handed
    /// back by the first move that reaches that bucket. `O(1)`.
    pub(crate) fn put(&mut self, key: K, margin: f64) {
        let bucket = ((self.travel + margin.max(0.0)) / self.bucket_m).floor() as i64;
        if let Some(&(old, slot)) = self.entries.get(&key) {
            if old == bucket {
                return;
            }
            self.unbucket(old, slot);
        }
        let spare = &mut self.spare;
        let keys = self
            .buckets
            .entry(bucket)
            .or_insert_with(|| spare.pop().unwrap_or_default());
        self.entries.insert(key, (bucket, keys.len() as u32));
        keys.push(key);
    }

    /// Takes `key` out. Returns whether it waited. `O(1)`.
    pub(crate) fn remove(&mut self, key: K) -> bool {
        let Some((bucket, slot)) = self.entries.remove(&key) else {
            return false;
        };
        self.unbucket(bucket, slot);
        true
    }

    /// Adds `moved` metres of travel and appends to `due` the keys of every bucket the travel
    /// reached since the last move, the current one included; they leave the buckets. Returns
    /// the buckets visited. Nothing for a move of zero.
    pub(crate) fn advance(&mut self, moved: f64, due: &mut Vec<K>) -> usize {
        if moved <= 0.0 {
            return 0;
        }
        self.travel += moved;
        let last = (self.travel / self.bucket_m).floor() as i64;
        let mut visited = 0;
        for bucket in self.first..=last {
            visited += 1;
            let Some(mut keys) = self.buckets.remove(&bucket) else {
                continue;
            };
            for key in keys.drain(..) {
                self.entries.remove(&key);
                due.push(key);
            }
            self.spare.push(keys);
        }
        self.first = last;
        visited
    }

    /// Forgets every key; the travel carries on.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        for (_, mut keys) in self.buckets.drain() {
            keys.clear();
            self.spare.push(keys);
        }
    }

    /// The capacities of the storage, which a steady frame must leave unchanged (tests).
    #[cfg(test)]
    pub(crate) fn footprint(&self) -> [usize; 3] {
        [
            self.entries.capacity(),
            self.buckets.capacity(),
            self.spare.capacity(),
        ]
    }

    fn unbucket(&mut self, bucket: i64, slot: u32) {
        let keys = self.buckets.get_mut(&bucket).expect("a key's bucket exists");
        keys.swap_remove(slot as usize);
        if let Some(&moved) = keys.get(slot as usize) {
            self.entries.get_mut(&moved).expect("a bucketed key waits").1 = slot;
        }
        if keys.is_empty() {
            let keys = self.buckets.remove(&bucket).expect("present");
            self.spare.push(keys);
        }
    }
}

#[cfg(test)]
mod tests;
