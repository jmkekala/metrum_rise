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

#[test]
fn explicit_vertical_step_segments_use_endpoint_pair_when_full_edge_sources_are_stale() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 8);
    let actual_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let stale_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 7);
    let start = RoadVec2::new(1.0, 0.0);
    let end = RoadVec2::new(1.0, 1.0);
    let stale_full_edge = NodeRegionSeamConstraint {
        constraint_index: 49,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 8 },
        owner: Some(carriageway),
        opposite_owner: Some(stale_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let actual_start = NodeRegionSeamConstraint {
        constraint_index: 261,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 10 },
        owner: Some(carriageway),
        opposite_owner: Some(actual_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: start,
    };
    let actual_end = NodeRegionSeamConstraint {
        constraint_index: 262,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 10 },
        owner: Some(carriageway),
        opposite_owner: Some(actual_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: end,
        end_xz: end,
    };
    let heights = two_region_height_solution_with_material_heights(
        carriageway,
        actual_curb,
        0.0,
        0.12,
        vec![
            stale_full_edge.clone(),
            actual_start.clone(),
            actual_end.clone(),
        ],
        vec![stale_full_edge, actual_start, actual_end],
    );
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("endpoint owner-pair sources should survive stale full-edge sources");
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        carriageway,
        actual_curb,
    )
    .expect("test segment is non-degenerate");

    assert!(
        arrangement
            .explicit_vertical_step_segments()
            .contains(&expected)
    );
}

#[test]
fn explicit_vertical_step_segments_use_endpoint_material_path_for_final_owner_pair() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 8);
    let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let bridge_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 7);
    let start = RoadVec2::new(1.0, 0.0);
    let end = RoadVec2::new(1.0, 1.0);
    let start_carriageway_to_bridge = NodeRegionSeamConstraint {
        constraint_index: 49,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 8 },
        owner: Some(carriageway),
        opposite_owner: Some(bridge_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: start,
    };
    let start_bridge_to_curb = NodeRegionSeamConstraint {
        constraint_index: 1705,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 10 },
        owner: Some(bridge_curb),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: start,
    };
    let end_direct = NodeRegionSeamConstraint {
        constraint_index: 29,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 10 },
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: end,
        end_xz: end,
    };
    let heights = two_region_height_solution_with_material_heights(
        carriageway,
        curb,
        0.0,
        0.12,
        vec![
            start_carriageway_to_bridge.clone(),
            start_bridge_to_curb.clone(),
            end_direct.clone(),
        ],
        vec![
            start_carriageway_to_bridge,
            start_bridge_to_curb,
            end_direct,
        ],
    );
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("endpoint material path should authorize final owner-pair step segment");
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        carriageway,
        curb,
    )
    .expect("test segment is non-degenerate");

    assert!(
        arrangement
            .explicit_vertical_step_segments()
            .contains(&expected)
    );
}

#[test]
fn explicit_vertical_step_segments_materialize_unpaired_endpoint_path_source() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
    let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 16);
    let bridge_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(1.0, 0.0);
    let start_direct = NodeRegionSeamConstraint {
        constraint_index: 135,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 16 },
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: start,
    };
    let end_curb_to_bridge = NodeRegionSeamConstraint {
        constraint_index: 1697,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 16 },
        owner: Some(bridge_curb),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: end,
        end_xz: end,
    };
    let end_bridge_to_carriageway = NodeRegionSeamConstraint {
        constraint_index: 96,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 2 },
        owner: Some(carriageway),
        opposite_owner: Some(bridge_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: end,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 14,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![test_height_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![
                height_vertex(0.0, 0.0, 0.12),
                height_vertex(1.0, 0.0, 0.12),
                height_vertex(0.0, 1.0, 0.12),
            ],
            vec![start_direct, end_curb_to_bridge, end_bridge_to_carriageway],
        )],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("endpoint material path should authorize unpaired exposed step edge");
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        carriageway,
        curb,
    )
    .expect("test segment is non-degenerate");

    assert!(
        arrangement
            .explicit_vertical_step_segments()
            .contains(&expected)
    );
}

#[test]
fn explicit_vertical_step_segments_select_unique_cross_kind_endpoint_path_source() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
    let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 16);
    let bridge_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(1.0, 0.0);
    let start_curb_to_carriageway = NodeRegionSeamConstraint {
        constraint_index: 135,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 16 },
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: start,
    };
    let start_curb_to_bridge = NodeRegionSeamConstraint {
        constraint_index: 136,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 16 },
        owner: Some(bridge_curb),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: start,
    };
    let end_curb_to_bridge = NodeRegionSeamConstraint {
        constraint_index: 1697,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 16 },
        owner: Some(bridge_curb),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: end,
        end_xz: end,
    };
    let end_bridge_to_carriageway = NodeRegionSeamConstraint {
        constraint_index: 96,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 2 },
        owner: Some(carriageway),
        opposite_owner: Some(bridge_curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: end,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 14,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![test_height_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![
                height_vertex(0.0, 0.0, 0.12),
                height_vertex(1.0, 0.0, 0.12),
                height_vertex(0.0, 1.0, 0.12),
            ],
            vec![
                start_curb_to_carriageway,
                start_curb_to_bridge,
                end_curb_to_bridge,
                end_bridge_to_carriageway,
            ],
        )],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("unique cross-kind endpoint path should authorize the exposed step edge");
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        carriageway,
        curb,
    )
    .expect("test segment is non-degenerate");
    let same_kind_bridge = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        bridge_curb,
        curb,
    )
    .expect("test segment is non-degenerate");
    let segments = arrangement.explicit_vertical_step_segments();

    assert!(segments.contains(&expected));
    assert!(!segments.contains(&same_kind_bridge));
}
