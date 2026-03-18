use super::data_grid::DataGrid;
use crate::simulation::network::graph::TransitGraph;
use godot::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ZoneType {
    None = 0,
    Residential = 1,
    Commercial = 2,
    Industrial = 3,
    Mixed = 4,
}

pub struct ZoningSystem {
    pub zones: DataGrid<ZoneType>,
    pub validity_mask: DataGrid<bool>,
}

impl ZoningSystem {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            zones: DataGrid::new(width, height, ZoneType::None),
            validity_mask: DataGrid::new(width, height, false),
        }
    }

    /// Paints a circular brush of a specific zone type.
    /// Only paints cells where `validity_mask` is true (close to a road).
    pub fn paint_zone(&mut self, center_x: f32, center_y: f32, radius: f32, zone_type: ZoneType) {
        let r_int = radius.ceil() as i32;
        let cx = center_x as i32;
        let cy = center_y as i32;

        let r_sq = radius * radius;
        let mut painted_count = 0;

        for y in (cy - r_int)..=(cy + r_int) {
            for x in (cx - r_int)..=(cx + r_int) {
                if x < 0 || y < 0 { continue; }
                
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                
                if dx * dx + dy * dy <= r_sq {
                    let ux = x as usize;
                    let uy = y as usize;
                    
                    if self.validity_mask.in_bounds(ux, uy) {
                        // Only allow painting if the cell is close to a road
                        // (Or if we are erasing, zone_type == None)
                        if zone_type == ZoneType::None || *self.validity_mask.get(ux, uy).unwrap_or(&false) {
                            self.zones.set(ux, uy, zone_type);
                            painted_count += 1;
                        }
                    }
                }
            }
        }
        godot_print!("Rust Paint Zone: ({},{}), radius: {}, type: {:?} -> Painted {} cells.", cx, cy, radius, zone_type, painted_count);
    }

    /// Updates the boolean validity mask where zoning is allowed (e.g. within 40m of any road)
    pub fn update_validity_mask(&mut self, graph: &TransitGraph, max_distance: f32) {
        // Simple flood-fill or distance sweep.
        // For now, reset and distance check.
        // A more optimal approach is to draw the roads onto the grid and dilate.

        self.validity_mask = DataGrid::new(self.zones.width, self.zones.height, false);

        let hw = (self.zones.width as f32 - 1.0) * 0.5;
        let hh = (self.zones.height as f32 - 1.0) * 0.5;
        
        let mut valid_count = 0;

        for edge in &graph.edges {
            for p in &edge.geometry {
                // Convert World Space (-128 to 128) to Array Space (0 to 256)
                let center_x = p.x + hw;
                let center_y = p.z + hh; // 3D z is 2D y
                
                let r_int = max_distance.ceil() as i32;
                let cx = center_x as i32;
                let cy = center_y as i32;
                let r_sq = max_distance * max_distance;

                for y in (cy - r_int)..=(cy + r_int) {
                    for x in (cx - r_int)..=(cx + r_int) {
                        if x < 0 || y < 0 { continue; }
                        let dx = x as f32 - center_x;
                        let dy = y as f32 - center_y;
                        if dx * dx + dy * dy <= r_sq {
                            if self.validity_mask.in_bounds(x as usize, y as usize) {
                                if !*self.validity_mask.get(x as usize, y as usize).unwrap() {
                                    self.validity_mask.set(x as usize, y as usize, true);
                                    valid_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        godot_print!("Updated Validity Mask! Total valid zoned cells: {}", valid_count);
    }

    /// Converts the zone grid into an RGBA byte array for Godot Textures.
    pub fn generate_image_data(&self) -> PackedByteArray {
        let mut pixels = Vec::with_capacity(self.zones.width * self.zones.height * 4);

        for y in 0..self.zones.height {
            for x in 0..self.zones.width {
                let color = match self.zones.get(x, y).unwrap() {
                    ZoneType::None => (0, 0, 0, 0),             // Transparent
                    ZoneType::Residential => (34, 197, 94, 200),  // Green
                    ZoneType::Commercial => (59, 130, 246, 200),  // Blue
                    ZoneType::Industrial => (234, 179, 8, 200),   // Yellow
                    ZoneType::Mixed => (168, 85, 247, 200),       // Purple
                };

                pixels.push(color.0);
                pixels.push(color.1);
                pixels.push(color.2);
                pixels.push(color.3);
            }
        }

        PackedByteArray::from_iter(pixels)
    }
}
