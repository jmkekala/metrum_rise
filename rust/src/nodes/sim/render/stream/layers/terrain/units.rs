// SPDX-License-Identifier: GPL-3.0-or-later

//! The patch layers' units by kind (`TERRAIN-41` L1): what each kind of unit costs the frame's
//! install ledger, counted a frame ([`super::PatchWork`]) and kept as a histogram of measured
//! microseconds over a session: the evidence for the fixed unit prices ([`super::UnitCosts`]).
//! Measuring never changes a decision: the prices stay constants.

/// A unit's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnitKind {
    /// An activation of a patch whose node is built (kept hidden, or prewarmed): shown again.
    Show,
    /// An activation that builds the patch's node from its payload.
    Build,
    /// A resident patch hidden because it left.
    Hide,
    /// A resident patch given the mesh of another detail step.
    Step,
    /// A prewarm build (hidden).
    Prewarm,
    /// A release of a kept patch outside the budget's release rectangle (`TERRAIN-41` B1).
    Release,
}

/// The unit kinds.
pub(crate) const UNIT_KINDS: usize = 6;
/// The kinds' names in the perf record, in [`UnitKind`] order.
pub(crate) const UNIT_KIND_NAMES: [&str; UNIT_KINDS] =
    ["show", "build", "hide", "step", "prewarm", "release"];

// One-microsecond buckets; the last holds every unit at or above it.
const BUCKETS: usize = 2048;

/// Measured unit times by kind since the last [`UnitTimes::clear`]: a histogram of one
/// microsecond buckets (the last open above 2,047 us), sized once. `O(1)` a unit, `O(buckets)` a
/// percentile.
pub(crate) struct UnitTimes {
    buckets: Vec<u32>,
    sum_us: [f64; UNIT_KINDS],
    max_us: [f64; UNIT_KINDS],
}

impl Default for UnitTimes {
    fn default() -> Self {
        Self {
            buckets: vec![0; UNIT_KINDS * BUCKETS],
            sum_us: [0.0; UNIT_KINDS],
            max_us: [0.0; UNIT_KINDS],
        }
    }
}

impl UnitTimes {
    /// Counts a unit of `kind` that took `us`.
    pub(crate) fn add(&mut self, kind: UnitKind, us: f64) {
        let kind = kind as usize;
        let bucket = (us.max(0.0) as usize).min(BUCKETS - 1);
        self.buckets[kind * BUCKETS + bucket] += 1;
        self.sum_us[kind] += us;
        self.max_us[kind] = self.max_us[kind].max(us);
    }

    /// Starts over.
    pub(crate) fn clear(&mut self) {
        self.buckets.fill(0);
        self.sum_us = [0.0; UNIT_KINDS];
        self.max_us = [0.0; UNIT_KINDS];
    }

    fn kind(&self, kind: usize) -> &[u32] {
        &self.buckets[kind * BUCKETS..(kind + 1) * BUCKETS]
    }

    /// Units of `kind` counted.
    pub(crate) fn count(&self, kind: usize) -> u64 {
        self.kind(kind).iter().map(|&n| u64::from(n)).sum()
    }

    /// Their mean and longest, microseconds (0 without units).
    pub(crate) fn mean_max(&self, kind: usize) -> [f64; 2] {
        let count = self.count(kind);
        if count == 0 {
            return [0.0; 2];
        }
        [self.sum_us[kind] / count as f64, self.max_us[kind]]
    }

    /// The `q` quantile (0..=1) of `kind`'s units, as the upper edge of its bucket,
    /// microseconds (0 without units).
    pub(crate) fn quantile(&self, kind: usize, q: f64) -> f64 {
        let count = self.count(kind);
        if count == 0 {
            return 0.0;
        }
        // The smallest bucket holding at least ceil(q x count) units.
        let rank = ((q * count as f64).ceil() as u64).max(1);
        let mut seen = 0;
        for (bucket, &n) in self.kind(kind).iter().enumerate() {
            seen += u64::from(n);
            if seen >= rank {
                return (bucket + 1) as f64;
            }
        }
        BUCKETS as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_are_bucket_edges_by_rank() {
        let mut times = UnitTimes::default();
        for us in 0..100 {
            times.add(UnitKind::Show, f64::from(us) + 0.5);
        }
        times.add(UnitKind::Build, 5000.0);
        let show = UnitKind::Show as usize;
        assert_eq!(times.count(show), 100);
        assert_eq!(times.quantile(show, 0.95), 95.0);
        assert_eq!(times.quantile(show, 0.5), 50.0);
        assert_eq!(times.mean_max(show), [50.0, 99.5]);
        assert_eq!(times.quantile(UnitKind::Build as usize, 0.95), BUCKETS as f64);
        assert_eq!(times.quantile(UnitKind::Hide as usize, 0.95), 0.0);
        times.clear();
        assert_eq!(times.count(show), 0);
    }
}
