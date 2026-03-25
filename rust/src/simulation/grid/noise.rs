use super::data_grid::DataGrid;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::buildings::allocator::BuildingAllocator;

pub struct NoiseSystem {
    pub grid: DataGrid<f32>,
    pub swap: DataGrid<f32>,
}

use rayon::prelude::*;

impl NoiseSystem {
    pub fn new(config: &crate::simulation::core::config::MapConfig) -> Self {
        let (w, h) = config.get_env_grid_size();
        Self {
            grid: DataGrid::new(w, h, 0.0),
            swap: DataGrid::new(w, h, 0.0),
        }
    }

    pub fn tick(&mut self, allocator: &BuildingAllocator, graph: &RegionGraph, config: &crate::simulation::core::config::MapConfig) {
        std::mem::swap(&mut self.grid, &mut self.swap);
        
        let w = self.grid.width;
        let h = self.grid.height;
        let world_size_x = config.width_m;
        let world_size_y = config.height_m;

        // 1. Emission (Sequential - small count)
        for b in &allocator.buildings {
            let gx = (((b.center_x / world_size_x) + 0.5) * w as f32).round() as usize;
            let gy = (((b.center_y / world_size_y) + 0.5) * h as f32).round() as usize;

            if b.zone_type == ZoneType::Commercial {
                if let Some(val) = self.grid.get_mut(gx, gy) {
                    *val = (*val + 30.0).min(100.0);
                }
            }
            if b.zone_type == ZoneType::Industrial {
                if let Some(val) = self.grid.get_mut(gx, gy) {
                    *val = (*val + 80.0).min(100.0);
                }
            }
        }

        // 1B. Emission (Roads)
        for edge in &graph.edges {
            if edge.deleted { continue; }
            let road_noise = if edge.speed_limit > 60.0 { 4.0 } else { 1.0 };
            for p in &edge.physical_geometry {
                let gx = (((p.x / world_size_x) + 0.5) * w as f32).round() as i32;
                let gz = (((p.z / world_size_y) + 0.5) * h as f32).round() as i32;
                if gx >= 0 && gx < w as i32 && gz >= 0 && gz < h as i32 {
                    if let Some(val) = self.grid.get_mut(gx as usize, gz as usize) {
                        *val = (*val + road_noise).min(100.0);
                    }
                }
            }
        }

        // 2. Diffusion & 3. Decay (Parallelized)
        let old_grid_ref = &self.swap;
        
        self.grid.data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let current = *old_grid_ref.get(x, y).unwrap_or(&0.0);
                
                let mut neighbor_sum = 0.0;
                let mut count = 0.0;
                
                if x > 0 { neighbor_sum += *old_grid_ref.get(x - 1, y).unwrap_or(&0.0); count += 1.0; }
                if x < w - 1 { neighbor_sum += *old_grid_ref.get(x + 1, y).unwrap_or(&0.0); count += 1.0; }
                if y > 0 { neighbor_sum += *old_grid_ref.get(x, y - 1).unwrap_or(&0.0); count += 1.0; }
                if y < h - 1 { neighbor_sum += *old_grid_ref.get(x, y + 1).unwrap_or(&0.0); count += 1.0; }
                
                let avg = if count > 0.0 { neighbor_sum / count } else { 0.0 };
                
                // Noise diffuses very fast, but naturally decays over distance.
                let mut propagated = current * 0.50 + avg * 0.50;
                propagated *= 0.90; // 10% decay per tick
                
                row[x] = (row[x] - current).max(0.0) + propagated; 
                row[x] = row[x].min(100.0).max(0.0); 
            }
        });
    }
}
