pub struct DemandSystem {
    pub residential: f32,
    pub commercial: f32,
    pub industrial: f32,
}

impl DemandSystem {
    pub fn new() -> Self {
        Self {
            residential: 50.0,
            commercial: 25.0,
            industrial: 25.0, // Base starter demand
        }
    }

    pub fn tick(&mut self) {
        // Simple organic growth for now
        self.residential = (self.residential + 0.1).min(100.0);
        self.commercial = (self.commercial + 0.05).min(100.0);
        self.industrial = (self.industrial + 0.05).min(100.0);
    }
}
