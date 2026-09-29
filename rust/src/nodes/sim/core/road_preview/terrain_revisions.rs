// SPDX-License-Identifier: GPL-2.0-only

//! Display revisions for preview terrain products, so unchanged patches are not re-exported.

use std::collections::HashMap;
use std::sync::Arc;

use super::super::terrain_payloads::{CachedRefinedTerrainPatch, RefinedTerrainPatchCacheKey};

/// The latest compiled footprint's products and their display revisions.
/// Replaced on every assignment, so it holds one footprint, never drag history.
#[derive(Default)]
pub(crate) struct RoadPreviewTerrainRevisions {
    next: u64,
    products: HashMap<RefinedTerrainPatchCacheKey, (Arc<CachedRefinedTerrainPatch>, u64)>,
}

impl RoadPreviewTerrainRevisions {
    /// Keeps a patch's previous revision when its exported display is provably identical,
    /// otherwise issues a fresh one. O(patches + compared plain-patch samples).
    pub(crate) fn assign(
        &mut self,
        patches: &[Arc<CachedRefinedTerrainPatch>],
    ) -> HashMap<RefinedTerrainPatchCacheKey, u64> {
        let mut products = HashMap::with_capacity(patches.len());
        for patch in patches {
            let revision = match self.products.get(&patch.key) {
                Some((previous, revision)) if same_display(previous, patch) => *revision,
                _ => {
                    self.next = self.next.wrapping_add(1).max(1);
                    self.next
                }
            };
            products.insert(patch.key, (Arc::clone(patch), revision));
        }
        self.products = products;
        self.products
            .iter()
            .map(|(key, (_, revision))| (*key, *revision))
            .collect()
    }
}

/// A road-free patch exports only its height snapshot and fixed flags, so equal snapshots
/// display identically. Refined patches rebuild their buffers per request and never match.
fn same_display(previous: &CachedRefinedTerrainPatch, next: &CachedRefinedTerrainPatch) -> bool {
    previous.key == next.key
        && previous.input_road_loops == 0
        && next.input_road_loops == 0
        && previous.patch == next.patch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::terrain::TerrainPatchSnapshot;

    fn product(x: usize, loops: usize, height: f32) -> Arc<CachedRefinedTerrainPatch> {
        Arc::new(CachedRefinedTerrainPatch {
            site_surfaces: Vec::new(),
            key: RefinedTerrainPatchCacheKey {
                patch_x: x,
                patch_z: 0,
                render_step_mm: 1000,
            },
            contract_revision: 0,
            surface_generation: 0,
            patch: TerrainPatchSnapshot {
                patch_x: x,
                patch_z: 0,
                sample_width: 2,
                sample_height: 2,
                texture_width: 2,
                texture_height: 2,
                inner_offset_x: 0,
                inner_offset_z: 0,
                world_origin_x: 0.0,
                world_origin_z: 0.0,
                world_size_x: 1.0,
                world_size_z: 1.0,
                height_data: vec![height; 4],
            },
            input_road_loops: loops,
            input_source_samples: 0,
            windows: Vec::new(),
            mesh_buffers: None,
            requires_engineered_refinement: loops > 0,
            requires_road_clipping: loops > 0,
            clip_source_count: 0,
            road_clip_source_count: 0,
            road_clip_loop_count: 0,
            site_clip_loop_count: 0,
            omitted_margin_clip_loop_count: 0,
            clip_error_label: None,
            clip_query_margin_m: 0.0,
            cdt_ms: 0.0,
            reused_windows: 0,
        })
    }

    #[test]
    fn only_identical_plain_patches_keep_their_revision() {
        let mut revisions = RoadPreviewTerrainRevisions::default();
        let first = revisions.assign(&[product(0, 0, 1.0), product(1, 0, 1.0), product(2, 2, 1.0)]);
        // Fresh Arcs with equal heights: the plain patch keeps its revision, the refined one
        // does not, and a changed height issues a new one.
        let second =
            revisions.assign(&[product(0, 0, 1.0), product(1, 0, 2.0), product(2, 2, 1.0)]);
        let key = |x| RefinedTerrainPatchCacheKey {
            patch_x: x,
            patch_z: 0,
            render_step_mm: 1000,
        };
        assert_eq!(second[&key(0)], first[&key(0)]);
        assert_ne!(second[&key(1)], first[&key(1)]);
        assert_ne!(second[&key(2)], first[&key(2)]);
        let mut all: Vec<_> = first.values().chain(second.values()).copied().collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 5, "fresh revisions are never reused");
        // Only the latest footprint is retained: a dropped patch returns with a new revision.
        revisions.assign(&[product(1, 0, 2.0)]);
        let returned = revisions.assign(&[product(0, 0, 1.0)]);
        assert_ne!(returned[&key(0)], first[&key(0)]);
    }
}
