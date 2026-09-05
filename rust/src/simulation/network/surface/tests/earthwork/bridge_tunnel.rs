// SPDX-License-Identifier: GPL-2.0-only

//! Bridge and tunnel earthwork policy tests.

use super::*;

fn assert_structural_terminals_are_span_owned(surface: &RoadSurfaceSystem, start: u32, end: u32) {
    assert!(
        surface.published_generation_matches_source(),
        "span-owned structural terminals must be a complete published generation"
    );
    for node_id in [start, end] {
        assert!(
            !surface.compiled_visual_node_pieces().contains_key(&node_id),
            "bridge/tunnel terminal {node_id} must be omitted by policy before node compilation"
        );
    }
}

#[test]
fn structural_terminal_policy_removes_and_restores_caps_incrementally() {
    let terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert!(surface.compiled_visual_node_pieces().contains_key(&start));
    assert!(surface.compiled_visual_node_pieces().contains_key(&end));

    graph.edge_mut(edge_idx).class = EdgeClass::Bridge;
    surface.mark_edge_dirty(&graph, edge_idx);
    surface.mark_node_dirty(&graph, start);
    surface.mark_node_dirty(&graph, end);
    surface.compile_dirty(&graph, &terrain);
    assert_structural_terminals_are_span_owned(&surface, start, end);

    graph.edge_mut(edge_idx).class = EdgeClass::Standard;
    surface.mark_edge_dirty(&graph, edge_idx);
    surface.mark_node_dirty(&graph, start);
    surface.mark_node_dirty(&graph, end);
    surface.compile_dirty(&graph, &terrain);
    assert!(surface.published_generation_matches_source());
    assert!(surface.compiled_visual_node_pieces().contains_key(&start));
    assert!(surface.compiled_visual_node_pieces().contains_key(&end));
}

#[test]
fn bridge_earthworks_do_not_stamp_terrain() {
    let mut terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 6.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 6.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-24.0, 6.0, 0.0),
            Vector3::new(0.0, 6.0, 0.0),
            Vector3::new(24.0, 6.0, 0.0),
        ],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("bridge span should compile");
    assert_structural_terminals_are_span_owned(&surface, start, end);
    assert!(span_piece.span_earthwork_support_regions.is_empty());
    assert!(span_piece.earthwork_surface_polygons.is_empty());
    assert!(span_piece.render_earthwork_faces.is_empty());

    let span_center = terrain.sample_visual_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let start_terminal =
        terrain.sample_visual_height_world(-20.0, 0.0) * crate::config::HEIGHT_SCALE;
    let end_terminal = terrain.sample_visual_height_world(20.0, 0.0) * crate::config::HEIGHT_SCALE;
    assert!(span_center.abs() <= 0.01);
    assert!(start_terminal.abs() <= 0.01);
    assert!(end_terminal.abs() <= 0.01);
}

#[test]
fn bridge_ramp_earthworks_do_not_raise_elevated_terminal_terrain() {
    let mut terrain = flat_terrain(129, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-48.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(48.0, 7.5, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(-48.0, 0.0, 0.0), Vector3::new(48.0, 7.5, 0.0)],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("bridge ramp span should compile");
    assert_structural_terminals_are_span_owned(&surface, start, end);

    assert!(
        !span_piece.span_earthwork_support_regions.is_empty(),
        "the grounded end of a bridge ramp must own a terrain-clipped abutment"
    );
    assert!(
        !span_piece.terrain_clip_boundary_loops.is_empty(),
        "the bridge abutment must remove coplanar terrain beneath the deck"
    );
    assert!(span_piece.render_earthwork_faces.is_empty());
    for (x, expected) in [(-48.0, 0.0), (0.0, 0.0), (48.0, 0.0)] {
        let visual_height =
            terrain.sample_visual_height_world(x, 0.0) * crate::config::HEIGHT_SCALE;
        assert!(
            (visual_height - expected).abs() <= 0.01,
            "bridge ramp must not raise visual terrain at x={x}: {visual_height:.3}"
        );
    }
}

#[test]
fn tunnel_earthworks_only_stamp_portals() {
    let mut terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-24.0, 0.0, 0.0),
            Vector3::new(-10.0, -6.0, 0.0),
            Vector3::new(10.0, -6.0, 0.0),
            Vector3::new(24.0, 0.0, 0.0),
        ],
        10.0,
        EdgeClass::Tunnel,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("tunnel span should compile");
    assert_structural_terminals_are_span_owned(&surface, start, end);
    assert!(!span_piece.span_earthwork_support_regions.is_empty());
    assert_span_earthwork_faces_have_support_provenance(span_piece, edge_idx, EdgeClass::Tunnel);
    assert!(
        span_piece
            .span_earthwork_support_regions
            .iter()
            .all(|region| !(region.start_s_m < 24.0 && region.end_s_m > 24.0)),
        "tunnel support regions must stay at visible portals instead of owning buried midspan terrain"
    );
    assert!(
        span_piece.render_earthwork_faces.iter().all(|face| {
            let RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                start_s_m,
                end_s_m,
                support_policy,
                ..
            } = face.source
            else {
                return false;
            };
            support_policy == RoadSurfaceEarthworkSupportPolicy::TunnelVisiblePortals
                && !(start_s_m < 24.0 && end_s_m > 24.0)
        }),
        "tunnel earthwork faces must preserve visible portal support provenance"
    );

    let center = terrain.sample_visual_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let portal = terrain.sample_visual_height_world(-20.0, 0.0) * crate::config::HEIGHT_SCALE;
    assert!(center.abs() <= 0.01);
    assert!(portal <= -0.1);
}
