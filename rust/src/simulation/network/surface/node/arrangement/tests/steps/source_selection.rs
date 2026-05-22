//! Explicit vertical-step source-selection tests.

use super::*;

#[test]
fn explicit_vertical_step_segments_use_authorized_source_pair_on_boundary_edge() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 8);
    let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 7);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 95,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 7 },
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 11,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![test_height_region_with_seams(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![
                height_vertex(0.0, 0.0, 0.0),
                height_vertex(2.0, 0.0, 0.0),
                height_vertex(0.0, 1.0, 0.0),
            ],
            vec![seam],
        )],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("source-authorized boundary edge should produce a canonical arrangement");

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
fn explicit_vertical_step_segments_use_selected_source_pair_on_exposed_boundary() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 9);
    let selected_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let stale_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 13);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let selected_seam = NodeRegionSeamConstraint {
        constraint_index: 103,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 9 },
        owner: Some(carriageway),
        opposite_owner: Some(selected_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let stale_seam = NodeRegionSeamConstraint {
        constraint_index: 416,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 9 },
        owner: Some(carriageway),
        opposite_owner: Some(stale_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 11,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![test_height_region_with_seams(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![
                height_vertex(0.0, 0.0, 0.0),
                height_vertex(2.0, 0.0, 0.0),
                height_vertex(0.0, 1.0, 0.0),
            ],
            vec![selected_seam, stale_seam],
        )],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("selected exposed boundary source should produce a canonical arrangement");

    let segments = arrangement.explicit_vertical_step_segments();
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        carriageway,
        selected_curb,
    )
    .expect("test segment is non-degenerate");
    let stale = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        carriageway,
        stale_curb,
    )
    .expect("test segment is non-degenerate");

    assert!(segments.contains(&expected));
    assert!(!segments.contains(&stale));
}

#[test]
fn explicit_vertical_step_segments_materialize_exposed_endpoint_pair_source() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 14);
    let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let start_contact = NodeRegionSeamConstraint {
        constraint_index: 325,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 14 },
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: start,
    };
    let end_contact = NodeRegionSeamConstraint {
        constraint_index: 327,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 14 },
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: end,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 11,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![test_height_region_with_seams(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![
                height_vertex(0.0, 0.0, 0.0),
                height_vertex(2.0, 0.0, 0.0),
                height_vertex(0.0, 1.0, 0.0),
            ],
            vec![start_contact, end_contact],
        )],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("endpoint step contacts should produce a canonical arrangement");

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
fn explicit_vertical_step_segments_prefer_final_edge_owner_pair_source() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 9);
    let actual_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let stale_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let start = RoadVec2::new(1.0, 0.0);
    let end = RoadVec2::new(1.0, 1.0);
    let actual_seam = NodeRegionSeamConstraint {
        constraint_index: 572,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
        owner: Some(carriageway),
        opposite_owner: Some(actual_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let stale_overlapping_seam = NodeRegionSeamConstraint {
        constraint_index: 96,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 10 },
        owner: Some(carriageway),
        opposite_owner: Some(stale_curb),
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
                vec![actual_seam.clone(), stale_overlapping_seam.clone()],
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                actual_curb,
                vec![
                    height_vertex(1.0, 0.0, 0.12),
                    height_vertex(2.0, 0.0, 0.12),
                    height_vertex(2.0, 1.0, 0.12),
                    height_vertex(1.0, 1.0, 0.12),
                ],
                vec![actual_seam, stale_overlapping_seam],
            ),
        ],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("actual edge owner-pair source should authorize the step");

    let segments = arrangement.explicit_vertical_step_segments();
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        carriageway,
        actual_curb,
    )
    .expect("test segment is non-degenerate");
    let stale = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        carriageway,
        stale_curb,
    )
    .expect("test segment is non-degenerate");

    assert!(segments.contains(&expected));
    assert!(!segments.contains(&stale));
}
