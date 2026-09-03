// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: desirability.rs
//  script_path: rust/src/simulation/grid/desirability.rs
//  module_name: desirability
//  version: 0.2.0
//  author: [BantedHam]
//  description: The land value grid. Its base is the delivered engine
//           desirability wherever the probe grid covers, bilinear, with
//           the flat default everywhere else; pollution and noise subtract
//           from there. One snapshot per tick, before the parallel loop.
//  kind: module
//  spec: none
//  internal_dependencies: [data_grid, engine_inputs, pollution, noise]
//  external_dependencies: [godot-rust]
//  features: [land-value, engine-base, parallel-tick]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-09-02
// ========================================================================

use super::data_grid::DataGrid;
use super::noise::NoiseSystem;
use super::pollution::PollutionSystem;
use crate::simulation::zoning::ZoningSystem;
use rayon::prelude::*;

/// A grid-based system that calculates land desirability/value.
///
/// Composite formula: `50 - pollution * 2.0 - noise * 1.5`.
pub struct DesirabilitySystem {
    /// The current land value grid (0-100).
    pub grid: DataGrid<f32>,
}

impl DesirabilitySystem {
    /// Creates a new desirability system derived from the world map configuration.
    pub fn new(config: &crate::simulation::core::config::WorldConfig) -> Self {
        let (w, h) = config.get_env_grid_size();
        Self {
            grid: DataGrid::new(w, h, 0.0),
        }
    }

    /// Recalculates the desirability grid based on current pollution and
    /// noise levels. Where the engine boundary has delivered evaluated
    /// desirability, that is the BASE land value (measured overrides
    /// derived); outside its coverage the flat 50 stays the fallback, so
    /// with no delivery this is byte-identical to the old formula.
    pub fn tick(
        &mut self,
        _zoning: &ZoningSystem,
        pollution: &PollutionSystem,
        noise: &NoiseSystem,
        config: &crate::simulation::core::config::WorldConfig,
    ) {
        let w = self.grid.width;
        let p_grid = &pollution.grid;
        let n_grid = &noise.grid;
        // One snapshot per tick; no lock crosses into the parallel loop.
        let engine = crate::simulation::engine_inputs::snapshot();
        let cell_m = config.env_cell_m as f64;
        let half_w = (config.width_m as f64) * 0.5;
        let half_h = (config.height_m as f64) * 0.5;

        self.grid
            .data
            .par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row)| {
                let world_z = (y as f64 + 0.5) * cell_m - half_h;
                for x in 0..w {
                    let world_x = (x as f64 + 0.5) * cell_m - half_w;
                    let mut land_value = engine
                        .desirability_base(world_x, world_z)
                        .unwrap_or(50.0) as f32;

                    let pol = *p_grid.get(x, y).unwrap_or(&0.0);
                    let nse = *n_grid.get(x, y).unwrap_or(&0.0);

                    land_value -= pol * 2.0;
                    land_value -= nse * 1.5;

                    row[x] = land_value.clamp(0.0, 100.0);
                }
            });
    }
}
