// SPDX-License-Identifier: GPL-2.0-only

//! Preview/committed-surface comparison helpers for road-surface tests.

use super::*;

pub(in crate::simulation::network::surface::tests) fn compile_committed_preview_reference(
    surface: &RoadSurfaceSystem,
    raw_points: &[Vector3],
    terrain: &TerrainSystem,
    fwd_lanes: u8,
    bkw_lanes: u8,
) -> (
    PreviewRoadSurfaceResult,
    Vec<RoadSurfaceSection>,
    Vec<RoadSurfaceVisualNodePiece>,
) {
    let preview = surface.compile_preview_surface(raw_points, fwd_lanes, bkw_lanes, terrain);
    if preview.prepared_points.len() < 2 {
        return (preview, Vec::new(), Vec::new());
    }

    let mut graph = RegionGraph::new();
    let start_node = graph.add_node(preview.prepared_points[0], NodeType::Junction);
    let end_node = graph.add_node(*preview.prepared_points.last().unwrap(), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start_node,
        end_node,
        preview.prepared_points.clone(),
        ((fwd_lanes + bkw_lanes) as f32 * crate::config::LANE_WIDTH).max(2.0),
        preview.edge_class,
        if fwd_lanes == 0 && bkw_lanes == 0 {
            TransitType::Foot
        } else {
            TransitType::Road
        },
        if fwd_lanes == 0 && bkw_lanes == 0 {
            TransitFlags::FOOT
        } else {
            TransitFlags::CAR | TransitFlags::FOOT
        },
    ));

    let (chunk_origin_x_m, chunk_origin_z_m) = surface.chunk_origin_m();
    let mut committed = RoadSurfaceSystem::new_with_chunk_grid(
        surface.chunk_span_m(),
        chunk_origin_x_m,
        chunk_origin_z_m,
    );
    committed.compile_dirty(&graph, terrain);
    let compiled_sections = committed
        .compiled_sections()
        .get(&edge_idx)
        .map(|sections| sections.as_ref().clone())
        .unwrap_or_default();
    let compiled_visual_node_pieces = [start_node, end_node]
        .into_iter()
        .filter_map(|node_id| {
            committed
                .compiled_visual_node_pieces()
                .get(&node_id)
                .map(|piece| piece.as_ref().clone())
        })
        .collect();
    (preview, compiled_sections, compiled_visual_node_pieces)
}
