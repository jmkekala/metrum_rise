// SPDX-License-Identifier: GPL-2.0-only

//! Terrain-specific spatial queries (raycasting, height sampling).

use crate::config::HEIGHT_SCALE;
use crate::nodes::sim::core::SimCore;
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch};
use godot::prelude::*;
use std::cell::RefCell;
#[cfg(test)]
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
            query_index: Some(&self.terrain_query_index),
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
    /// Returns the nearer of the compiled roadbed and the graded ground the renderer draws. The
    /// site index must be ready; query paths do not compile geometry.
    pub fn intersect_world_surface_internal(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        let origin = [ray_origin.x, ray_origin.y, ray_origin.z].map(f64::from);
        let direction = [ray_dir.x, ray_dir.y, ray_dir.z].map(f64::from);
        self.with_graded_surface(|surface, scratch| {
            surface
                .raycast_surface(origin, direction, scratch)
                .map(|[x, y, z]| Vector3::new(x as f32, y as f32, z as f32))
        })
    }
}
