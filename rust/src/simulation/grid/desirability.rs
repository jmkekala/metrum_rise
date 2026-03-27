use super::data_grid::DataGrid;
use super::noise::NoiseSystem;
use super::pollution::PollutionSystem;
use super::zoning::ZoningSystem;
use rayon::prelude::*;

pub struct DesirabilitySystem {
    pub grid: DataGrid<f32>,
}

impl DesirabilitySystem {
    pub fn new(config: &crate::simulation::core::config::MapConfig) -> Self {
        let (w, h) = config.get_env_grid_size();
        Self {
            grid: DataGrid::new(w, h, 0.0),
        }
    }

    pub fn tick(
        &mut self,
        _zoning: &ZoningSystem,
        pollution: &PollutionSystem,
        noise: &NoiseSystem,
    ) {
        let w = self.grid.width;
        let p_grid = &pollution.grid;
        let n_grid = &noise.grid;

        self.grid
            .data
            .par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..w {
                    let mut land_value = 50.0_f32;

                    let pol = *p_grid.get(x, y).unwrap_or(&0.0);
                    let nse = *n_grid.get(x, y).unwrap_or(&0.0);

                    land_value -= pol * 2.0;
                    land_value -= nse * 1.5;

                    row[x] = land_value.clamp(0.0, 100.0);
                }
            });
    }
}
