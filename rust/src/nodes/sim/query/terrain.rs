//! Terrain-specific spatial queries (raycasting, height sampling).

use crate::config::HEIGHT_SCALE;
use crate::nodes::sim::core::SimCore;
use godot::prelude::*;

impl SimCore {
    /// Returns the heightmap dimensions in terrain samples as a Godot Vector2.
    pub fn get_heightmap_size_internal(&self) -> Vector2 {
        Vector2::new(self.heightmap.width as f32, self.heightmap.height as f32)
    }

    /// Returns the terrain world extent in current gameplay world units.
    pub fn get_terrain_world_size_internal(&self) -> Vector2 {
        let (world_w, world_h) = self.heightmap.world_size();
        Vector2::new(world_w, world_h)
    }

    /// Returns the terrain height at the given world position.
    pub fn get_height_at_internal(&self, pos: Vector2) -> f32 {
        self.heightmap.sample_height_world(pos.x, pos.y) * HEIGHT_SCALE
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
