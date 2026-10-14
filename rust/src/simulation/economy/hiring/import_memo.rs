// SPDX-License-Identifier: GPL-2.0-only

//! Reuse guards for import-only staffing trials within one funding refresh.

/// Cached result of one import-only staffing trial.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ImportMemo {
    ceiling: u32,
    activity_bits: u32,
    workers: u32,
    activity: f32,
    headroom: [f64; 2],
    valid: bool,
}

impl ImportMemo {
    /// Tests a tighter closure with exact input equality in O(output ports), without allocation.
    pub(super) fn matches(
        &self,
        ceiling: u32,
        activity: f32,
        consumer_units: &[f64],
        saved_units: &[f64],
    ) -> bool {
        self.valid
            && self.activity_bits == activity.to_bits()
            && ceiling <= self.ceiling
            && self.workers <= ceiling
            && consumer_units.len() == saved_units.len()
            && consumer_units
                .iter()
                .zip(saved_units)
                .all(|(current, saved)| current.to_bits() == saved.to_bits())
    }

    /// Returns the cached worker count, activity and financial headroom.
    pub(super) fn result(&self) -> (u32, f32, [f64; 2]) {
        (self.workers, self.activity, self.headroom)
    }

    /// Stores one staffing result for reuse during the current refresh.
    pub(super) fn remember(
        &mut self,
        ceiling: u32,
        activity: f32,
        result: (u32, f32),
        headroom: [f64; 2],
    ) {
        self.ceiling = ceiling;
        self.activity_bits = activity.to_bits();
        self.workers = result.0;
        self.activity = result.1;
        self.headroom = headroom;
        self.valid = true;
    }
}
