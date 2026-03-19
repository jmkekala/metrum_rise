use super::data_grid::DataGrid;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::graph::TransitGraph;
use crate::simulation::buildings::allocator::BuildingAllocator;

pub struct NoiseSystem {
    pub grid: DataGrid<f32>,
}

impl NoiseSystem {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            grid: DataGrid::new(width, height, 0.0),
        }
    }

    pub fn tick(&mut self, allocator: &BuildingAllocator, graph: &TransitGraph) {
        let mut new_grid = self.grid.clone();
        
        let w = self.grid.width;
        let h = self.grid.height;
        let hw = (w as f32 - 1.0) * 0.5;
        let hh = (h as f32 - 1.0) * 0.5;

        // 1. Emission (Only built structures emit noise, not empty paint)
        for b in &allocator.buildings {
            let cx = b.center_x.round() as usize;
            let cy = b.center_y.round() as usize;
            if b.zone_type == ZoneType::Commercial {
                if let Some(val) = new_grid.get_mut(cx, cy) {
                    *val = (*val + 30.0).min(100.0);
                }
            }
            if b.zone_type == ZoneType::Industrial {
                if let Some(val) = new_grid.get_mut(cx, cy) {
                    *val = (*val + 80.0).min(100.0);
                }
            }
        }

        // 1B. Emission (Roads)
        for edge in &graph.edges {
            // Speed limits > 60 emit heavy noise (highways)
            let road_noise = if edge.speed_limit > 60.0 { 4.0 } else { 1.0 };
            for p in &edge.geometry {
                let x = p.x + hw;
                let y = p.z + hh;
                if x >= 0.0 && x < w as f32 && y >= 0.0 && y < h as f32 {
                    if let Some(val) = new_grid.get_mut(x as usize, y as usize) {
                        *val += road_noise;
                    }
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
                
                // Noise diffuses very fast, but naturally decays over distance.
                let mut propagated = current * 0.50 + avg * 0.50;
                propagated *= 0.90; // 10% decay per tick (reaches further than 15%)
                
                if let Some(val) = new_grid.get_mut(x, y) {
                    *val = (*val - current).max(0.0) + propagated; 
                    *val = val.min(100.0).max(0.0); 
                }
            }
        }
        
        self.grid = new_grid;
    }
}
