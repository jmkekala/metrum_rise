//! Heightmap terrain system used for road grade, raycasting, and rendering.
//!
//! Two height arrays are maintained: `source_data` (user-sculpted, never modified by roads)
//! and `data` (final map with road beds stamped in). Road snapping and cost calculations
//! always read from `source_data` to avoid feedback loops.

use godot::prelude::Vector3;

/// Dual-buffer heightmap for the terrain surface.
pub struct TerrainSystem {
    /// Map width in height samples. One sample per metre at standard resolution.
    pub width: usize,
    /// Map height (depth) in height samples.
    pub height: usize,
    /// Final/visual heightmap (metres). Road-bed depressions are baked into this buffer.
    pub data: Vec<f32>,
    /// Source heightmap as sculpted by the player, without road modifications.
    /// Used for road grade calculation and slope cost — never written by road placement.
    pub source_data: Vec<f32>,
}

impl TerrainSystem {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0.0; width * height],
            source_data: vec![0.0; width * height],
        }
    }

    pub fn set_height(&mut self, x: usize, y: usize, value: f32) {
        if x < self.width && y < self.height {
            self.source_data[y * self.width + x] = value;
            self.data[y * self.width + x] = value;
        }
    }

    pub fn get_height(&self, x: usize, y: usize) -> f32 {
        if x < self.width && y < self.height {
            self.source_data[y * self.width + x]
        } else {
            0.0
        }
    }

    pub fn get_height_interpolated(&self, x: f32, z: f32) -> f32 {
        let x_clamped = x.clamp(0.0, (self.width - 1) as f32);
        let z_clamped = z.clamp(0.0, (self.height - 1) as f32);
        
        let x0 = x_clamped.floor() as usize;
        let z0 = z_clamped.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let z1 = (z0 + 1).min(self.height - 1);
        
        let fx = x_clamped.fract();
        let fz = z_clamped.fract();
        
        // Sample from SOURCE data for all interpolation
        let h00 = self.source_data[z0 * self.width + x0];
        let h10 = self.source_data[z0 * self.width + x1];
        let h01 = self.source_data[z1 * self.width + x0];
        let h11 = self.source_data[z1 * self.width + x1];
        
        let h0 = h00 * (1.0 - fx) + h10 * fx;
        let h1 = h01 * (1.0 - fx) + h11 * fx;
        
        h0 * (1.0 - fz) + h1 * fz
    }

    pub fn get_normal_interpolated(&self, x: f32, z: f32) -> Vector3 {
        let eps = 0.1;
        let h_x1 = self.get_height_interpolated(x + eps, z);
        let h_x0 = self.get_height_interpolated(x - eps, z);
        let h_z1 = self.get_height_interpolated(x, z + eps);
        let h_z0 = self.get_height_interpolated(x, z - eps);

        let dx = (h_x1 - h_x0) / (2.0 * eps);
        let dz = (h_z1 - h_z0) / (2.0 * eps);

        // This is the gradient in "unit" height units. 
        // We need to scale it by our world height scale (20.0) for accurate slope.
        Vector3::new(-dx * 20.0, 1.0, -dz * 20.0).normalized()
    }

    pub fn raycast_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Option<Vector3> {
        // Transform ray to local heightmap coordinates (0 to width/height)
        // Symmetric Centering: (W-1)*0.5 maps world 0 to grid center (127.5 for 256)
        let hw = (self.width as f32 - 1.0) * 0.5;
        let hh = (self.height as f32 - 1.0) * 0.5;
        
        let local_origin = Vector3::new(ray_origin.x + hw, ray_origin.y, ray_origin.z + hh);
        let local_dir = ray_dir; // Direction stays the same
        
        // Linear search for entry/exit or surface intersection
        let mut t = 0.0;
        let max_dist = 500.0; // Reasonable world limit
        let step = 0.5; // Half-meter steps for safety
        
        let mut prev_diff = local_origin.y - self.get_height_interpolated(local_origin.x, local_origin.z) * 20.0;
        
        while t < max_dist {
            t += step;
            let p = local_origin + local_dir * t;
            
            // Bounds check
            if p.x < 0.0 || p.x >= self.width as f32 || p.z < 0.0 || p.z >= self.height as f32 {
                if t > 0.0 && p.y < -10.0 { break; } // Went under map
                continue;
            }
            
            let h = self.get_height_interpolated(p.x, p.z) * 20.0;
            let diff = p.y - h;
            
            // Intersection detected (crossed the surface)
            if diff.signum() != prev_diff.signum() {
                // Binary search refinement for precision
                let mut t_low = t - step;
                let mut t_high = t;
                for _ in 0..8 {
                    let t_mid = (t_low + t_high) * 0.5;
                    let pm = local_origin + local_dir * t_mid;
                    let hm = self.get_height_interpolated(pm.x, pm.z) * 20.0;
                    if (pm.y - hm).signum() == prev_diff.signum() {
                        t_low = t_mid;
                    } else {
                        t_high = t_mid;
                    }
                }
                
                let final_p = local_origin + local_dir * ((t_low + t_high) * 0.5);
                return Some(Vector3::new(final_p.x - hw, final_p.y, final_p.z - hh));
            }
            
            prev_diff = diff;
        }
        
        None
    }

    pub fn sculpt(&mut self, center_x: f32, center_y: f32, radius: f32, strength: f32) {
        let r_int = radius.ceil() as i32;
        let cx_int = center_x as i32;
        let cy_int = center_y as i32;

        for y in (cy_int - r_int)..=(cy_int + r_int) {
            for x in (cx_int - r_int)..=(cx_int + r_int) {
                if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                    continue;
                }

                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= radius * radius {
                    let dist = dist_sq.sqrt();
                    let normalized_dist = dist / radius;
                    let falloff = (1.0 + (normalized_dist * std::f32::consts::PI).cos()) * 0.5;
                    
                    let idx = (y as usize) * self.width + (x as usize);
                    let current_h = self.source_data[idx];
                    let next_h = current_h + strength * falloff;
                    
                    self.source_data[idx] = next_h;
                    self.data[idx] = next_h;
                }
            }
        }
    }

    pub fn reset_visuals_from_source(&mut self) {
        self.data.copy_from_slice(&self.source_data);
    }
}
