// SPDX-License-Identifier: GPL-2.0-only

//! Read-only engineered-ground queries using the existing render-patch and CDT tile grids.

use super::*;
use crate::nodes::sim::core::{
    CachedRefinedTerrainPatch, ROAD_LOCKED_TERRAIN_RENDER_STEP_M, RefinedTerrainPatchCacheKey,
};
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::network::roadbed::RoadVec2;

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
}
