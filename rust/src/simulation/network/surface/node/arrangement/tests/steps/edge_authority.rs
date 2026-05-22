//! Explicit vertical-step edge-authority tests.

use super::*;

#[test]
fn explicit_vertical_step_segments_use_canonical_edge_owner_pair() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
    let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(1.0, 0.0);
    let end = RoadVec2::new(1.0, 1.0);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 91,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 11,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(1.0, 0.0, 0.0),
                    height_vertex(1.0, 1.0, 0.0),
                    height_vertex(0.0, 1.0, 0.0),
                ],
                vec![seam.clone()],
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![
                    height_vertex(1.0, 0.0, 0.12),
                    height_vertex(2.0, 0.0, 0.12),
                    height_vertex(2.0, 1.0, 0.12),
                    height_vertex(1.0, 1.0, 0.12),
                ],
                vec![seam],
            ),
        ],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("explicit curb step seam should produce a canonical arrangement");

    let segments = arrangement.explicit_vertical_step_segments();
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        carriageway,
        curb,
    )
    .expect("test segment is non-degenerate");

    assert!(segments.contains(&expected));
}

#[test]
fn explicit_vertical_step_segments_do_not_derive_steps_from_face_overlap() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
    let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mut arrangement = NodeArrangement::new(11, RoadSurfaceVisualNodePieceKind::Bend);
    let carriageway_height = height_field_id(RoadSurfaceBandKind::Carriageway, 2);
    let curb_height = height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1);

    let carriageway_region = arrangement.push_region(
        carriageway,
        carriageway_height,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        1.0,
        Vec::new(),
    );
    let curb_region = arrangement.push_region(
        curb,
        curb_height,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        1.0,
        Vec::new(),
    );

    let carriageway_start = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 0.0),
            0.0,
            [carriageway],
            carriageway_height,
            [],
        )
        .expect("test vertex is legal");
    let carriageway_end = arrangement
        .insert_vertex(
            RoadVec2::new(4.0, 0.0),
            0.0,
            [carriageway],
            carriageway_height,
            [],
        )
        .expect("test vertex is legal");
    let carriageway_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 1.0),
            0.0,
            [carriageway],
            carriageway_height,
            [],
        )
        .expect("test vertex is legal");
    arrangement.push_face(
        carriageway_region,
        carriageway,
        [carriageway_start, carriageway_end, carriageway_apex],
    );

    let curb_start = arrangement
        .insert_vertex(RoadVec2::new(1.0, 0.0), 0.12, [curb], curb_height, [])
        .expect("test vertex is legal");
    let curb_end = arrangement
        .insert_vertex(RoadVec2::new(3.0, 0.0), 0.12, [curb], curb_height, [])
        .expect("test vertex is legal");
    let curb_apex = arrangement
        .insert_vertex(RoadVec2::new(1.0, -1.0), 0.12, [curb], curb_height, [])
        .expect("test vertex is legal");
    arrangement.push_face(curb_region, curb, [curb_start, curb_end, curb_apex]);

    let segments = arrangement.explicit_vertical_step_segments();
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
        NodeArrangementKey::from_point(RoadVec2::new(3.0, 0.0)),
        carriageway,
        curb,
    )
    .expect("test segment is non-degenerate");
    let stale_full_edge = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0)),
        NodeArrangementKey::from_point(RoadVec2::new(4.0, 0.0)),
        carriageway,
        curb,
    )
    .expect("test segment is non-degenerate");

    assert!(!segments.contains(&expected));
    assert!(!segments.contains(&stale_full_edge));
}

#[test]
fn explicit_vertical_step_segments_require_source_for_same_kind_junction_edge() {
    let lower = owner(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let raised = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_height = height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let mut arrangement = NodeArrangement::new(11, RoadSurfaceVisualNodePieceKind::JunctionN);
    let start = arrangement
        .insert_vertex(RoadVec2::new(0.0, 0.0), 0.0, [lower], lower_height, [])
        .expect("test vertex is legal");
    let end = arrangement
        .insert_vertex(RoadVec2::new(1.0, 0.0), 0.0, [lower], lower_height, [])
        .expect("test vertex is legal");

    arrangement.push_edge(
        start,
        end,
        lower,
        lower_height,
        Some(raised),
        Some(height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1)),
        false,
        false,
        false,
        NodeSeamSource::RaisedStepContact { owner_index: 1 },
        Vec::new(),
    );

    assert!(
        arrangement.explicit_vertical_step_segments().is_empty(),
        "same-kind JunctionN edges require source-authorized step constraints"
    );
}

#[test]
fn explicit_vertical_step_segments_include_direct_sidewalk_contacts() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
    let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 92,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 2 },
        owner: Some(carriageway),
        opposite_owner: Some(sidewalk),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: RoadVec2::new(1.0, 0.0),
        end_xz: RoadVec2::new(1.0, 1.0),
    };
    let heights = two_region_height_solution_with_material_heights(
        carriageway,
        sidewalk,
        0.0,
        0.12,
        vec![seam.clone()],
        vec![seam],
    );
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("explicit non-road step seam should produce a canonical arrangement");

    let segments = arrangement.explicit_vertical_step_segments();
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 1.0)),
        carriageway,
        sidewalk,
    )
    .expect("test segment is non-degenerate");

    assert!(segments.contains(&expected));
}
