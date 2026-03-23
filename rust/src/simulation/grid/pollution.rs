use super::data_grid::DataGrid;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::buildings::allocator::BuildingAllocator;

pub struct PollutionSystem {
    pub grid: DataGrid<f32>,
}

use rayon::prelude::*;

impl PollutionSystem {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            grid: DataGrid::new(width, height, 0.0),
        }
    }

    pub fn tick(&mut self, allocator: &BuildingAllocator) {
        let mut new_grid = self.grid.clone();
        
        let w = self.grid.width;
        let h = self.grid.height;

        // 1. Emission (Sequential as building count is small compared to grid)
        for b in &allocator.buildings {
            if b.zone_type == ZoneType::Industrial {
                let cx = b.center_x.round() as usize;
                let cy = b.center_y.round() as usize;
                if let Some(val) = new_grid.get_mut(cx, cy) {
                    *val += 100.0;
                }
            }
        }

        // 2. Diffusion & 3. Decay (Parallelized)
        let old_grid = &self.grid;
        
        new_grid.data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let current = *old_grid.get(x, y).unwrap_or(&0.0);
                
                let mut neighbor_sum = 0.0;
                let mut count = 0.0;
                
                // Neighborhood Sampling
                if x > 0 { neighbor_sum += *old_grid.get(x - 1, y).unwrap_or(&0.0); count += 1.0; }
                if x < w - 1 { neighbor_sum += *old_grid.get(x + 1, y).unwrap_or(&0.0); count += 1.0; }
                if y > 0 { neighbor_sum += *old_grid.get(x, y - 1).unwrap_or(&0.0); count += 1.0; }
                if y < h - 1 { neighbor_sum += *old_grid.get(x, y + 1).unwrap_or(&0.0); count += 1.0; }
                
                let avg = if count > 0.0 { neighbor_sum / count } else { 0.0 };
                
                // Diffuse: keep 60% of own, take 40% of avg neighbor. Decay: 99.5% retention.
                let mut propagated = current * 0.60 + avg * 0.40;
                propagated *= 0.995; 
                
                // Combine emission (already in row[x]) + diffused state
                row[x] = (row[x] - current).max(0.0) + propagated; 
                row[x] = row[x].min(100.0).max(0.0);
            }
        });
        
        self.grid = new_grid;
    }
}
