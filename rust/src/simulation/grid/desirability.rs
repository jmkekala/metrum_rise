use super::data_grid::DataGrid;
use super::pollution::PollutionSystem;
use super::noise::NoiseSystem;
use super::zoning::ZoningSystem;

pub struct DesirabilitySystem {
    pub grid: DataGrid<f32>,
}

impl DesirabilitySystem {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            grid: DataGrid::new(width, height, 0.0),
        }
    }

    pub fn tick(&mut self, _zoning: &ZoningSystem, pollution: &PollutionSystem, noise: &NoiseSystem) {
        let w = self.grid.width;
        let h = self.grid.height;

        for y in 0..h {
            for x in 0..w {
                let mut land_value = 50.0_f32; // Default base value for all tiles natively since physical integer proximity matrices are removed


                let pol = *pollution.grid.get(x, y).unwrap_or(&0.0);
                let nse = *noise.grid.get(x, y).unwrap_or(&0.0);

                land_value -= pol * 2.0; // Severe penalty for smog
                land_value -= nse * 1.5; // Moderate penalty for traffic/commercial noise

                // Note: Industrial desirability might actually like being near roads and NOT care about noise/pollution!
                // But for a generic land-value overlay, this is exactly what we want.
                land_value = land_value.clamp(0.0, 100.0);
                
                self.grid.set(x, y, land_value);
            }
        }
    }
}
