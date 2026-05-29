//! Terrain-specific spatial queries (raycasting, height sampling).

use crate::config::HEIGHT_SCALE;
use crate::nodes::sim::core::SimCore;
use godot::prelude::*;

impl SimCore {
    /// Returns the heightmap dimensions in terrain samples as a Godot Vector2.
    pub fn get_heightmap_size_internal(&self) -> Vector2 {
        Vector2::new(self.heightmap.width as f32, self.heightmap.height as f32)
    }

    /// Returns the terrain world extent in metres.
    pub fn get_terrain_world_size_internal(&self) -> Vector2 {
        let (world_w, world_h) = self.heightmap.world_size();
        Vector2::new(world_w, world_h)
    }

    /// Returns the terrain height at the given world position.
    pub fn get_height_at_internal(&self, pos: Vector2) -> f32 {
        self.heightmap.sample_height_world(pos.x, pos.y) * HEIGHT_SCALE
    }

    /// Returns the visible world-surface height at the given world position.
    ///
    /// This reads the already compiled roadbed surface when it owns the queried point and otherwise
    /// falls back to the current visual terrain buffer. Query paths must not compile road geometry.
    pub fn get_world_surface_height_internal(&self, pos: Vector2) -> f32 {
        self.transit_network
            .road_surface
            .sample_visible_surface_height(&self.region_graph, &self.heightmap, pos.x, pos.y)
            .unwrap_or_else(|| {
                self.heightmap.sample_visual_height_world(pos.x, pos.y) * HEIGHT_SCALE
            })
    }

    /// Raycasts against the terrain heightmap.
    pub fn intersect_terrain_internal(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        self.heightmap.raycast_terrain(ray_origin, ray_dir)
    }

    /// Raycasts against the visible world surface.
    ///
    /// The combined surface uses the already compiled roadbed where road ownership exists and
    /// otherwise falls back to the visual terrain surface. Query paths must not compile road
    /// geometry.
    pub fn intersect_world_surface_internal(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        self.transit_network.road_surface.raycast_visible_surface(
            &self.region_graph,
            &self.heightmap,
            ray_origin,
            ray_dir,
        )
    }
}
