// SPDX-License-Identifier: GPL-2.0-only

//! The install scheduler (`RENDER-32-E`, `docs/terrain.md` "Streaming core" 5.4): one
//! streaming budget a frame for every layer, one order, and a cost predicted from fixed
//! constants.
//!
//! A frame's installs run as units (a layer's chunk product, atomic). A unit starts only if its
//! predicted cost fits what is left of the budget, the first unit of a frame excepted, so work
//! always progresses. What is spent is the larger of the units' measured time (a monotonic
//! clock read around every unit) and their predicted cost, so both stay within the budget save
//! for the first unit. The prediction is `calls x CALL_US + bytes x BYTE_US`, constants measured
//! by the `PLAN-STREAM` install microbenchmark (pooled objects, buffer written: 1.5-2.3 us for a
//! 64-instance batch, 71 us for 4,000) with a margin; never adapted at run time, so which units
//! start in a frame depends on the inputs and the clock alone. Units are taken in [`Class`]
//! order, then nearest first by exact squared distance, then by key. `O(1)` per unit.

/// The streaming budget of one frame, microseconds (`PLAN-STREAM` decision 6).
pub(crate) const STREAM_BUDGET_US: f64 = 2000.0;
/// Predicted cost of one backend call, microseconds.
pub(crate) const CALL_US: f64 = 0.5;
/// Predicted cost of one byte written to a buffer, microseconds (0.37 ns measured).
pub(crate) const BYTE_US: f64 = 0.0005;

/// Predicted cost of a unit of `calls` backend calls writing `bytes`.
pub(crate) fn predicted_us(calls: usize, bytes: usize) -> f64 {
    calls as f64 * CALL_US + bytes as f64 * BYTE_US
}

/// Install classes, most urgent first. The frustum changes a chunk's class, never whether it is
/// wanted. (An edit's atomic batch comes first of all once a layer has edits, `G2c`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Class {
    /// Wanted, drawing nothing, in the view.
    HoleInView,
    /// Wanted, drawing nothing, within the near ring.
    HoleNear,
    /// Wanted, drawing nothing.
    Hole,
    /// Replaces a drawn version, in the view.
    UpgradeInView,
    /// Replaces a drawn version.
    Upgrade,
}

/// A source of monotonic microseconds.
pub(crate) trait Clock {
    /// Microseconds since an arbitrary fixed point.
    fn now_us(&self) -> f64;
}

/// The process's monotonic clock.
pub(crate) struct MonotonicClock(std::time::Instant);

impl Default for MonotonicClock {
    fn default() -> Self {
        Self(std::time::Instant::now())
    }
}

impl Clock for MonotonicClock {
    fn now_us(&self) -> f64 {
        self.0.elapsed().as_secs_f64() * 1e6
    }
}

/// One frame's account of the streaming budget.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameBudget {
    budget_us: f64,
    /// Units started this frame.
    pub(crate) units: u32,
    /// Their predicted cost, microseconds.
    pub(crate) predicted_us: f64,
    /// Their measured time, microseconds.
    pub(crate) measured_us: f64,
    /// The longest unit, microseconds.
    pub(crate) max_unit_us: f64,
    /// The largest amount by which a unit's measured time exceeded its prediction.
    pub(crate) max_overrun_us: f64,
}

impl FrameBudget {
    /// A frame's account of `budget_us`.
    pub(crate) fn new(budget_us: f64) -> Self {
        Self {
            budget_us,
            ..Self::default()
        }
    }

    /// Starts a new frame.
    pub(crate) fn begin_frame(&mut self) {
        *self = Self::new(self.budget_us);
    }

    /// What the frame has spent: the larger of the measured and the predicted time.
    pub(crate) fn spent_us(&self) -> f64 {
        self.measured_us.max(self.predicted_us)
    }

    /// Whether a unit predicted at `predicted_us` may start now: always as the frame's first.
    pub(crate) fn admits(&self, predicted_us: f64) -> bool {
        self.units == 0 || self.spent_us() + predicted_us <= self.budget_us
    }

    /// Accounts a unit that ran.
    pub(crate) fn record(&mut self, predicted_us: f64, measured_us: f64) {
        self.units += 1;
        self.predicted_us += predicted_us;
        self.measured_us += measured_us;
        self.max_unit_us = self.max_unit_us.max(measured_us);
        self.max_overrun_us = self.max_overrun_us.max(measured_us - predicted_us);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_unit_always_runs_and_the_rest_fit_the_remainder() {
        let mut frame = FrameBudget::new(100.0);
        assert!(
            frame.admits(500.0),
            "the first unit of a frame always starts"
        );
        frame.record(500.0, 20.0);
        assert!(
            !frame.admits(1.0),
            "the predicted spend counts when it exceeds the clock"
        );
        frame.begin_frame();
        frame.record(30.0, 70.0);
        assert!(
            frame.admits(30.0) && !frame.admits(31.0),
            "the clock counts when it exceeds the prediction"
        );
        assert_eq!(frame.max_overrun_us, 40.0);
    }

    #[test]
    fn classes_order_holes_before_upgrades_and_the_view_first() {
        let mut classes = [
            Class::Upgrade,
            Class::Hole,
            Class::UpgradeInView,
            Class::HoleNear,
            Class::HoleInView,
        ];
        classes.sort();
        assert_eq!(
            classes,
            [
                Class::HoleInView,
                Class::HoleNear,
                Class::Hole,
                Class::UpgradeInView,
                Class::Upgrade
            ]
        );
    }
}
