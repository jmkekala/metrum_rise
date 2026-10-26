// SPDX-License-Identifier: GPL-2.0-only

//! Terrain-specific spatial queries (raycasting, height sampling).

use crate::config::HEIGHT_SCALE;
use crate::nodes::sim::core::SimCore;
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch};
use godot::prelude::*;
use std::cell::RefCell;
mod engineered;

thread_local! {
    // One graded query scratch per thread (`TERRAIN-07-F`): the snapshot and render workers and
    // the simulation thread keep theirs warm, so a surface query allocates nothing.
    static SURFACE_SCRATCH: RefCell<QueryScratch> = RefCell::default();
}

impl SimCore {
    /// Returns the heightmap dimensions in terrain samples as a Godot Vector2.
    pub fn get_heightmap_size_internal(&self) -> Vector2 {
        Vector2::new(self.heightmap.width as f32, self.heightmap.height as f32)
    }

    /// Returns the terrain height at the given world position.
    pub fn get_height_at_internal(&self, pos: Vector2) -> f32 {
        self.heightmap.sample_height_world(pos.x, pos.y) * HEIGHT_SCALE
    }

    /// The graded surface (`TERRAIN-07`) over the heightmap, the roadbed and the building sites:
    /// the read API of every surface consumer. The site index must be ready
    /// (`BuildingAllocator::prepare_building_site_query_index`) while it is queried.
    pub(crate) fn graded_surface(&self) -> GradedSurface<'_> {
        GradedSurface {
            terrain: &self.heightmap,
            roadbed: &self.transit_network.roadbed,
            sites: &self.allocator,
        }
    }

    /// Runs `query` on the graded surface with this thread's reused query scratch.
    pub(crate) fn with_graded_surface<T>(
        &self,
        query: impl FnOnce(&GradedSurface<'_>, &mut QueryScratch) -> T,
    ) -> T {
        let surface = self.graded_surface();
        SURFACE_SCRATCH.with_borrow_mut(|scratch| query(&surface, scratch))
    }

    /// Returns the visible world-surface height at the given world position: the road's where a
    /// road covers it, else the graded ground the terrain renders
    /// ([`GradedSurface::surface_height_at`], `TERRAIN-07-F`), a pad's support height inside its
    /// footprint. The site index must be ready.
    pub fn get_world_surface_height_internal(&self, pos: Vector2) -> f32 {
        self.with_graded_surface(|surface, scratch| {
            surface.surface_height_at(f64::from(pos.x), f64::from(pos.y), scratch) as f32
        })
    }

    /// A quad's corners (a zoning cell or parcel) on the visible surface, `lift` metres above it:
    /// one gather of the graded surface around the corners, then each corner's exact
    /// [`Self::get_world_surface_height_internal`]. The site index must be ready.
    pub(crate) fn graded_surface_corners(&self, corners: [Vector2; 4], lift: f32) -> [Vector3; 4] {
        let centre = corners.iter().fold(Vector2::ZERO, |sum, &p| sum + p) * 0.25;
        let reach = corners
            .iter()
            .map(|p| (p.x - centre.x).abs().max((p.y - centre.y).abs()))
            .fold(0.0f32, f32::max);
        self.with_graded_surface(|surface, scratch| {
            let (x, z) = (f64::from(centre.x), f64::from(centre.y));
            // The slack covers the f32 rounding of the reach; a corner outside the gather would
            // still be answered exactly, by its own query.
            surface.gather_near(x, z, f64::from(reach) + 0.01, scratch);
            corners.map(|p| {
                let height = surface.surface_height_near(f64::from(p.x), f64::from(p.y), scratch);
                Vector3::new(p.x, height as f32 + lift, p.y)
            })
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
        let road_hit = self.transit_network.roadbed.raycast(ray_origin, ray_dir);
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
                    .roadbed
                    .height_at(pos.x, pos.y)
                    .is_none()
                    && self.allocator.sample_building_site_height(pos).is_none()
                    && self.sample_engineered_ground_height(pos).is_none()
            });
        let owned_hit = closest_ray_hit(ray_origin, ray_dir, road_hit, site_hit);
        let owned_hit = closest_ray_hit(
            ray_origin,
            ray_dir,
            owned_hit,
            self.raycast_engineered_ground(ray_origin, ray_dir),
        );
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
