//! Shared chunk and triangle traversal helpers for surface queries.

use super::super::{
    RoadSurfaceSystem, RoadSurfaceVisualNodePiece, RoadSurfaceVisualPolygon,
    RoadSurfaceVisualSpanPiece, SurfaceChunkKey, backend::RoadVec3,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;

const SAMPLE_EPSILON_M: f64 = 0.001;
const SURFACE_MIN_TRIANGLE_DOUBLE_AREA_M2: f64 = 1.0e-8;
const SURFACE_MIN_TRIANGLE_ALTITUDE_M: f64 = 0.01;

impl RoadSurfaceSystem {
    pub(super) fn collect_query_contributors(
        &self,
        min_chunk: SurfaceChunkKey,
        max_chunk: SurfaceChunkKey,
    ) -> (Vec<usize>, Vec<u32>) {
        let mut edge_indices = Vec::new();
        let mut node_ids = Vec::new();
        for cx in (min_chunk.0 - 1)..=(max_chunk.0 + 1) {
            for cz in (min_chunk.1 - 1)..=(max_chunk.1 + 1) {
                let chunk = (cx, cz);
                if let Some(entry) = self.surface_chunk_cache.get(&chunk) {
                    edge_indices.extend(entry.edge_indices.iter().copied());
                    node_ids.extend(entry.node_ids.iter().copied());
                }
                if let Some(entry) = self.earthwork_chunk_cache.get(&chunk) {
                    edge_indices.extend(entry.edge_indices.iter().copied());
                    node_ids.extend(entry.node_ids.iter().copied());
                }
            }
        }

        edge_indices.sort_unstable();
        edge_indices.dedup();
        node_ids.sort_unstable();
        node_ids.dedup();
        (edge_indices, node_ids)
    }

    pub(super) fn visit_visible_span_piece_triangles<F>(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        visitor: &mut F,
    ) where
        F: FnMut([RoadVec3; 3]),
    {
        for region in &piece.span_owned_regions {
            Self::visit_visual_polygon_triangles(&region.polygon, visitor);
        }
    }

    pub(super) fn visit_span_piece_earthwork_triangles<F>(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        visitor: &mut F,
    ) where
        F: FnMut([RoadVec3; 3]),
    {
        for polygon in &piece.earthwork_surface_polygons {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    #[cfg(test)]
    pub(super) fn visit_span_piece_clearance_triangles<F>(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        visitor: &mut F,
    ) where
        F: FnMut([RoadVec3; 3]),
    {
        for region in &piece.span_earthwork_support_regions {
            Self::visit_visual_polygon_triangles(&region.polygon, visitor);
        }
    }

    pub(super) fn visit_visible_node_piece_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        visitor: &mut F,
    ) where
        F: FnMut([RoadVec3; 3]),
    {
        if !self.node_uses_visible_surface(graph, terrain, node_id) {
            return;
        }

        for polygon in piece
            .road_surface_polygons
            .iter()
            .chain(&piece.curb_surface_polygons)
            .chain(&piece.sidewalk_surface_polygons)
        {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    pub(super) fn visit_node_piece_earthwork_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        visitor: &mut F,
    ) where
        F: FnMut([RoadVec3; 3]),
    {
        if !self.node_piece_uses_earthworks(graph, node_id, terrain) {
            return;
        }

        for face in &piece.render_earthwork_faces {
            if !self
                .node_earthwork_face_uses_visible_earthwork(graph, terrain, node_id, piece, face)
            {
                continue;
            }
            Self::visit_visual_polygon_triangles(&face.polygon, visitor);
        }
    }

    pub(super) fn visit_visible_top_surface_query_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_indices: &[usize],
        node_ids: &[u32],
        visitor: &mut F,
    ) where
        F: FnMut([RoadVec3; 3]),
    {
        for &node_id in node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            self.visit_visible_node_piece_triangles(graph, terrain, node_id, piece, visitor);
        }

        for &edge_idx in edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            self.visit_visible_span_piece_triangles(piece, visitor);
        }
    }

    pub(super) fn visit_visible_earthwork_query_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_indices: &[usize],
        node_ids: &[u32],
        visitor: &mut F,
    ) where
        F: FnMut([RoadVec3; 3]),
    {
        for &edge_idx in edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            if !self.span_piece_uses_visible_earthwork(piece) {
                continue;
            }
            self.visit_span_piece_earthwork_triangles(piece, visitor);
        }

        for &node_id in node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
                continue;
            }
            self.visit_node_piece_earthwork_triangles(graph, terrain, node_id, piece, visitor);
        }
    }

    pub(in crate::simulation::network::surface) fn visit_visual_polygon_triangles<F>(
        polygon: &RoadSurfaceVisualPolygon,
        visitor: &mut F,
    ) where
        F: FnMut([RoadVec3; 3]),
    {
        for &triangle in &polygon.triangles_world {
            if road_triangle_has_area_xz(triangle) {
                visitor(triangle);
            }
        }
    }
}

fn road_triangle_has_area_xz(triangle: [RoadVec3; 3]) -> bool {
    let projected_cross = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
        - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
    if projected_cross.abs() <= SURFACE_MIN_TRIANGLE_DOUBLE_AREA_M2 {
        return false;
    }
    let edge_ab = RoadVec3::new(
        triangle[1].x - triangle[0].x,
        0.0,
        triangle[1].z - triangle[0].z,
    )
    .length();
    let edge_bc = RoadVec3::new(
        triangle[2].x - triangle[1].x,
        0.0,
        triangle[2].z - triangle[1].z,
    )
    .length();
    let edge_ca = RoadVec3::new(
        triangle[0].x - triangle[2].x,
        0.0,
        triangle[0].z - triangle[2].z,
    )
    .length();
    let max_edge_m = edge_ab.max(edge_bc).max(edge_ca);
    projected_cross.abs() / max_edge_m.max(SAMPLE_EPSILON_M) >= SURFACE_MIN_TRIANGLE_ALTITUDE_M
}
