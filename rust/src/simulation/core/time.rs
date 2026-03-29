//! Day-clock and simulation speed control.

/// Manages the progression of simulation time and day cycles.
pub struct TimeSystem {
    /// Total real-world seconds elapsed since the start of the simulation, scaled by speed.
    pub time_elapsed: f64,
    /// Simulation speed multiplier. 0.0 is paused, 1.0 is normal, 2.0 is fast.
    pub speed_multiplier: f32,
    /// The current simulation day (1-indexed).
    pub current_day: u32,
    /// The number of real-world seconds required to advance one simulation day at 1.0 speed.
    pub seconds_per_day: f64,
}

impl TimeSystem {
    /// Creates a new time system, starting at day 1 and initially paused.
    pub fn new() -> Self {
        Self {
            time_elapsed: 0.0,
            speed_multiplier: 0.0, // Start paused
            current_day: 1,
            seconds_per_day: 2.0, // 2 real seconds = 1 day at 1.0 speed
        }
    }

    /// Process time delta. Returns `true` if a simulation discrete step (a day) has occurred.
    pub fn process_delta(&mut self, delta: f64) -> bool {
        if self.speed_multiplier <= 0.0 {
            return false; // Paused
        }

        self.time_elapsed += delta * self.speed_multiplier as f64;

        if self.time_elapsed >= self.seconds_per_day {
            // We only trigger ONE tick per Godot frame minimum, even if massive lag.
            self.time_elapsed -= self.seconds_per_day;
            self.current_day += 1;
            true
        } else {
            false
        }
    }
}
