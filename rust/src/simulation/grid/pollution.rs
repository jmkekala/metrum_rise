use super::data_grid::DataGrid;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::grid::zoning::ZoneType;

pub struct PollutionSystem {
    pub grid: DataGrid<f32>,
    pub swap: DataGrid<f32>,
}

use rayon::prelude::*;

impl PollutionSystem {
    pub fn new(config: &crate::simulation::core::config::MapConfig) -> Self {
        let (w, h) = config.get_env_grid_size();
        Self {
            grid: DataGrid::new(w, h, 0.0),
            swap: DataGrid::new(w, h, 0.0),
        }
    }

    pub fn tick(
        &mut self,
        allocator: &BuildingAllocator,
        config: &crate::simulation::core::config::MapConfig,
    ) {
        // Swap buffers: current grid moves to swap (source),
        // swap (old data) moves to grid (target for this tick)
        std::mem::swap(&mut self.grid, &mut self.swap);
        self.grid.data.fill(0.0);

        let w = self.grid.width;
        let h = self.grid.height;


        // 1. Emission (Sequential as building count is small compared to grid)
        for b in &allocator.buildings {
            if b.zone_type == ZoneType::Industrial {
                let (gx_raw, gy_raw) = config.world_to_env_grid(b.center_x, b.center_y, w, h);
                let gx = gx_raw.round() as i32;
                let gy = gy_raw.round() as i32;

                if gx >= 0 && gx < w as i32 && gy >= 0 && gy < h as i32 {
                    if let Some(val) = self.grid.get_mut(gx as usize, gy as usize) {
                        *val += 5.0;
                    }
                }
            }
        }

        // 2. Diffusion & 3. Decay (Parallelized)
        let old_grid = &self.swap;

        self.grid
            .data
            .par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..w {
                    let current = *old_grid.get(x, y).unwrap_or(&0.0);

                    let mut neighbor_sum = 0.0;
                    let mut count = 0.0;

                    // Neighborhood Sampling
                    if x > 0 {
                        neighbor_sum += *old_grid.get(x - 1, y).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if x < w - 1 {
                        neighbor_sum += *old_grid.get(x + 1, y).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if y > 0 {
                        neighbor_sum += *old_grid.get(x, y - 1).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if y < h - 1 {
                        neighbor_sum += *old_grid.get(x, y + 1).unwrap_or(&0.0);
                        count += 1.0;
                    }

                    let avg = if count > 0.0 {
                        neighbor_sum / count
                    } else {
                        0.0
                    };

                    // Diffuse: keep 60% of own, take 40% of avg neighbor. Decay: 99.5% retention.
                    let propagated = (current * 0.60 + avg * 0.40) * 0.995;

                    // Combine emission (already in row[x]) + diffused state
                    row[x] = (row[x] + propagated).min(100.0).max(0.0);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::grid::zoning::ZoneType;
    use godot::prelude::Vector2;

    #[test]
    fn test_pollution_diffusion_and_decay() {
        let config = MapConfig::default();
        let mut system = PollutionSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        // 1. Add one industrial source at world (0,0)
        // Default MapConfig is 20km x 20km, so (0,0) is at the center of the grid.
        let source_building = Building {
            center_x: 0.0,
            center_y: 0.0,
            width: 30,
            depth: 30,
            zone_type: ZoneType::Industrial,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            frontage_node: 0,
            side_offset: 0.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            variant: 0,
        };
        allocator.buildings.push(source_building);

        let (gw, gh) = config.get_env_grid_size();
        let source_gx = (gw / 2) as usize;
        let source_gy = (gh / 2) as usize;

        // 2. Tick 200 times to allow diffusion
        for _ in 0..200 {
            system.tick(&allocator, &config);
        }

        // 3. Assertions
        let source_val = *system.grid.get(source_gx, source_gy).unwrap();
        assert!(
            source_val > 0.0,
            "Source cell should have positive pollution. Got: {}",
            source_val
        );

        let diffused_val = *system.grid.get(source_gx + 5, source_gy).unwrap();
        assert!(
            diffused_val > 0.0,
            "Cell 5 steps away should have nonzero diffused pollution. Got: {}",
            diffused_val
        );

        // Check for NaN/Inf
        for y in 0..gh {
            for x in 0..gw {
                let val = *system.grid.get(x, y).unwrap();
                assert!(
                    val.is_finite(),
                    "Pollution value at ({}, {}) is not finite: {}",
                    x,
                    y,
                    val
                );
            }
        }

        // 4. Test Decay: Remove source and track average
        allocator.buildings.clear();

        let mut avg_before = 0.0;
        for &val in &system.grid.data {
            avg_before += val;
        }
        avg_before /= system.grid.data.len() as f32;

        // Tick 50 times more
        for _ in 0..50 {
            system.tick(&allocator, &config);
        }

        let mut avg_after = 0.0;
        for &val in &system.grid.data {
            avg_after += val;
        }
        avg_after /= system.grid.data.len() as f32;

        assert!(
            avg_after < avg_before,
            "Average pollution should decay after source removal. Before: {}, After: {}",
            avg_before,
            avg_after
        );
    }
}
