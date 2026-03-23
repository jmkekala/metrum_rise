pub struct WaterSystem {
    pub width: usize,
    pub height: usize,
    pub depth: Vec<f32>,
    pub velocity: Vec<f32>,
    pub flux: Vec<[f32; 4]>, // [Left, Right, Top, Bottom]
    pub sources: Vec<(usize, usize, f32)>, // (x, y, rate)
}

use rayon::prelude::*;

impl WaterSystem {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            depth: vec![0.0; width * height],
            velocity: vec![0.0; width * height],
            flux: vec![[0.0; 4]; width * height],
            sources: Vec::new(),
        }
    }

    pub fn tick(&mut self, terrain: &[f32], dt: f32) {
        // 0. Inject water from sources (Sequential but small count)
        for &(x, y, rate) in &self.sources {
            let idx = y * self.width + x;
            self.depth[idx] += rate * dt;
        }

        let l = 1.0; // Pipe length
        let a = 1.0; // Pipe area
        let g = 9.81;
        let w = self.width;
        let h = self.height;

        // --- 1. Calculate flux (Parallelized rows) ---
        // Pre-cloning or sharing depth/terrain for immutable read
        let depth_ref = &self.depth;
        let terrain_ref = terrain;
        
        self.flux.par_chunks_mut(w).enumerate().for_each(|(y, row_flux)| {
            if y == 0 || y >= h - 1 { return; }
            
            for x in 1..w - 1 {
                let idx = y * w + x;
                
                // SKIPPING LOGIC: If cell is dry and has no existing flux, skip.
                // Note: We check if it HAS flux because even if depth is 0, 
                // water might be leaving due to momentum (flux > 0).
                if depth_ref[idx] <= 1e-6 && row_flux[x].iter().all(|&f| f <= 0.0) {
                    // Check neighbors to see if water might flow IN
                    let n1 = (y - 1) * w + x;
                    let n2 = (y + 1) * w + x;
                    if depth_ref[idx-1] <= 1e-6 && depth_ref[idx+1] <= 1e-6 && 
                       depth_ref[n1] <= 1e-6 && depth_ref[n2] <= 1e-6 {
                        continue;
                    }
                }

                let h_self = terrain_ref[idx] + depth_ref[idx];
                let mut f = row_flux[x];
                
                // Neighbors: [Left, Right, Top, Bottom]
                let nx = [x - 1, x + 1, x, x];
                let ny = [y, y, y - 1, y + 1];
                
                for i in 0..4 {
                    let n_idx = ny[i] * w + nx[i];
                    let h_neighbor = terrain_ref[n_idx] + depth_ref[n_idx];
                    let h_diff = h_self - h_neighbor;
                    f[i] = (f[i] + dt * g * a * (h_diff / l)).max(0.0);
                }
                
                // Scale flux to prevent negative depth
                let total_flux = f[0] + f[1] + f[2] + f[3];
                if total_flux > 0.0 {
                    let k = (depth_ref[idx] * l * l / (total_flux * dt)).min(1.0);
                    for i in 0..4 { f[i] *= k; }
                }
                
                row_flux[x] = f;
            }
        });

        // --- 2. Update depth (Parallelized rows) ---
        // Capture a read-only view of flux
        let flux_ref = &self.flux;
        
        self.depth.par_chunks_mut(w).enumerate().enumerate().for_each(|(y_idx, (y, row_depth))| {
            if y == 0 || y >= h - 1 { return; }

            // Using the velocity buffer to also find active rows
            let mut row_vel = vec![0.0; w]; // Temporary for this row, will write to self.velocity later
            
            for x in 1..w - 1 {
                let idx = y * w + x;
                
                let fin = flux_ref[idx - 1][1] // From left
                        + flux_ref[idx + 1][0] // From right
                        + flux_ref[idx - w][3] // From top
                        + flux_ref[idx + w][2]; // From bottom
                
                let fout = flux_ref[idx][0] + flux_ref[idx][1] + flux_ref[idx][2] + flux_ref[idx][3];
                
                if fin <= 1e-8 && fout <= 1e-8 && row_depth[x] <= 1e-6 {
                    continue; // Skip dry land updates
                }

                // Update depth
                row_depth[x] += dt * (fin - fout) / (l * l);
                if row_depth[x] < 0.0001 { row_depth[x] = 0.0; }

                // Calculate velocity magnitude (speed)
                if row_depth[x] > 0.001 {
                    row_vel[x] = (fin + fout) / (2.0 * row_depth[x] * l);
                } else {
                    row_vel[x] = 0.0;
                }
            }
            
            // Note: We need to write row_vel back to self.velocity
            // But self.velocity is currently being borrowed by tick.
            // We'll do a separate pass for it or use unsafe (not recommended).
            // Actually, we can just process velocity in a separate par_iter.
        });

        // Pass 3: Velocity (only for active cells)
        let depth_ref_2 = &self.depth;
        self.velocity.par_chunks_mut(w).enumerate().for_each(|(y, row_vel)| {
            for x in 1..w - 1 {
                let idx = y * w + x;
                if depth_ref_2[idx] > 0.001 {
                    let fin = flux_ref[idx - 1][1] + flux_ref[idx + 1][0] + flux_ref[idx - w][3] + flux_ref[idx + w][2];
                    let fout = flux_ref[idx][0] + flux_ref[idx][1] + flux_ref[idx][2] + flux_ref[idx][3];
                    row_vel[x] = (fin + fout) / (2.0 * depth_ref_2[idx] * l);
                } else {
                    row_vel[x] = 0.0;
                }
            }
        });
    }

    pub fn add_water(&mut self, x: usize, y: usize, amount: f32) {
        if x < self.width && y < self.height {
            self.depth[y * self.width + x] += amount;
        }
    }

    pub fn update_source(&mut self, x: usize, y: usize, rate_add: f32) {
        if x >= self.width || y >= self.height { return; }
        
        if let Some(source) = self.sources.iter_mut().find(|s| s.0 == x && s.1 == y) {
            source.2 += rate_add;
        } else {
            self.sources.push((x, y, rate_add));
        }
    }
}

