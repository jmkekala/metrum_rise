//! Explicit vertical-step arrangement tests.

use super::*;

#[test]
fn arrangement_rejects_same_material_same_xz_height_conflict_without_explicit_step() {
    let first = owner(RoadSurfaceBandKind::Sidewalk, 0);
    let second = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let heights = NodeHeightSolution {
        node_id: 12,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                first,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(1.0, 0.0, 0.0),
                    height_vertex(1.0, 1.0, 0.0),
                    height_vertex(0.0, 1.0, 0.0),
                ],
                Vec::new(),
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                second,
                vec![
                    height_vertex(1.0, 0.0, 0.5),
                    height_vertex(2.0, 0.0, 0.5),
                    height_vertex(2.0, 1.0, 0.5),
                    height_vertex(1.0, 1.0, 0.5),
                ],
                Vec::new(),
            ),
        ],
    };

    assert!(matches!(
        NodeArrangement::from_height_solution(&heights),
        Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
    ));
}

#[test]
fn arrangement_accepts_same_material_same_xz_height_split_with_explicit_vertical_step() {
    let lower = owner(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let raised = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(1.0, 0.0);
    let end = RoadVec2::new(1.0, 1.0);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 54,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
        owner: Some(lower),
        opposite_owner: Some(raised),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    };
    let heights = NodeHeightSolution {
        node_id: 12,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                lower,
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
                raised,
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
        .expect("explicit same-material raised step should authorize split heights");
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(start),
        NodeArrangementKey::from_point(end),
        lower,
        raised,
    )
    .expect("test segment is non-degenerate");

    assert!(
        arrangement
            .explicit_vertical_step_segments()
            .contains(&expected)
    );
}

#[test]
fn arrangement_accepts_height_ranked_step_endpoint_grouping() {
    let lower_left = owner(RoadSurfaceBandKind::Carriageway, 2);
    let raised_left = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_right = owner(RoadSurfaceBandKind::Carriageway, 15);
    let raised_right = owner(RoadSurfaceBandKind::CurbOrShoulder, 16);
    let lower_left_field = height_field_id(RoadSurfaceBandKind::Carriageway, 2);
    let raised_right_field = height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 16);
    let key = RoadVec2::new(0.0, 0.0);
    let left_end = RoadVec2::new(1.0, 0.0);
    let right_end = RoadVec2::new(0.0, 1.0);
    let left_step = NodeRegionSeamConstraint {
        constraint_index: 19,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 2 },
        owner: Some(lower_left),
        opposite_owner: Some(raised_left),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: key,
        end_xz: left_end,
    };
    let right_step = NodeRegionSeamConstraint {
        constraint_index: 72,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 16 },
        owner: Some(lower_right),
        opposite_owner: Some(raised_right),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: key,
        end_xz: right_end,
    };
    let mut arrangement = NodeArrangement::new(12, RoadSurfaceVisualNodePieceKind::JunctionN);

    let lower_start = arrangement
        .insert_vertex(
            key,
            0.0,
            [lower_left],
            lower_left_field,
            [left_step.seam_source],
        )
        .expect("lower step endpoint should insert");
    let lower_end = arrangement
        .insert_vertex(
            left_end,
            0.0,
            [lower_left],
            lower_left_field,
            [left_step.seam_source],
        )
        .expect("lower step edge endpoint should insert");
    let lower_edge = arrangement.push_edge(
        lower_start,
        lower_end,
        lower_left,
        lower_left_field,
        Some(raised_left),
        Some(height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1)),
        false,
        false,
        true,
        left_step.seam_source,
        vec![left_step.constraint_index],
    );
    arrangement.push_region(
        lower_left,
        lower_left_field,
        vec![lower_start, lower_end],
        Vec::new(),
        vec![lower_edge],
        1.0,
        vec![left_step],
    );

    let raised_start = arrangement
        .insert_vertex(
            key,
            0.12,
            [raised_right],
            raised_right_field,
            [right_step.seam_source],
        )
        .expect("raised step endpoint should insert");
    let raised_end = arrangement
        .insert_vertex(
            right_end,
            0.12,
            [raised_right],
            raised_right_field,
            [right_step.seam_source],
        )
        .expect("raised step edge endpoint should insert");
    let raised_edge = arrangement.push_edge(
        raised_start,
        raised_end,
        raised_right,
        raised_right_field,
        Some(lower_right),
        Some(height_field_id(RoadSurfaceBandKind::Carriageway, 15)),
        false,
        false,
        true,
        right_step.seam_source,
        vec![right_step.constraint_index],
    );
    arrangement.push_region(
        raised_right,
        raised_right_field,
        vec![raised_start, raised_end],
        Vec::new(),
        vec![raised_edge],
        1.0,
        vec![right_step],
    );

    arrangement
        .reject_implicit_material_height_conflicts()
        .expect("separate canonical step endpoints should authorize the ranked height split");
}

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
fn explicit_vertical_step_segments_reject_ambiguous_exposed_endpoint_pairs() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 14);
    let first_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let second_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 13);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let contact = |constraint_index, point, curb| NodeRegionSeamConstraint {
        constraint_index,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 14 },
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: point,
        end_xz: point,
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
            vec![
                contact(325, start, first_curb),
                contact(327, end, first_curb),
                contact(329, start, second_curb),
                contact(331, end, second_curb),
            ],
        )],
    };
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("ambiguous endpoint contacts should still produce a canonical arrangement");

    assert!(arrangement.explicit_vertical_step_segments().is_empty());
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
fn explicit_vertical_step_segments_require_explicit_owner_pair_source() {
    let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
    let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 93,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
        owner: Some(curb),
        opposite_owner: None,
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: RoadVec2::new(1.0, 0.0),
        end_xz: RoadVec2::new(1.0, 1.0),
    };
    let heights = two_region_height_solution_with_material_heights(
        carriageway,
        curb,
        0.0,
        0.12,
        vec![seam.clone()],
        vec![seam],
    );
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("role-only material seam should produce a canonical arrangement");

    let segments = arrangement.explicit_vertical_step_segments();
    let expected = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 1.0)),
        carriageway,
        curb,
    )
    .expect("test segment is non-degenerate");

    assert!(!segments.contains(&expected));
}
