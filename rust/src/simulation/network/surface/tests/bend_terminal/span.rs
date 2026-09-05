// SPDX-License-Identifier: GPL-2.0-only

//! Span visual-piece coverage tests.

use super::*;

#[test]
fn span_visual_pieces_compile_explicit_band_polygons() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .unwrap();
    assert!(!span_piece.outer_boundary_loops.is_empty());
    assert!(!span_piece.road_surface_polygons.is_empty());
    assert!(!span_piece.curb_surface_polygons.is_empty());
    assert!(!span_piece.raised_step_face_polygons.is_empty());
    assert!(!span_piece.sidewalk_surface_polygons.is_empty());
    assert!(!span_piece.span_owned_regions.is_empty());
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::Asphalt)
            .count(),
        span_piece.road_surface_polygons.len()
    );
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::CurbOrShoulder)
            .count(),
        span_piece.curb_surface_polygons.len()
    );
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::NonRoad)
            .count(),
        span_piece.sidewalk_surface_polygons.len()
    );
    assert!(
        span_piece.span_owned_regions.iter().all(|region| {
            region.edge_idx == edge_idx
                && region.end_section_index == region.start_section_index + 1
                && region.end_s_m > region.start_s_m
        }),
        "span owned regions must preserve edge, section interval, and solved section authority"
    );
    assert!(!span_piece.span_earthwork_support_regions.is_empty());
    assert_eq!(
        span_piece.span_earthwork_support_regions.len(),
        span_piece.span_owned_regions.len(),
        "grounded standard span support regions should cover the same solved band-owned footprint as the visible span"
    );
    assert!(
        std::sync::Arc::ptr_eq(
            &span_piece.span_owned_regions,
            &span_piece.span_earthwork_support_regions
        ),
        "identical grounded visible/earthwork ranges must share one immutable region solve"
    );
    for role in [
        RoadSurfaceSpanRegionRole::Asphalt,
        RoadSurfaceSpanRegionRole::CurbOrShoulder,
        RoadSurfaceSpanRegionRole::NonRoad,
    ] {
        assert!(
            span_piece
                .span_earthwork_support_regions
                .iter()
                .any(|region| region.role == role),
            "span earthwork support regions must retain role/material provenance for {role:?}"
        );
    }
    assert!(
        span_piece
            .span_earthwork_support_regions
            .iter()
            .all(|region| {
                region.edge_idx == edge_idx
                    && region.end_section_index == region.start_section_index + 1
                    && region.end_s_m > region.start_s_m
                    && RoadSurfaceSystem::polygon_has_area_xz(&region.polygon.points_world)
            }),
        "span earthwork support regions must preserve edge, section interval, source band, and top-surface geometry"
    );
    assert_eq!(
        span_piece.span_raised_step_sources.len(),
        span_piece.raised_step_face_polygons.len()
    );
    assert!(
        span_piece.span_raised_step_sources.iter().all(|source| {
            source.lower_owner.kind != source.raised_owner.kind
                && source.end_section_index == source.start_section_index + 1
                && source.end_s_m > source.start_s_m
                && source.start_raised_world.y > source.start_lower_world.y
                && source.end_raised_world.y > source.end_lower_world.y
        }),
        "span raised-step faces must carry owner-pair and solved section provenance"
    );
    assert!(
        span_piece
            .road_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .curb_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece.curb_surface_polygons.iter().all(|polygon| {
            polygon.triangles_world.iter().all(|triangle| {
                let min_y = triangle[0].y.min(triangle[1].y).min(triangle[2].y);
                let max_y = triangle[0].y.max(triangle[1].y).max(triangle[2].y);
                max_y - min_y <= 0.001
            })
        }),
        "curb top surface must be flat; vertical drop belongs to explicit raised-step faces"
    );
    assert!(
        span_piece
            .raised_step_face_polygons
            .iter()
            .all(|polygon| !RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .sidewalk_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(!span_piece.earthwork_surface_polygons.is_empty());
    assert!(!span_piece.earthwork_outer_boundary_loops.is_empty());
    assert!(!span_piece.render_earthwork_faces.is_empty());
    assert_span_earthwork_faces_have_support_provenance(span_piece, edge_idx, EdgeClass::Standard);
    assert!(
        span_piece
            .earthwork_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .render_earthwork_faces
            .iter()
            .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
    );
    assert_ne!(
        span_piece.earthwork_outer_boundary_loops,
        span_piece.outer_boundary_loops
    );
}
