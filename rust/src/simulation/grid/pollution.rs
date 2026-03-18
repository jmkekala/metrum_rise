use super::data_grid::DataGrid;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::buildings::allocator::BuildingAllocator;

pub struct PollutionSystem {
    pub grid: DataGrid<f32>,
}

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

        // 1. Emission (Only Buildings emit smog, empty Zones do not!)
        for b in &allocator.buildings {
            if b.zone_type == ZoneType::Industrial {
                if let Some(val) = new_grid.get_mut(b.x, b.y) {
                    *val += 100.0; // Vastly increased to compensate for per-building vs per-zone
                }
            }
        }

        // 2. Diffusion & 3. Decay
        for y in 0..h {
            for x in 0..w {
                let current = *self.grid.get(x, y).unwrap_or(&0.0);
                
                let mut neighbor_sum = 0.0;
                let mut count = 0.0;
                
                if x > 0 { neighbor_sum += *self.grid.get(x - 1, y).unwrap_or(&0.0); count += 1.0; }
                if x < w - 1 { neighbor_sum += *self.grid.get(x + 1, y).unwrap_or(&0.0); count += 1.0; }
                if y > 0 { neighbor_sum += *self.grid.get(x, y - 1).unwrap_or(&0.0); count += 1.0; }
                if y < h - 1 { neighbor_sum += *self.grid.get(x, y + 1).unwrap_or(&0.0); count += 1.0; }
                
                let avg = if count > 0.0 { neighbor_sum / count } else { 0.0 };
                
                // Diffuse: keep 60% of own, take 40% of avg neighbor. Decay: 99.5% retention.
                let mut propagated = current * 0.60 + avg * 0.40;
                propagated *= 0.995; // Very slow decay, smog will billow outward
                
                if let Some(val) = new_grid.get_mut(x, y) {
                    // Combine emission + diffused old state
                    *val = (*val - current).max(0.0) + propagated; 
                    *val = val.min(100.0).max(0.0); // Cap
                }
            }
        }
        
        self.grid = new_grid;
    }
}
