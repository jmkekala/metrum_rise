pub struct WaterSystem {
    pub width: usize,
    pub height: usize,
    pub depth: Vec<f32>,
    pub velocity: Vec<f32>,
    pub flux: Vec<[f32; 4]>, // [Left, Right, Top, Bottom]
    pub sources: Vec<(usize, usize, f32)>, // (x, y, rate)
}

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
        // 0. Inject water from sources
        for &(x, y, rate) in &self.sources {
            let idx = y * self.width + x;
            self.depth[idx] += rate * dt;
        }

        let l = 1.0; // Pipe length
        let a = 1.0; // Pipe area
        let g = 9.81;
        
        // 1. Calculate flux
        for y in 1..self.height - 1 {
            for x in 1..self.width - 1 {
                let idx = y * self.width + x;
                let h_self = terrain[idx] + self.depth[idx];
                
                let mut f = self.flux[idx];
                
                // Neighbors
                let neighbors = [
                    (x - 1, y, 0), // Left
                    (x + 1, y, 1), // Right
                    (x, y - 1, 2), // Top
                    (x, y + 1, 3), // Bottom
                ];
                
                for (nx, ny, i) in neighbors {
                    let n_idx = ny * self.width + nx;
                    let h_neighbor = terrain[n_idx] + self.depth[n_idx];
                    let h_diff = h_self - h_neighbor;
                    
                    f[i] = (f[i] + dt * g * a * (h_diff / l)).max(0.0);
                }
                
                // Scale flux to prevent negative depth
                let total_flux = f[0] + f[1] + f[2] + f[3];
                if total_flux > 0.0 {
                    let k = (self.depth[idx] * l * l / (total_flux * dt)).min(1.0);
                    for i in 0..4 {
                        f[i] *= k;
                    }
                }
                
                self.flux[idx] = f;
            }
        }
        
        // 2. Update depth and calculate velocity
        for y in 1..self.height - 1 {
            for x in 1..self.width - 1 {
                let idx = y * self.width + x;
                
                let fin = self.flux[(y) * self.width + (x - 1)][1] // From left
                        + self.flux[(y) * self.width + (x + 1)][0] // From right
                        + self.flux[(y - 1) * self.width + (x)][3] // From top
                        + self.flux[(y + 1) * self.width + (x)][2]; // From bottom
                
                let fout = self.flux[idx][0] + self.flux[idx][1] + self.flux[idx][2] + self.flux[idx][3];
                
                // Update depth
                self.depth[idx] += dt * (fin - fout) / (l * l);
                if self.depth[idx] < 0.0001 {
                    self.depth[idx] = 0.0;
                }

                // Calculate velocity magnitude (speed)
                // Roughly: (Total Out Flux - Total In Flux) is change, 
                // but let's just use average flux for "speed" visualization
                if self.depth[idx] > 0.001 {
                    self.velocity[idx] = (fin + fout) / (2.0 * self.depth[idx] * l);
                } else {
                    self.velocity[idx] = 0.0;
                }
            }
        }
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
