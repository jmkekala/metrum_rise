//! Terrain-patch invalidation and authoritative CDT exclusion loops for building sites.

use super::model::BuildingSiteClient;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop, TerrainCdtRoadLoopSourceEdge,
    TerrainCdtVertex,
};

const BUILDING_SITE_FOOTPRINT_GROUP_MASK: u64 = 0x8000_0000_0000_0000;

impl BuildingAllocator {
    pub(crate) fn terrain_render_patch_keys_with_building_site_margin(
        &self,
        terrain: &TerrainSystem,
        margin_m: f32,
    ) -> Vec<(usize, usize)> {
        let margin_m = margin_m.max(0.0);
        let mut keys = Vec::new();
        for site in &self.building_sites {
            let (min_x, min_z, max_x, max_z) = site.bounds();
            keys.extend(terrain.render_patch_keys_for_world_bounds(
                min_x - margin_m,
                min_z - margin_m,
                max_x + margin_m,
                max_z + margin_m,
            ));
        }
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    pub(crate) fn mark_building_site_terrain_bounds_dirty(
        &self,
        terrain: &mut TerrainSystem,
        bounds: (f32, f32, f32, f32),
        margin_m: f32,
    ) {
        let (min_x, min_z, max_x, max_z) = bounds;
        let margin_m = margin_m.max(0.0);
        terrain.mark_render_patches_for_world_bounds(
            min_x - margin_m,
            min_z - margin_m,
            max_x + margin_m,
            max_z + margin_m,
        );
    }

    pub(crate) fn terrain_cdt_site_loops_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<TerrainCdtRoadLoop> {
        let mut loops = Vec::new();
        for building_idx in self.site_candidate_indices_for_bounds(min_x, min_z, max_x, max_z) {
            let Some(site) = self.building_sites.get(building_idx) else {
                continue;
            };
            if !site.overlaps_bounds(min_x, min_z, max_x, max_z) {
                continue;
            }
            loops.push(building_site_cdt_loop(building_idx, site));
        }
        loops
    }
}

fn building_site_cdt_loop(building_idx: usize, site: &BuildingSiteClient) -> TerrainCdtRoadLoop {
    let stable_piece_id = BUILDING_SITE_FOOTPRINT_GROUP_MASK | building_idx as u64;
    let vertices = site
        .footprint_world
        .iter()
        .map(|point| TerrainCdtVertex::new(point.x as f64, site.support_height_m, point.y as f64))
        .collect::<Vec<_>>();
    let source_edges = vertices
        .iter()
        .copied()
        .enumerate()
        .map(|(edge_idx, start)| TerrainCdtRoadLoopSourceEdge {
            start,
            end: vertices[(edge_idx + 1) % vertices.len()],
            source: TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                building_idx: building_idx as u64,
                local_loop_index: 0,
                local_edge_index: u32::try_from(edge_idx).unwrap_or(u32::MAX),
            },
        })
        .collect();
    TerrainCdtRoadLoop::new_with_source_edges_and_topology(
        stable_piece_id,
        stable_piece_id,
        0,
        false,
        vertices,
        source_edges,
    )
}
