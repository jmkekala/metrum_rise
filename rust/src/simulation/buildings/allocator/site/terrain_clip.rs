//! Terrain-patch invalidation and authoritative CDT exclusion loops for building sites.

use super::model::{BuildingSiteClient, BuildingSiteTerrainClient, BuildingSiteTerrainSnapshot};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop, TerrainCdtRoadLoopSourceEdge,
    TerrainCdtVertex,
};

const BUILDING_SITE_FOOTPRINT_GROUP_MASK: u64 = 0x8000_0000_0000_0000;

impl BuildingAllocator {
    pub(crate) fn terrain_site_snapshot_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> BuildingSiteTerrainSnapshot {
        let sites = self
            .site_candidate_indices_for_bounds(min_x, min_z, max_x, max_z)
            .into_iter()
            .filter_map(|building_idx| {
                let site = self.building_sites.get(building_idx)?;
                site.overlaps_bounds(min_x, min_z, max_x, max_z).then(|| {
                    BuildingSiteTerrainClient {
                        building_idx,
                        footprint_world: site.footprint_world.clone(),
                        support_height_m: site.support_height_m,
                    }
                })
            })
            .collect();
        BuildingSiteTerrainSnapshot { sites }
    }

    /// Returns whether the chunk-index candidates contain a site overlapping the bounds.
    pub(crate) fn has_building_site_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> bool {
        self.site_candidate_indices_for_bounds(min_x, min_z, max_x, max_z)
            .into_iter()
            .any(|building_idx| {
                self.building_sites
                    .get(building_idx)
                    .is_some_and(|site| site.overlaps_bounds(min_x, min_z, max_x, max_z))
            })
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

impl BuildingSiteTerrainSnapshot {
    pub(crate) fn terrain_cdt_site_loops_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<TerrainCdtRoadLoop> {
        self.sites
            .iter()
            .filter(|site| site.overlaps_bounds(min_x, min_z, max_x, max_z))
            .map(building_site_snapshot_cdt_loop)
            .collect()
    }
}

impl BuildingSiteTerrainClient {
    fn overlaps_bounds(&self, min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> bool {
        let (site_min_x, site_min_z, site_max_x, site_max_z) =
            super::geometry::polygon_slice_bounds(&self.footprint_world);
        site_min_x <= max_x && site_max_x >= min_x && site_min_z <= max_z && site_max_z >= min_z
    }
}

fn building_site_cdt_loop(building_idx: usize, site: &BuildingSiteClient) -> TerrainCdtRoadLoop {
    building_site_cdt_loop_from_parts(building_idx, &site.footprint_world, site.support_height_m)
}

fn building_site_snapshot_cdt_loop(site: &BuildingSiteTerrainClient) -> TerrainCdtRoadLoop {
    building_site_cdt_loop_from_parts(
        site.building_idx,
        &site.footprint_world,
        site.support_height_m,
    )
}

fn building_site_cdt_loop_from_parts(
    building_idx: usize,
    footprint_world: &[godot::prelude::Vector2],
    support_height_m: f32,
) -> TerrainCdtRoadLoop {
    let stable_piece_id = BUILDING_SITE_FOOTPRINT_GROUP_MASK | building_idx as u64;
    let vertices = footprint_world
        .iter()
        .map(|point| TerrainCdtVertex::new(point.x as f64, support_height_m, point.y as f64))
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
