// SPDX-License-Identifier: GPL-3.0-or-later

//! A car place's long way (`PARK-25`): a route of more than four points, from its gate's road
//! connection to its stop, kept outside the place row. The row holds a `u32` way id (or
//! [`NO_WAY`]) and keeps the way's first, second, second to last and last points in its `path`,
//! as a dock's row does, so every reader of `path` is unchanged. The ids index a span table; the
//! points of every way share one pool.
//!
//! Pool rule: a freed run of N points goes on the free list for length N, and the next way of N
//! points takes the newest such run; any other way appends. So adding and freeing are O(points),
//! nothing is ever moved or compacted, and an edit touches only its own lot's ways. The pool's
//! high-water mark is at most the sum over lengths of the most ways of that length alive at once.
//! Ids and offsets are derived, never saved or compared: a load inserts the saved ways in row
//! order.

use crate::assets::asset::validation::MAX_ROUTE_POINTS;

/// `Place::way` of a place whose way is its row's four `path` points.
pub(super) const NO_WAY: u32 = u32::MAX;

/// The fewest points a stored way has: four fit the row's `path`.
pub(super) const MIN_WAY_POINTS: usize = 5;

// One stored way: 12 bytes. `metres` is the way's length, summed once when it is stored, so the
// catchment ordering's `maneuver_seconds` reads it instead of summing per call.
#[derive(Clone, Copy, Debug)]
struct Span {
    start: u32,
    len: u32,
    metres: f32,
}

/// Every long way of the parking system, by way id.
#[derive(Default, Debug)]
pub(super) struct CarWays {
    spans: Vec<Span>,
    // Ids of freed spans, newest last.
    free_ids: Vec<u32>,
    points: Vec<[f32; 2]>,
    // By length: the start offsets of freed runs of that many points, newest last.
    free_runs: [Vec<u32>; MAX_ROUTE_POINTS + 1],
}

impl CarWays {
    /// Whether `points` is a way this pool stores: more than four points and at most
    /// `MAX_ROUTE_POINTS`. O(1).
    pub(super) fn storable(points: &[[f32; 2]]) -> bool {
        (MIN_WAY_POINTS..=MAX_ROUTE_POINTS).contains(&points.len())
    }

    /// Stores `points` (see [`Self::storable`]) and returns its id. O(points); allocates only
    /// when no freed run of its length and no freed id is left.
    pub(super) fn insert(&mut self, points: &[[f32; 2]]) -> u32 {
        debug_assert!(Self::storable(points));
        let len = points.len();
        let start = match self.free_runs[len].pop() {
            Some(start) => {
                self.points[start as usize..start as usize + len].copy_from_slice(points);
                start
            }
            None => {
                let start = self.points.len() as u32;
                self.points.extend_from_slice(points);
                start
            }
        };
        let span = Span {
            start,
            len: len as u32,
            metres: length(points),
        };
        match self.free_ids.pop() {
            Some(id) => {
                self.spans[id as usize] = span;
                id
            }
            None => {
                self.spans.push(span);
                (self.spans.len() - 1) as u32
            }
        }
    }

    /// Frees way `id` for reuse. O(1).
    pub(super) fn remove(&mut self, id: u32) {
        let span = self.spans[id as usize];
        self.free_runs[span.len as usize].push(span.start);
        self.free_ids.push(id);
    }

    /// Way `id`'s points, from the road connection to the stop. O(1).
    pub(super) fn points(&self, id: u32) -> &[[f32; 2]] {
        let span = self.spans[id as usize];
        &self.points[span.start as usize..(span.start + span.len) as usize]
    }

    /// Way `id`'s length in metres, as stored. O(1).
    pub(super) fn metres(&self, id: u32) -> f32 {
        self.spans[id as usize].metres
    }

    /// Points the pool holds, live and freed: its high-water mark. O(1), test builds only.
    #[cfg(test)]
    pub(super) fn pool_len(&self) -> usize {
        self.points.len()
    }

    /// Checks the pool against the ids the rows hold: every live span in bounds and of a
    /// storable length, no two live spans or freed runs sharing a point, every id either live or
    /// freed, once. O(ids + pool points), test builds only.
    #[cfg(test)]
    pub(super) fn audit(&self, live: impl Iterator<Item = u32>) -> Result<(), String> {
        let mut owner = vec![false; self.points.len()];
        let mut claim = |start: u32, len: usize, what: &str| -> Result<(), String> {
            let run = owner
                .get_mut(start as usize..start as usize + len)
                .ok_or_else(|| format!("{what} at {start} out of bounds"))?;
            if run.iter().any(|&taken| taken) {
                return Err(format!("{what} at {start} overlaps another"));
            }
            run.fill(true);
            Ok(())
        };
        let mut seen = vec![false; self.spans.len()];
        for id in live {
            let span = self.spans.get(id as usize).ok_or("a row names no way")?;
            if std::mem::replace(&mut seen[id as usize], true) {
                return Err(format!("way {id} held by two rows"));
            }
            if !(MIN_WAY_POINTS..=MAX_ROUTE_POINTS).contains(&(span.len as usize)) {
                return Err(format!("way {id} has {} points", span.len));
            }
            claim(span.start, span.len as usize, "way")?;
        }
        for &id in &self.free_ids {
            if std::mem::replace(&mut seen[id as usize], true) {
                return Err(format!("way {id} both freed and held"));
            }
        }
        if seen.iter().any(|&s| !s) {
            return Err("a way id is neither held nor freed".into());
        }
        for (len, runs) in self.free_runs.iter().enumerate() {
            for &start in runs {
                claim(start, len, "freed run")?;
            }
        }
        if owner.iter().any(|&o| !o) {
            return Err("a pool point is neither held nor freed".into());
        }
        Ok(())
    }
}

/// A route's length in metres, leg by leg in travel order: the sum `maneuver_seconds` takes of a
/// row's four points. O(points).
pub(super) fn length(points: &[[f32; 2]]) -> f32 {
    points
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}
