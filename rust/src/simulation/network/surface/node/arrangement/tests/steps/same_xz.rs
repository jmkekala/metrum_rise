//! Same-XZ vertical-step arrangement tests.

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
fn arrangement_ignores_same_xz_height_split_without_final_boundary_contact() {
    let lower_left = owner(RoadSurfaceBandKind::Carriageway, 0);
    let raised = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_right = owner(RoadSurfaceBandKind::Carriageway, 2);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(1.0, 0.0);
    let step = NodeRegionSeamConstraint {
        constraint_index: 88,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 0 },
        owner: Some(lower_left),
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
                RoadSurfaceBandKind::Carriageway,
                lower_left,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(1.0, 0.0, 0.0),
                    height_vertex(0.0, 1.0, 0.0),
                ],
                vec![step.clone()],
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                lower_right,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(-1.0, 0.0, 0.0),
                    height_vertex(0.0, -1.0, 0.0),
                ],
                Vec::new(),
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                raised,
                vec![
                    height_vertex(0.0, 0.0, 0.12),
                    height_vertex(1.0, 0.0, 0.12),
                    height_vertex(1.0, -1.0, 0.12),
                ],
                vec![step],
            ),
        ],
    };

    NodeArrangement::from_height_solution(&heights)
        .expect("point-only coincidence is not a final owned boundary height conflict");
}
