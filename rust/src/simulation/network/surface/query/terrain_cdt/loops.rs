// SPDX-License-Identifier: GPL-2.0-only

//! Terrain CDT road loop export from road-owned terrain-clip loops.

use super::stable_ids::terrain_cdt_usize_to_u32;
use super::*;

impl RoadSurfaceSystem {
    pub(crate) fn terrain_cdt_road_loops_for_world_bounds(
        &self,
        graph: &RegionGraph,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Result<(Vec<TerrainCdtRoadLoop>, usize), RoadSurfaceTerrainClipExportError> {
        let boundary_loops =
            self.terrain_clip_boundary_loops_for_world_bounds(graph, min_x, min_z, max_x, max_z);
        let source_count = boundary_loops.len();
        let export = Self::union_terrain_clip_boundary_refs_export(&boundary_loops)?;
        let footprint_group_ids =
            Self::terrain_cdt_stable_footprint_group_ids_for_terrain_clip_export(&export);
        let road_loops = export
            .loops
            .iter()
            .enumerate()
            .map(|(loop_index, boundary_loop)| {
                let topology = export.loop_topologies[loop_index];
                let footprint_group_id = footprint_group_ids
                    .get(&topology.shape_index)
                    .copied()
                    .expect("terrain clip export topology must have a stable footprint group id");
                Self::terrain_cdt_road_loop_from_terrain_clip_loop(
                    loop_index,
                    boundary_loop,
                    topology,
                    footprint_group_id,
                )
            })
            .collect::<Vec<_>>();
        Ok((road_loops, source_count))
    }

    fn terrain_cdt_road_loop_from_terrain_clip_loop(
        loop_index: usize,
        boundary_loop: &RoadSurfaceTerrainClipLoop,
        topology: RoadSurfaceTerrainClipLoopTopology,
        footprint_group_id: u64,
    ) -> TerrainCdtRoadLoop {
        let stable_piece_id =
            Self::terrain_cdt_stable_piece_id_for_terrain_clip_loop(boundary_loop, loop_index);
        let vertices = boundary_loop
            .points_world
            .iter()
            .map(|point| TerrainCdtVertex::new(point.x, point.y as f32, point.z))
            .collect::<Vec<_>>();
        let source_edges = boundary_loop
            .source_edges
            .iter()
            .map(|edge| TerrainCdtRoadLoopSourceEdge {
                start: TerrainCdtVertex::new(edge.start.x, edge.start.y as f32, edge.start.z),
                end: TerrainCdtVertex::new(edge.end.x, edge.end.y as f32, edge.end.z),
                source: Self::terrain_cdt_boundary_source_from_surface(edge.source),
            })
            .collect::<Vec<_>>();
        TerrainCdtRoadLoop::new_with_source_edges_and_topology(
            stable_piece_id,
            footprint_group_id,
            terrain_cdt_usize_to_u32(loop_index),
            topology.role == RoadSurfaceTerrainClipContourRole::Hole,
            vertices,
            source_edges,
        )
    }
}
