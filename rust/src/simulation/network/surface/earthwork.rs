//! Road-owned earthwork generation, terrain stamping, and structural visibility rules.

use super::{
    ChunkCacheKind, RoadSurfaceSystem, RoadSurfaceVisualNodePiece, RoadSurfaceVisualSpanPiece,
    SurfaceChunkKey,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;

mod boundary;
mod geometry;
mod model;
mod ranges;
mod stamping;

pub(crate) use model::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkGeometryError,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceEarthworkSupportPolicy,
};

// Vertical roadbed offset applied when terrain earthworks need pavement clearance.
pub(super) const EARTHWORK_PAVEMENT_DEPTH_M: f32 = 0.04;

// Lateral terrain probing envelope and sampling cadence for slopes.
const EARTHWORK_MIN_MARGIN_M: f32 = 4.0;
pub(super) const EARTHWORK_MAX_MARGIN_M: f32 = 18.0;
const EARTHWORK_MARGIN_SAMPLE_STEP_M: f32 = 1.0;

// Earthwork slope rates and retaining-wall classification threshold.
const EARTHWORK_CUT_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_FILL_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD: f32 = 1.25;

// Structural end caps that constrain bridge abutments and tunnel portal stamps.
const BRIDGE_ABUTMENT_LENGTH_M: f32 = 12.0;
const TUNNEL_PORTAL_STAMP_DEPTH_M: f32 = 1.0;

impl RoadSurfaceSystem {
    /// Rebuilds terrain earthworks only for the currently dirty road-surface chunks.
    pub fn rebuild_dirty_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
    ) -> Vec<SurfaceChunkKey> {
        let had_dirty_work = !self.compiled_once
            || !self.dirty_edges.is_empty()
            || !self.dirty_nodes.is_empty()
            || !self.dirty_surface_chunks.is_empty()
            || !self.dirty_terrain_chunks.is_empty();
        self.compile_dirty(graph, terrain);

        let chunks = if had_dirty_work {
            self.last_rebuilt_terrain_chunks.clone()
        } else {
            self.collect_all_chunks(ChunkCacheKind::Earthwork)
        };
        self.apply_earthwork_chunks(graph, terrain, &chunks);
        chunks
    }

    /// Rebuilds terrain earthworks for the whole world from the current compiled roadbed cache.
    pub fn rebuild_all_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
    ) -> Vec<SurfaceChunkKey> {
        terrain.reset_visuals_from_source();
        self.compile_dirty(graph, terrain);
        let chunks = self.collect_all_chunks(ChunkCacheKind::Earthwork);
        self.apply_earthwork_chunks(graph, terrain, &chunks);
        chunks
    }

    fn apply_earthwork_chunks(
        &self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
        chunks: &[SurfaceChunkKey],
    ) {
        for &chunk in chunks {
            let (chunk_min, chunk_max) = self.chunk_bounds(chunk);
            terrain.reset_visual_region_from_source_world(
                chunk_min.x,
                chunk_min.z,
                chunk_max.x,
                chunk_max.z,
            );

            let Some(entry) = self.earthwork_chunk_cache.get(&chunk) else {
                continue;
            };

            for &edge_idx in &entry.edge_indices {
                let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                    continue;
                };
                self.stamp_visual_span_piece_earthworks_for_chunk(piece, chunk, terrain);
            }

            for &node_id in &entry.node_ids {
                if node_id as usize >= graph.node_count() {
                    continue;
                }
                let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                    continue;
                };
                self.stamp_visual_node_piece_earthworks_for_chunk(
                    graph, node_id, piece, chunk, terrain,
                );
            }
        }
    }

    fn stamp_visual_span_piece_earthworks_for_chunk(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        if !self.span_piece_uses_visible_earthwork(piece) {
            return;
        }

        let height_offset_m = self.span_piece_integrated_surface_offset_m(piece);
        self.stamp_span_top_surface_support_for_chunk(
            &piece.span_earthwork_support_regions,
            chunk,
            terrain,
            height_offset_m,
        );
    }

    fn stamp_visual_node_piece_earthworks_for_chunk(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        if !self.node_piece_uses_earthworks(graph, node_id, terrain) {
            return;
        }
        if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            return;
        }

        let height_offset_m = self.node_piece_integrated_surface_offset_m(graph, node_id, terrain);
        self.stamp_piece_top_surface_clearance_for_chunk(
            &piece.road_surface_polygons,
            &piece.curb_surface_polygons,
            &piece.sidewalk_surface_polygons,
            chunk,
            terrain,
            height_offset_m,
        );
    }
}
