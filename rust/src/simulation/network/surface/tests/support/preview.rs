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

    let mut committed = RoadSurfaceSystem::new(surface.chunk_span_m());
    committed.compile_dirty(&graph, terrain);
    let compiled_sections = committed
        .compiled_sections()
        .get(&edge_idx)
        .cloned()
        .unwrap_or_default();
    let compiled_visual_node_pieces = [start_node, end_node]
        .into_iter()
        .filter_map(|node_id| {
            committed
                .compiled_visual_node_pieces()
                .get(&node_id)
                .cloned()
        })
        .collect();
    (preview, compiled_sections, compiled_visual_node_pieces)
}

pub(in crate::simulation::network::surface::tests) fn assert_preview_vertices_use_solved_section_height_keys(
    preview: &PreviewRoadSurfaceResult,
) {
    let solved_height_keys = preview
        .compiled_sections
        .iter()
        .flat_map(|section| section.bands.iter())
        .flat_map(|band| {
            [
                SurfaceHeightMmKey::from_m_f32(band.height_start_m),
                SurfaceHeightMmKey::from_m_f32(band.height_end_m),
            ]
        })
        .collect::<BTreeSet<_>>();
    assert!(
        !solved_height_keys.is_empty(),
        "preview height-key regression check requires compiled section bands"
    );
    assert!(
        !preview.surface_vertices.is_empty(),
        "preview height-key regression check requires preview mesh vertices"
    );

    for vertex in &preview.surface_vertices {
        let key = SurfaceHeightMmKey::from_m_f32(vertex.y);
        assert!(
            solved_height_keys.contains(&key),
            "preview mesh vertex height must come from solved section geometry without render lift: y={:.6} key={} solved_keys={:?}",
            vertex.y,
            key.as_i64(),
            solved_height_keys
        );
    }
}
