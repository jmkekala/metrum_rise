// SPDX-License-Identifier: GPL-2.0-only

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
    /// This reads owned road surfaces first, building-site top surfaces second, and the current
    /// visual terrain buffer only when no engineered surface owns the query point.
    pub fn get_world_surface_height_internal(&self, pos: Vector2) -> f32 {
        self.transit_network
            .road_surface
            .sample_visible_surface_height(&self.region_graph, &self.heightmap, pos.x, pos.y)
            .or_else(|| self.allocator.sample_building_site_height(pos))
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
    /// Owned road surfaces and building-site top surfaces hide source/visual terrain below them.
    /// Query paths must not compile geometry.
    pub fn intersect_world_surface_internal(
        &mut self,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        self.allocator
            .prepare_building_site_query_index(self.zoning.config.zone_cell_m);
        let road_hit = self
            .transit_network
            .road_surface
            .raycast_road_visible_surface(&self.region_graph, &self.heightmap, ray_origin, ray_dir);
        let (half_w, half_h) = self.heightmap.half_world_extents();
        let site_hit = self.allocator.raycast_building_site_surface(
            ray_origin,
            ray_dir,
            (-half_w, -half_h, half_w, half_h),
        );
        let terrain_hit = self
            .heightmap
            .raycast_visual_terrain(ray_origin, ray_dir)
            .filter(|hit| {
                let pos = Vector2::new(hit.x, hit.z);
                self.transit_network
                    .road_surface
                    .sample_visible_surface_height(
                        &self.region_graph,
                        &self.heightmap,
                        pos.x,
                        pos.y,
                    )
                    .is_none()
                    && self.allocator.sample_building_site_height(pos).is_none()
            });
        let owned_hit = closest_ray_hit(ray_origin, ray_dir, road_hit, site_hit);
        closest_ray_hit(ray_origin, ray_dir, owned_hit, terrain_hit)
    }
}

fn closest_ray_hit(
    ray_origin: Vector3,
    ray_dir: Vector3,
    left: Option<Vector3>,
    right: Option<Vector3>,
) -> Option<Vector3> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let denom = ray_dir.length_squared().max(f32::EPSILON);
            let left_t = (left - ray_origin).dot(ray_dir) / denom;
            let right_t = (right - ray_origin).dot(ray_dir) / denom;
            if right_t < left_t {
                Some(right)
            } else {
                Some(left)
            }
        }
        (Some(hit), None) | (None, Some(hit)) => Some(hit),
        (None, None) => None,
    }
}
