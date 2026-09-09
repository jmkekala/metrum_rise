// SPDX-License-Identifier: GPL-2.0-only

//! Bounded immutable site dependencies for a road plan's affected terrain patches.

use super::*;
use crate::simulation::buildings::allocator::BuildingSiteTerrainSnapshot;

#[derive(Debug)]
struct PatchSites {
    bounds: (f32, f32, f32, f32),
    snapshot: BuildingSiteTerrainSnapshot,
}

/// Exact local site inputs, collected once under the core lock and compiled after releasing it.
/// Road splits and attachment repair update reference coordinates, never the authored world pose
/// or support height used by site derivation. These are therefore also the post-topology site
/// inputs; their grading is compiled against the final planned road query, not the old network.
#[derive(Debug)]
pub(in crate::nodes::sim::core) struct RoadTerrainSiteInputs {
    patches: HashMap<(usize, usize), PatchSites>,
}

impl RoadTerrainSiteInputs {
    /// Uses the prepared shared index; never rebuilds or copies the resident building collection.
    pub(in crate::nodes::sim::core) fn capture(
        core: &SimCore,
        plan: &RoadEarthworkPlan,
    ) -> Option<Self> {
        if !core.allocator.building_site_query_index_ready() {
            return None;
        }
        let patches = plan
            .clip_patches
            .iter()
            .map(|clip| {
                let patch = &clip.patch;
                let margin = clip.query_margin_m;
                let bounds = (
                    patch.world_origin_x - margin,
                    patch.world_origin_z - margin,
                    patch.world_origin_x + patch.world_size_x + margin,
                    patch.world_origin_z + patch.world_size_z + margin,
                );
                let snapshot = core
                    .allocator
                    .terrain_site_snapshot_for_world_bounds(bounds.0, bounds.1, bounds.2, bounds.3);
                (
                    (patch.patch_x, patch.patch_z),
                    PatchSites { bounds, snapshot },
                )
            })
            .collect();
        Some(Self { patches })
    }

    /// Borrows the immutable contributors captured for one affected render patch.
    pub(super) fn get(&self, x: usize, z: usize) -> Option<&BuildingSiteTerrainSnapshot> {
        self.patches.get(&(x, z)).map(|patch| &patch.snapshot)
    }

    /// A changed broad-phase margin is valid only if its exact site set remains unchanged.
    pub(super) fn matches_bounds(
        &self,
        core: &SimCore,
        key: (usize, usize),
        bounds: (f32, f32, f32, f32),
    ) -> bool {
        self.patches.get(&key).is_some_and(|patch| {
            core.allocator
                .terrain_site_snapshot_for_world_bounds(bounds.0, bounds.1, bounds.2, bounds.3)
                == patch.snapshot
        })
    }

    /// Exact local comparison, including newly added/removed footprints and ID remaps.
    pub(in crate::nodes::sim::core) fn matches(&self, core: &SimCore) -> bool {
        if !core.allocator.building_site_query_index_ready() {
            return false;
        }
        self.patches.values().all(|patch| {
            let (x0, z0, x1, z1) = patch.bounds;
            core.allocator
                .terrain_site_snapshot_for_world_bounds(x0, z0, x1, z1)
                == patch.snapshot
        })
    }
}
