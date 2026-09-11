// SPDX-License-Identifier: GPL-2.0-only

//! Read-only engineered-ground queries using the existing render-patch and CDT tile grids.

use super::*;
use crate::nodes::sim::core::{
    CachedRefinedTerrainPatch, ROAD_LOCKED_TERRAIN_RENDER_STEP_M, RefinedTerrainPatchCacheKey,
};
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::network::surface::{
    RoadVec2, RoadVec3, ray_xz_interval_for_bounds, road_ray_triangle_intersection_t,
};

impl SimCore {
    fn current_ground_patch(&self, x: usize, z: usize) -> Option<&CachedRefinedTerrainPatch> {
        let key = RefinedTerrainPatchCacheKey {
            patch_x: x,
            patch_z: z,
            render_step_mm: (ROAD_LOCKED_TERRAIN_RENDER_STEP_M * 1000.0).round() as u32,
        };
        self.refined_terrain_patch_cache
            .get(&key)
            .filter(|patch| {
                patch.surface_generation == self.terrain_payload_generation_for_patch(x, z)
                    && patch.mesh_buffers.is_some()
                    && SimulationNode::cached_refined_cdt_failure_label(patch).is_none()
            })
            .map(|patch| patch.as_ref())
    }

    /// Allocation-free point query: at most four patch lookups and the containing fixed CDT tiles.
    pub(crate) fn sample_engineered_ground_height(&self, pos: Vector2) -> Option<f32> {
        let (half_x, half_z) = self.heightmap.half_world_extents();
        if pos.x < -half_x || pos.x > half_x || pos.y < -half_z || pos.y > half_z {
            return None;
        }
        let span =
            self.heightmap.render_patch_interval_cells() as f32 * self.heightmap.cell_size_m();
        let x = ((pos.x + half_x) / span).floor() as usize;
        let z = ((pos.y + half_z) / span).floor() as usize;
        // Include the lower neighbor at exact patch boundaries, where one side can be unrefined.
        for patch_z in z.saturating_sub(1)..=z {
            for patch_x in x.saturating_sub(1)..=x {
                let Some(patch) = self.current_ground_patch(patch_x, patch_z) else {
                    continue;
                };
                for window in &patch.windows {
                    let bounds = window.cdt_patch;
                    if f64::from(pos.x) < bounds.min_x
                        || f64::from(pos.x) > bounds.max_x
                        || f64::from(pos.y) < bounds.min_z
                        || f64::from(pos.y) > bounds.max_z
                    {
                        continue;
                    }
                    if let Some(sample) = window
                        .ground_query
                        .sample_ground_height(RoadVec2::new(f64::from(pos.x), f64::from(pos.y)))
                    {
                        return Some(sample);
                    }
                }
            }
        }
        None
    }

    pub(super) fn raycast_engineered_ground(
        &self,
        origin: Vector3,
        direction: Vector3,
    ) -> Option<Vector3> {
        if !origin.is_finite()
            || !direction.is_finite()
            || direction.length_squared() <= f32::EPSILON
        {
            return None;
        }
        let origin = RoadVec3::new(
            f64::from(origin.x),
            f64::from(origin.y),
            f64::from(origin.z),
        );
        let direction = RoadVec3::new(
            f64::from(direction.x),
            f64::from(direction.y),
            f64::from(direction.z),
        );
        let (half_x, half_z) = self.heightmap.half_world_extents();
        let span = f64::from(
            self.heightmap.render_patch_interval_cells() as f32 * self.heightmap.cell_size_m(),
        );
        let mut closest = f64::INFINITY;
        // Walk rows of the existing patch grid, then only columns intersected by the ray
        // within each row. No city-wide cache/triangle scan or query-time compilation.
        for z in 0..self.heightmap.render_patch_rows() {
            let min_z = -f64::from(half_z) + z as f64 * span;
            let max_z = (min_z + span).min(f64::from(half_z));
            let Some((enter, exit)) = ray_xz_interval_for_bounds(
                origin,
                direction,
                -f64::from(half_x),
                min_z,
                f64::from(half_x),
                max_z,
            ) else {
                continue;
            };
            if enter.max(0.0) > closest {
                continue;
            }
            let start_x = origin.x + direction.x * enter.max(0.0);
            let end_x = if exit.is_finite() {
                origin.x + direction.x * exit.min(closest)
            } else {
                start_x
            };
            let first = (((start_x.min(end_x) + f64::from(half_x)) / span).floor() as usize)
                .saturating_sub(1);
            let last = (((start_x.max(end_x) + f64::from(half_x)) / span).floor() as usize)
                .min(self.heightmap.render_patch_cols() - 1);
            for x in first..=last {
                let Some(patch) = self.current_ground_patch(x, z) else {
                    continue;
                };
                for window in &patch.windows {
                    let b = window.cdt_patch;
                    let Some((enter, _)) = ray_xz_interval_for_bounds(
                        origin, direction, b.min_x, b.min_z, b.max_x, b.max_z,
                    ) else {
                        continue;
                    };
                    if enter.max(0.0) > closest {
                        continue;
                    }
                    let Ok(mesh) = &window.mesh_result else {
                        continue;
                    };
                    for indices in mesh.triangles.iter().chain(&mesh.retaining_wall_triangles) {
                        let triangle = indices.map(|i| {
                            let p = mesh.vertices[i];
                            RoadVec3::new(p.x, f64::from(p.height_m), p.z)
                        });
                        if let Some(t) =
                            road_ray_triangle_intersection_t(triangle, origin, direction)
                        {
                            closest = closest.min(t);
                        }
                    }
                }
            }
        }
        closest.is_finite().then(|| {
            let p = origin + direction * closest;
            Vector3::new(p.x as f32, p.y as f32, p.z as f32)
        })
    }
}
