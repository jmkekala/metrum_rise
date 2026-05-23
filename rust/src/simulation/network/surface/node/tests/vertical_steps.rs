//! Node vertical-step export tests.

use super::*;

#[test]
fn vertical_step_export_uses_exact_canonical_arrangement_keys() {
    let (arrangement, segments) =
        arrangement_with_vertical_step_support(RoadVec2::new(0.0, 0.0), RoadVec2::new(2.0, 0.0));

    let faces =
        RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

    assert_eq!(segments.len(), 1);
    assert_eq!(faces.len(), 1);
}

#[test]
fn vertical_step_export_rejects_unsplit_arrangement_edge_endpoint() {
    let lower_owner = owner(RoadSurfaceBandKind::Carriageway, 0);
    let raised_owner = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_height = height_field(lower_owner);
    let raised_height = height_field(raised_owner);
    let start = RoadVec2::new(0.0, 0.0);
    let mid = RoadVec2::new(1.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let seam = raised_step_seam(lower_owner, raised_owner, start, end);
    let mut arrangement = NodeArrangement::new(91, RoadSurfaceVisualNodePieceKind::Bend);

    let lower_start = arrangement
        .insert_vertex(start, 0.0, [lower_owner], lower_height, [])
        .expect("lower start vertex is valid");
    let lower_end = arrangement
        .insert_vertex(end, 0.0, [lower_owner], lower_height, [])
        .expect("lower end vertex is valid");
    let lower_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, -1.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower apex vertex is valid");
    let lower_edge = arrangement.push_edge(
        lower_start,
        lower_end,
        lower_owner,
        lower_height,
        Some(raised_owner),
        Some(raised_height),
        false,
        false,
        true,
        NodeSeamSource::RaisedStepContact {
            owner_index: raised_owner.owner_index(),
        },
        vec![seam.constraint_index],
    );
    let lower_region = arrangement.push_region(
        lower_owner,
        lower_height,
        vec![lower_start, lower_end, lower_apex],
        Vec::new(),
        vec![lower_edge],
        1.0,
        vec![seam.clone()],
    );
    arrangement.push_face(
        lower_region,
        lower_owner,
        [lower_start, lower_end, lower_apex],
    );

    let raised_start = arrangement
        .insert_vertex(start, 0.12, [raised_owner], raised_height, [])
        .expect("raised start vertex is valid");
    let raised_mid = arrangement
        .insert_vertex(mid, 0.12, [raised_owner], raised_height, [])
        .expect("raised mid vertex is valid");
    let raised_end = arrangement
        .insert_vertex(end, 0.12, [raised_owner], raised_height, [])
        .expect("raised end vertex is valid");
    let raised_left_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 1.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised left apex vertex is valid");
    let raised_right_apex = arrangement
        .insert_vertex(
            RoadVec2::new(2.0, 1.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised right apex vertex is valid");
    let raised_region = arrangement.push_region(
        raised_owner,
        raised_height,
        vec![
            raised_start,
            raised_mid,
            raised_end,
            raised_right_apex,
            raised_left_apex,
        ],
        Vec::new(),
        Vec::new(),
        1.0,
        vec![seam],
    );
    arrangement.push_face(
        raised_region,
        raised_owner,
        [raised_start, raised_mid, raised_left_apex],
    );
    arrangement.push_face(
        raised_region,
        raised_owner,
        [raised_mid, raised_end, raised_right_apex],
    );

    let segments = arrangement.explicit_vertical_step_segments();
    let faces =
        RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

    assert_eq!(segments.len(), 1);
    assert!(
        faces.is_empty(),
        "a split on one owner must not synthesize a midpoint on the other owner's unsplit edge"
    );
}

#[test]
fn vertical_step_export_uses_generic_curb_sidewalk_owner_pair() {
    let (arrangement, segments) = arrangement_with_owner_pair_vertical_step_support(
        RoadSurfaceBandKind::CurbOrShoulder,
        RoadSurfaceBandKind::Sidewalk,
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(2.0, 0.0),
    );

    let faces =
        RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

    assert_eq!(segments.len(), 1);
    assert_eq!(faces.len(), 1);
}

#[test]
fn vertical_step_export_requires_raised_owner_height_at_both_endpoints() {
    let (arrangement, segments) = arrangement_with_owner_pair_vertical_step_support_and_heights(
        RoadSurfaceBandKind::Carriageway,
        RoadSurfaceBandKind::CurbOrShoulder,
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(2.0, 0.0),
        0.0,
        0.0,
        0.12,
        -0.12,
    );

    let faces =
        RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

    assert_eq!(segments.len(), 1);
    assert!(
        faces.is_empty(),
        "material-rank raised-step authority must not emit an inverted physical step"
    );
}

#[test]
fn vertical_step_export_requires_arrangement_boundary_interval_support() {
    let lower_owner = owner(RoadSurfaceBandKind::Carriageway, 0);
    let raised_owner = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_height = height_field(lower_owner);
    let raised_height = height_field(raised_owner);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let seam = raised_step_seam(lower_owner, raised_owner, start, end);
    let mut arrangement = NodeArrangement::new(87, RoadSurfaceVisualNodePieceKind::Bend);

    let lower_start = arrangement
        .insert_vertex(start, 0.0, [lower_owner], lower_height, [])
        .expect("lower start vertex is valid");
    let lower_end = arrangement
        .insert_vertex(end, 0.0, [lower_owner], lower_height, [])
        .expect("lower end vertex is valid");
    let lower_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, -1.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower apex vertex is valid");
    let lower_edge = arrangement.push_edge(
        lower_start,
        lower_end,
        lower_owner,
        lower_height,
        Some(raised_owner),
        Some(raised_height),
        false,
        false,
        true,
        NodeSeamSource::RaisedStepContact {
            owner_index: raised_owner.owner_index(),
        },
        vec![seam.constraint_index],
    );
    let lower_region = arrangement.push_region(
        lower_owner,
        lower_height,
        vec![lower_start, lower_end, lower_apex],
        Vec::new(),
        vec![lower_edge],
        1.0,
        vec![seam.clone()],
    );
    arrangement.push_face(
        lower_region,
        lower_owner,
        [lower_start, lower_end, lower_apex],
    );

    let raised_start = arrangement
        .insert_vertex(start, 0.12, [raised_owner], raised_height, [])
        .expect("raised start vertex is valid");
    let raised_end = arrangement
        .insert_vertex(end, 0.12, [raised_owner], raised_height, [])
        .expect("raised end vertex is valid");
    let raised_start_left = arrangement
        .insert_vertex(
            RoadVec2::new(-0.2, 1.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised start left vertex is valid");
    let raised_start_right = arrangement
        .insert_vertex(
            RoadVec2::new(0.2, 1.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised start right vertex is valid");
    let raised_end_left = arrangement
        .insert_vertex(
            RoadVec2::new(1.8, 1.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised end left vertex is valid");
    let raised_end_right = arrangement
        .insert_vertex(
            RoadVec2::new(2.2, 1.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised end right vertex is valid");
    let raised_region = arrangement.push_region(
        raised_owner,
        raised_height,
        vec![
            raised_start,
            raised_start_left,
            raised_start_right,
            raised_end_left,
            raised_end_right,
            raised_end,
        ],
        Vec::new(),
        Vec::new(),
        1.0,
        vec![seam],
    );
    arrangement.push_face(
        raised_region,
        raised_owner,
        [raised_start, raised_start_left, raised_start_right],
    );
    arrangement.push_face(
        raised_region,
        raised_owner,
        [raised_end, raised_end_left, raised_end_right],
    );

    let segments = arrangement.explicit_vertical_step_segments();
    let faces =
        RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

    assert_eq!(segments.len(), 1);
    assert!(
        faces.is_empty(),
        "endpoint presence alone must not synthesize a vertical face without arrangement boundary interval support"
    );
}

#[test]
fn vertical_step_export_does_not_use_overlay_sibling_support() {
    let (arrangement, segments) = arrangement_with_vertical_step_support(
        RoadVec2::new(0.0, 0.000001),
        RoadVec2::new(2.0, 0.000001),
    );

    let faces =
        RoadSurfaceSystem::raised_step_face_polygons_from_arrangement(&arrangement, &segments);

    assert_eq!(segments.len(), 1);
    assert!(
        faces.is_empty(),
        "overlay-neighbor support must not synthesize a vertical face"
    );
}
