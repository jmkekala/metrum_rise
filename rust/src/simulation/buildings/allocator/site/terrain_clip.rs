// SPDX-License-Identifier: GPL-2.0-only

//! Terrain-patch invalidation and authoritative CDT exclusion loops for building sites.

use super::geometry::site_lot_center;
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
                        lot_center: site_lot_center(site),
                        footprint_world: site.footprint_world.clone(),
                        support_height_m: site.support_height_m,
                        surfaces: site.surfaces.clone(),
                    }
                })
            })
            .collect();
        BuildingSiteTerrainSnapshot {
            sites,
            max_site_radius_m: self.max_site_radius_m,
        }
    }

    /// Returns whether the chunk-index candidates contain a site overlapping the bounds.
    /// Appends the bounds `[min_x, min_z, max_x, max_z]` of every building site overlapping the
    /// bounds, in building order: one candidate query for any number of later overlap tests
    /// (`ROAD-68-D` part 6).
    pub(crate) fn append_building_site_bounds_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        out: &mut Vec<[f32; 4]>,
    ) {
        out.extend(
            self.site_candidate_indices_for_bounds(min_x, min_z, max_x, max_z)
                .into_iter()
                .filter_map(|building_idx| {
                    let bounds = self.building_sites.get(building_idx)?.bounds();
                    overlaps(bounds, (min_x, min_z, max_x, max_z))
                        .then_some(<[f32; 4]>::from(bounds))
                }),
        );
    }

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
    /// The captured sites overlapping the bounds, in capture order: what
    /// [`BuildingAllocator::terrain_site_snapshot_for_world_bounds`] returns for bounds inside
    /// the captured ones, since both keep the overlapping sites in building order.
    pub(crate) fn within_bounds(&self, min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> Self {
        Self {
            sites: self
                .sites
                .iter()
                .filter(|site| site.overlaps_bounds(min_x, min_z, max_x, max_z))
                .cloned()
                .collect(),
            max_site_radius_m: self.max_site_radius_m,
        }
    }

    /// Whether a captured site overlaps the bounds, as
    /// [`BuildingAllocator::has_building_site_for_world_bounds`] answers for bounds inside the
    /// captured ones (`ROAD-68-E`).
    pub(crate) fn has_site_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> bool {
        self.sites
            .iter()
            .any(|site| site.overlaps_bounds(min_x, min_z, max_x, max_z))
    }

    /// Appends the bounds `[min_x, min_z, max_x, max_z]` of every captured site overlapping the
    /// bounds, in site order, as
    /// [`BuildingAllocator::append_building_site_bounds_for_world_bounds`] does for bounds inside
    /// the captured ones (`ROAD-68-E`, `ROAD-68-D` part 6).
    pub(crate) fn append_site_bounds_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        out: &mut Vec<[f32; 4]>,
    ) {
        out.extend(self.sites.iter().filter_map(|site| {
            let bounds = site.bounds();
            overlaps(bounds, (min_x, min_z, max_x, max_z)).then_some(<[f32; 4]>::from(bounds))
        }));
    }

    /// Returns the captured local sites' support pads, in site order.
    pub(crate) fn foundations(&self) -> Vec<super::model::BuildingSiteFoundation> {
        self.sites
            .iter()
            .map(|site| super::model::BuildingSiteFoundation {
                footprint_world: site.footprint_world.clone(),
                support_height_m: site.support_height_m,
            })
            .collect()
    }

    /// Returns ordered paving regions from the captured local sites, including apron-only hits.
    pub(crate) fn paving_surfaces(&self) -> Vec<super::model::BuildingSiteSurfaceClient> {
        self.sites
            .iter()
            .flat_map(|site| site.surfaces.iter().cloned())
            .collect()
    }
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
    fn bounds(&self) -> (f32, f32, f32, f32) {
        super::model::site_surface_bounds(&self.footprint_world, &self.surfaces)
    }

    fn overlaps_bounds(&self, min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> bool {
        overlaps(self.bounds(), (min_x, min_z, max_x, max_z))
    }
}

// Closed axis-aligned overlap of a site's bounds with query bounds, both `(min_x, min_z, max_x,
// max_z)`.
fn overlaps(site: (f32, f32, f32, f32), query: (f32, f32, f32, f32)) -> bool {
    site.0 <= query.2 && site.2 >= query.0 && site.1 <= query.3 && site.3 >= query.1
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
    // The cutout, flat foundation and height query must share exactly one boundary.
    // An inset lets graded terrain rise through the outer strip of the level pad.
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
