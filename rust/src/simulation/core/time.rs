// SPDX-License-Identifier: GPL-2.0-only

//! Operational day-clock and simulation speed control.

use crate::simulation::economy::definitions::load_runtime_economy_tuning;

const MINUTES_PER_DAY: u16 = 24 * 60;
const DEFAULT_SECONDS_PER_DAY: f64 = 24.0 * 60.0;

/// Minute-boundary advancement returned by [`TimeSystem::process_delta`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeAdvance {
    /// Day index before the processed delta.
    pub start_day_index: u32,
    /// Operational minute before the processed delta.
    pub start_minute_of_day: u16,
    /// Number of whole authored minutes crossed by the processed delta.
    pub elapsed_minutes: u16,
}

impl TimeAdvance {
    /// Returns `true` when the processed delta crossed at least one operational minute.
    pub fn has_elapsed_minutes(&self) -> bool {
        self.elapsed_minutes > 0
    }

    /// Returns `true` when at least one day boundary was crossed.
    pub fn crossed_day_boundary(&self) -> bool {
        u32::from(self.start_minute_of_day) + u32::from(self.elapsed_minutes)
            >= u32::from(MINUTES_PER_DAY)
    }

    /// Iterates the exact minute marks crossed by this advancement, in order.
    pub fn iter_elapsed_minutes(&self) -> TimeAdvanceIter {
        TimeAdvanceIter {
            remaining: self.elapsed_minutes,
            day_index: self.start_day_index,
            minute_of_day: self.start_minute_of_day,
        }
    }
}

/// Iterator over the minute marks crossed by one processed delta.
pub struct TimeAdvanceIter {
    remaining: u16,
    day_index: u32,
    minute_of_day: u16,
}

impl Iterator for TimeAdvanceIter {
    type Item = (u32, u16);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        self.minute_of_day += 1;
        if self.minute_of_day >= MINUTES_PER_DAY {
            self.minute_of_day = 0;
            self.day_index = self.day_index.saturating_add(1);
        }
        Some((self.day_index, self.minute_of_day))
    }
}

/// Manages the progression of simulation time on the shared operational clock.
pub struct TimeSystem {
    /// Seconds accumulated inside the current authored minute.
    pub time_elapsed: f64,
    /// Simulation speed multiplier. `0.0` is paused, `1.0` is normal, `2.0` is fast.
    pub speed_multiplier: f32,
    /// Current operational day index (1-indexed).
    pub day_index: u32,
    /// Current minute since operational midnight in `0..1439`.
    pub minute_of_day: u16,
    /// Real seconds required to advance one authored operational day at `1.0x` speed.
    pub seconds_per_day: f64,
}

impl TimeSystem {
    /// Creates a new time system, starting at day `1`, `00:00`, and initially paused.
    pub fn new() -> Self {
        let seconds_per_day = load_runtime_economy_tuning()
            .map(|tuning| tuning.operational_clock.seconds_per_day)
            .unwrap_or(DEFAULT_SECONDS_PER_DAY);
        Self {
            time_elapsed: 0.0,
            speed_multiplier: 0.0,
            day_index: 1,
            minute_of_day: 0,
            seconds_per_day,
        }
    }

    /// Returns the authored seconds per minute on the operational clock.
    pub fn seconds_per_minute(&self) -> f64 {
        self.seconds_per_day / f64::from(MINUTES_PER_DAY)
    }

    /// Returns the absolute authored operational seconds elapsed since day `1 00:00`.
    pub fn operational_time_seconds(&self) -> f64 {
        let elapsed_days = self.day_index.saturating_sub(1) as f64;
        elapsed_days * self.seconds_per_day
            + f64::from(self.minute_of_day) * self.seconds_per_minute()
            + self.time_elapsed
    }

    /// Processes real delta seconds and returns any authored minute boundaries crossed.
    pub fn process_delta(&mut self, delta: f64) -> TimeAdvance {
        let mut advance = TimeAdvance {
            start_day_index: self.day_index,
            start_minute_of_day: self.minute_of_day,
            elapsed_minutes: 0,
        };
        if self.speed_multiplier <= 0.0 {
            return advance;
        }

        self.time_elapsed += delta * self.speed_multiplier as f64;
        let seconds_per_minute = self.seconds_per_minute();
        while self.time_elapsed >= seconds_per_minute {
            self.time_elapsed -= seconds_per_minute;
            self.minute_of_day += 1;
            if self.minute_of_day >= MINUTES_PER_DAY {
                self.minute_of_day = 0;
                self.day_index = self.day_index.saturating_add(1);
            }
            advance.elapsed_minutes = advance.elapsed_minutes.saturating_add(1);
        }
        advance
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SECONDS_PER_DAY, MINUTES_PER_DAY, TimeSystem};

    #[test]
    fn process_delta_advances_exact_minutes() {
        let mut time = TimeSystem {
            time_elapsed: 0.0,
            speed_multiplier: 1.0,
            day_index: 1,
            minute_of_day: 0,
            seconds_per_day: DEFAULT_SECONDS_PER_DAY,
        };

        let advance = time.process_delta(61.0);
        assert_eq!(advance.elapsed_minutes, 61);
        assert_eq!(time.day_index, 1);
        assert_eq!(time.minute_of_day, 61);
    }

    #[test]
    fn process_delta_wraps_day_boundary() {
        let mut time = TimeSystem {
            time_elapsed: 0.0,
            speed_multiplier: 1.0,
            day_index: 3,
            minute_of_day: MINUTES_PER_DAY - 1,
            seconds_per_day: DEFAULT_SECONDS_PER_DAY,
        };

        let advance = time.process_delta(2.0);
        assert_eq!(advance.elapsed_minutes, 2);
        assert!(advance.crossed_day_boundary());
        assert_eq!(time.day_index, 4);
        assert_eq!(time.minute_of_day, 1);
    }
}
