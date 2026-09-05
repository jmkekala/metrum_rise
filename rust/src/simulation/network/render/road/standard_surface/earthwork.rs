// SPDX-License-Identifier: GPL-2.0-only

//! Emission of visible slope and retaining-wall faces owned by compiled road pieces.

use super::super::{MeshLayer, NetworkMeshData, concrete_color, earthwork_color};
use super::coverage::CompiledSurfaceCoverage;
use super::geometry::emit_surface_polygon;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::{
    RoadSurfaceEarthworkFaceKind, RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem,
    RoadSurfaceVisualNodePiece,
};
use crate::simulation::terrain::TerrainSystem;

pub(super) fn emit_compiled_earthwork_mesh(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    for &edge_idx in &coverage.edge_indices {
        let Some(piece) = road_surface.compiled_visual_span_pieces().get(&edge_idx) else {
            continue;
        };
        emit_structural_span_earthwork_faces(mesh, road_surface, &piece.render_earthwork_faces);
    }

    for &node_id in &coverage.node_ids {
        let Some(piece) = road_surface.compiled_visual_node_pieces().get(&node_id) else {
            continue;
        };
        if road_surface.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            emit_structural_node_earthwork_faces(
                mesh,
                graph,
                road_surface,
                terrain,
                node_id,
                piece,
            );
        }
    }
}

fn emit_structural_earthwork_face(
    mesh: &mut NetworkMeshData,
    face: &RoadSurfaceEarthworkRenderFace,
) {
    match face.kind {
        RoadSurfaceEarthworkFaceKind::Slope => {
            emit_surface_polygon(mesh, MeshLayer::Earthwork, &face.polygon, earthwork_color());
        }
        RoadSurfaceEarthworkFaceKind::RetainingWall => {
            emit_surface_polygon(mesh, MeshLayer::Concrete, &face.polygon, concrete_color());
        }
    }
}

fn emit_structural_span_earthwork_faces(
    mesh: &mut NetworkMeshData,
    road_surface: &RoadSurfaceSystem,
    faces: &[RoadSurfaceEarthworkRenderFace],
) {
    for face in faces {
        if !road_surface.span_earthwork_face_uses_visible_earthwork(face) {
            continue;
        }
        emit_structural_earthwork_face(mesh, face);
    }
}

fn emit_structural_node_earthwork_faces(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    node_id: u32,
    piece: &RoadSurfaceVisualNodePiece,
) {
    for face in &piece.render_earthwork_faces {
        if !road_surface
            .node_earthwork_face_uses_visible_earthwork(graph, terrain, node_id, piece, face)
        {
            continue;
        }
        emit_structural_earthwork_face(mesh, face);
    }
}
