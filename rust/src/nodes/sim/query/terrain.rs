//! Terrain-specific spatial queries (raycasting, height sampling).

use crate::config::HEIGHT_SCALE;
use crate::nodes::sim::core::SimCore;
use godot::prelude::*;

impl SimCore {
    /// Returns the heightmap dimensions as a Godot Vector2.
    pub fn get_heightmap_size_internal(&self) -> Vector2 {
        Vector2::new(self.heightmap.width as f32, self.heightmap.height as f32)
    }

    /// Returns the terrain height at the given world position.
    pub fn get_height_at_internal(&self, pos: Vector2) -> f32 {
        let size = self.get_heightmap_size_internal();
        let hw = (size.x - 1.0) * 0.5;
        let hh = (size.y - 1.0) * 0.5;
        let gx = pos.x + hw;
        let gz = pos.y + hh;
        self.heightmap.get_height_interpolated(gx, gz) * HEIGHT_SCALE
    }

    /// Raycasts against the terrain heightmap.
    pub fn intersect_terrain_internal(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        self.heightmap.raycast_terrain(ray_origin, ray_dir)
    }
}
