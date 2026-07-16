//! Emission of compiled road, curb, sidewalk, and raised-step top surfaces.

use super::super::{MeshLayer, NetworkMeshData, curb_color, road_color, sidewalk_color};
use super::bridge::emit_compiled_bridge_concrete;
use super::coverage::CompiledSurfaceCoverage;
use super::earthwork::emit_compiled_earthwork_mesh;
use super::geometry::{
    emit_node_top_surface_polygons, emit_surface_polygon, emit_vertical_surface_polygon,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;

pub(in crate::simulation::network::render::road) fn emit_compiled_surface_mesh(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    emit_compiled_earthwork_mesh(mesh, graph, road_surface, terrain, coverage);

    for &edge_idx in &coverage.edge_indices {
        let edge = graph.edge(edge_idx);
        let Some(piece) = road_surface.compiled_visual_span_pieces().get(&edge_idx) else {
            continue;
        };
        for polygon in &piece.curb_surface_polygons {
            emit_surface_polygon(mesh, MeshLayer::Curb, polygon, curb_color());
        }
        for polygon in &piece.raised_step_face_polygons {
            emit_vertical_surface_polygon(mesh, polygon, curb_color());
        }
        for polygon in &piece.sidewalk_surface_polygons {
            emit_surface_polygon(mesh, MeshLayer::Sidewalk, polygon, sidewalk_color());
        }
        for polygon in &piece.road_surface_polygons {
            emit_surface_polygon(mesh, MeshLayer::Road, polygon, road_color());
        }
        if edge.class == EdgeClass::Bridge {
            let Some(sections) = road_surface.compiled_sections().get(&edge_idx) else {
                continue;
            };
            for (start_index, end_index) in
                road_surface.visible_section_ranges_for_edge(graph, terrain, edge_idx, sections)
            {
                if end_index <= start_index {
                    continue;
                }
                emit_compiled_bridge_concrete(mesh, terrain, &sections[start_index..=end_index]);
            }
        }
    }

    for &node_id in &coverage.node_ids {
        let Some(piece) = road_surface.compiled_visual_node_pieces().get(&node_id) else {
            continue;
        };
        emit_node_top_surface_polygons(
            mesh,
            MeshLayer::Curb,
            &piece.curb_surface_polygons,
            curb_color(),
        );
        for polygon in &piece.raised_step_face_polygons {
            emit_vertical_surface_polygon(mesh, polygon, curb_color());
        }
        emit_node_top_surface_polygons(
            mesh,
            MeshLayer::Sidewalk,
            &piece.sidewalk_surface_polygons,
            sidewalk_color(),
        );
        emit_node_top_surface_polygons(
            mesh,
            MeshLayer::Road,
            &piece.road_surface_polygons,
            road_color(),
        );
    }
}
