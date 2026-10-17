// SPDX-License-Identifier: GPL-2.0-only

//! Land desirability derived from the current pollution and noise fields.

use std::ops::Range;

use super::data_grid::DataGrid;
use super::noise::NoiseSystem;
use super::pollution::PollutionSystem;
use rayon::prelude::*;

/// A grid-based system that calculates land desirability/value.
///
/// Composite formula: `50 - pollution * 2.0 - noise * 1.5`.
pub struct DesirabilitySystem {
    /// The current land value grid (0-100), which every reader uses.
    pub grid: DataGrid<f32>,
    /// The next grid being built (`SIM-02-H`), composed row by row from the pollution and noise
    /// back grids as their rows finish.
    pub back: DataGrid<f32>,
}

impl DesirabilitySystem {
    /// Creates a new desirability system derived from the world map configuration.
    pub fn new(config: &crate::simulation::core::config::WorldConfig) -> Self {
        let (w, h) = config.get_env_grid_size();
        Self {
            grid: DataGrid::new(w, h, 0.0),
            back: DataGrid::new(w, h, 0.0),
        }
    }

    /// Recalculates the desirability grid based on current pollution and noise levels.
    pub fn tick(&mut self, pollution: &PollutionSystem, noise: &NoiseSystem) {
        let rows = 0..self.grid.height;
        compose_rows(&mut self.grid, rows, &pollution.grid, &noise.grid);
    }

    /// Composes `rows` of the back grid from the same rows of the pollution and noise back
    /// grids. O(rows × width).
    pub(crate) fn compose_back_rows(
        &mut self,
        rows: Range<usize>,
        pollution: &PollutionSystem,
        noise: &NoiseSystem,
    ) {
        compose_rows(&mut self.back, rows, &pollution.back, &noise.back);
    }

    /// Makes the finished back grid the front; the old front becomes the next back. O(1).
    pub(crate) fn swap_buffers(&mut self) {
        std::mem::swap(&mut self.grid, &mut self.back);
    }
}

// Writes `rows` of `target` from the pollution and noise values of the same cells.
fn compose_rows(
    target: &mut DataGrid<f32>,
    rows: Range<usize>,
    p_grid: &DataGrid<f32>,
    n_grid: &DataGrid<f32>,
) {
    if rows.is_empty() {
        return;
    }
    let w = target.width;
    let first = rows.start;
    target.data[rows.start * w..rows.end * w]
        .par_chunks_mut(w)
        .enumerate()
        .for_each(|(offset, row)| {
            let y = first + offset;
            for (x, cell) in row.iter_mut().enumerate() {
                let mut land_value = 50.0_f32;

                let pol = *p_grid.get(x, y).unwrap_or(&0.0);
                let nse = *n_grid.get(x, y).unwrap_or(&0.0);

                land_value -= pol * 2.0;
                land_value -= nse * 1.5;

                *cell = land_value.clamp(0.0, 100.0);
            }
        });
}
