// SPDX-License-Identifier: GPL-2.0-only

//! Operational day-clock and simulation speed control.

use crate::simulation::economy::definitions::load_runtime_economy_tuning;

/// Operational minutes in one day, independent of the authored real-time day duration.
pub(crate) const MINUTES_PER_DAY: u16 = 24 * 60;

/// Supported UI speed steps; the final entry also bounds live and saved speed multipliers.
pub(crate) const SIMULATION_SPEED_STEPS: [f32; 8] = [0.0, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];

const DEFAULT_SECONDS_PER_DAY: f64 = 24.0 * 60.0;

// A fresh city opens mid-morning rather than at midnight. The clock drives the rendered
// day/night cycle, so starting at minute zero would open every new game in the dark, and it
// would spend the first seven operational hours before anyone commutes. Loaded saves restore
// their own clock and are unaffected.
const START_MINUTE_OF_DAY: u16 = 7 * 60 + 30;

/// Validates a speed request and preserves negative-to-pause input handling.
pub(crate) fn validated_simulation_speed(speed: f32) -> Option<f32> {
    (speed.is_finite() && speed <= SIMULATION_SPEED_STEPS[SIMULATION_SPEED_STEPS.len() - 1])
        .then(|| speed.max(0.0))
}

/// Enforces the authored minimum day duration at both configuration and save boundaries.
pub(crate) fn validate_day_duration(seconds_per_day: f64) -> Result<(), &'static str> {
    if seconds_per_day.is_finite() && seconds_per_day >= 60.0 {
        Ok(())
    } else {
        Err("day duration must be finite and at least 60 seconds")
    }
}

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
    /// Creates a new time system, starting at day `1`, `07:30`, and initially paused.
    pub fn new() -> Self {
        let seconds_per_day = load_runtime_economy_tuning()
            .map(|tuning| tuning.operational_clock.seconds_per_day)
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        Self {
            time_elapsed: 0.0,
            speed_multiplier: 0.0,
            day_index: 1,
            minute_of_day: START_MINUTE_OF_DAY,
            seconds_per_day,
        }
    }

    /// Returns the authored seconds per minute on the operational clock.
    pub fn seconds_per_minute(&self) -> f64 {
        self.seconds_per_day / f64::from(MINUTES_PER_DAY)
    }

    /// Returns the position inside the current operational day as a fraction in `0.0..1.0`.
    ///
    /// `0.0` is midnight and `0.5` is midday. Unlike [`Self::minute_of_day`] this includes the
    /// partial minute in progress, so a renderer driving a day/night cycle from it moves
    /// continuously instead of stepping once per authored minute.
    pub fn day_fraction(&self) -> f32 {
        let seconds_per_day = if self.seconds_per_day > 0.0 {
            self.seconds_per_day
        } else {
            DEFAULT_SECONDS_PER_DAY
        };
        let elapsed = f64::from(self.minute_of_day) * self.seconds_per_minute() + self.time_elapsed;
        // The accumulator is drained every whole minute, so this only exceeds a day if a caller
        // hand-built a TimeSystem with an out-of-range minute. Wrap rather than clamp to 1.0,
        // which would read as midnight-tomorrow instead of midnight.
        (elapsed / seconds_per_day).rem_euclid(1.0) as f32
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

/// Creates an isolated one-second-per-minute test clock without loading gameplay tuning.
#[cfg(test)]
pub(crate) const fn test_clock(day_index: u32, minute_of_day: u16) -> TimeSystem {
    TimeSystem {
        time_elapsed: 0.0,
        speed_multiplier: 1.0,
        day_index,
        minute_of_day,
        seconds_per_day: MINUTES_PER_DAY as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SECONDS_PER_DAY, MINUTES_PER_DAY, TimeSystem, test_clock};

    #[test]
    fn process_delta_advances_exact_minutes() {
        let mut time = test_clock(1, 0);

        let advance = time.process_delta(61.0);
        assert_eq!(advance.elapsed_minutes, 61);
        assert_eq!(time.day_index, 1);
        assert_eq!(time.minute_of_day, 61);
    }

    #[test]
    fn day_fraction_is_continuous_inside_the_minute_and_wraps_at_midnight() {
        let mut time = TimeSystem {
            time_elapsed: 0.0,
            speed_multiplier: 1.0,
            day_index: 1,
            minute_of_day: 0,
            seconds_per_day: DEFAULT_SECONDS_PER_DAY,
        };
        assert_eq!(time.day_fraction(), 0.0);

        // Midday is exactly half the authored day.
        time.minute_of_day = MINUTES_PER_DAY / 2;
        assert!((time.day_fraction() - 0.5).abs() < 1e-6);

        // The partial minute advances the fraction, which is the whole point of the accessor.
        let before = time.day_fraction();
        time.time_elapsed = time.seconds_per_minute() * 0.5;
        assert!(time.day_fraction() > before);

        // A day boundary returns to midnight rather than saturating at 1.0.
        time.minute_of_day = MINUTES_PER_DAY - 1;
        time.time_elapsed = 0.0;
        time.process_delta(time.seconds_per_minute());
        assert_eq!(time.minute_of_day, 0);
        assert_eq!(time.day_fraction(), 0.0);
    }

    #[test]
    fn process_delta_wraps_day_boundary() {
        let mut time = test_clock(3, MINUTES_PER_DAY - 1);

        let advance = time.process_delta(2.0);
        assert_eq!(advance.elapsed_minutes, 2);
        assert!(advance.crossed_day_boundary());
        assert_eq!(time.day_index, 4);
        assert_eq!(time.minute_of_day, 1);
    }
}
