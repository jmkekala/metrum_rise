pub struct TimeSystem {
    pub time_elapsed: f64,
    pub speed_multiplier: f32, // 0.0 = Paused, 1.0 = Normal, 2.0 = Fast
    pub current_day: u32,
    pub seconds_per_day: f64,
}

impl TimeSystem {
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
