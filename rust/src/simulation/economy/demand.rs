//! Global R/C/I demand tracking.

/// Tracks the global demand for Residential, Commercial, and Industrial zones.
///
/// Demand is consumed by the building allocator when spawning new buildings
/// and grows organically over time.
pub struct DemandSystem {
    /// Residential demand (0-100).
    pub residential: f32,
    /// Commercial demand (0-100).
    pub commercial: f32,
    /// Industrial demand (0-100).
    pub industrial: f32,
}

impl DemandSystem {
    /// Creates a new demand system with base starter values.
    pub fn new() -> Self {
        Self {
            residential: 50.0,
            commercial: 25.0,
            industrial: 25.0, // Base starter demand
        }
    }

    /// Advances the demand counters by one organic growth step.
    pub fn tick(&mut self) {
        // Simple organic growth for now
        self.residential = (self.residential + 0.3).min(100.0);
        self.commercial = (self.commercial + 0.15).min(100.0);
        self.industrial = (self.industrial + 0.15).min(100.0);
    }
}
