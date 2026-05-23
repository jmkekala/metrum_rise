//! Same-height handoff bridge coverage tests.

use super::*;

#[test]
fn accepts_same_height_handoff_with_complete_split_bridge_edge_coverage() {
    let mouth_asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let joined_asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mouth_asphalt_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let joined_asphalt_field = NodeBandHeightFieldId::new(1, 0, RoadSurfaceBandKind::Carriageway);
    let curb_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::CurbOrShoulder);
    let start = NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0));
    let end = NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0));
    let asphalt_handoff =
        NodeExplicitVerticalStepSegment::new(start, end, mouth_asphalt_owner, joined_asphalt_owner)
            .expect("non-degenerate asphalt handoff segment");
    let curb_step =
        NodeExplicitVerticalStepSegment::new(start, end, joined_asphalt_owner, curb_owner)
            .expect("non-degenerate curb step segment");
    let solution = NodeTriangulationSolution {
        node_id: 103,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            manual_region_with_kind(
                RoadSurfaceBandKind::Carriageway,
                0,
                mouth_asphalt_field,
                vec![
                    RoadVec3::new(0.0, 0.0, 0.0),
                    RoadVec3::new(1.0, 0.0, 0.0),
                    RoadVec3::new(0.0, 0.0, -1.0),
                ],
            ),
            split_bridge_region(2, joined_asphalt_field),
            manual_region_with_kind(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(0.5, 0.12, -1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![asphalt_handoff, curb_step],
    };

    NodeValidationReport::from_triangulation_solution(&solution)
        .expect("split bridge edges fully covering the seam should carry handoff authority");
}

#[test]
fn accepts_same_height_handoff_when_bridge_edge_contains_conflict_edge() {
    let mouth_asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let joined_asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mouth_asphalt_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let joined_asphalt_field = NodeBandHeightFieldId::new(1, 0, RoadSurfaceBandKind::Carriageway);
    let curb_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::CurbOrShoulder);
    let start = NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0));
    let end = NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0));
    let asphalt_handoff =
        NodeExplicitVerticalStepSegment::new(start, end, mouth_asphalt_owner, joined_asphalt_owner)
            .expect("non-degenerate asphalt handoff segment");
    let curb_step =
        NodeExplicitVerticalStepSegment::new(start, end, joined_asphalt_owner, curb_owner)
            .expect("non-degenerate curb step segment");
    let solution = NodeTriangulationSolution {
        node_id: 106,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            manual_region_with_kind(
                RoadSurfaceBandKind::Carriageway,
                0,
                mouth_asphalt_field,
                vec![
                    RoadVec3::new(0.0, 0.0, 0.0),
                    RoadVec3::new(1.0, 0.0, 0.0),
                    RoadVec3::new(0.0, 0.0, -1.0),
                ],
            ),
            long_bridge_region(2, joined_asphalt_field),
            manual_region_with_kind(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(0.5, 0.12, -1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![asphalt_handoff, curb_step],
    };

    NodeValidationReport::from_triangulation_solution(&solution)
        .expect("a longer exact bridge edge may prove complete conflict-edge coverage");
}

#[test]
fn rejects_same_height_handoff_with_bridge_endpoints_but_no_bridge_edge() {
    let mouth_asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let joined_asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mouth_asphalt_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let joined_asphalt_field = NodeBandHeightFieldId::new(1, 0, RoadSurfaceBandKind::Carriageway);
    let curb_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::CurbOrShoulder);
    let start = NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0));
    let end = NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0));
    let asphalt_handoff =
        NodeExplicitVerticalStepSegment::new(start, end, mouth_asphalt_owner, joined_asphalt_owner)
            .expect("non-degenerate asphalt handoff segment");
    let curb_step =
        NodeExplicitVerticalStepSegment::new(start, end, joined_asphalt_owner, curb_owner)
            .expect("non-degenerate curb step segment");
    let endpoint_only_bridge = manual_region_with_constraints_and_triangles(
        RoadSurfaceBandKind::Carriageway,
        2,
        joined_asphalt_field,
        vec![RoadVec3::new(0.0, 0.0, 0.0), RoadVec3::new(1.0, 0.0, 0.0)],
        Vec::new(),
        Vec::new(),
        0.0,
    );
    let solution = NodeTriangulationSolution {
        node_id: 104,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            manual_region_with_kind(
                RoadSurfaceBandKind::Carriageway,
                0,
                mouth_asphalt_field,
                vec![
                    RoadVec3::new(0.0, 0.0, 0.0),
                    RoadVec3::new(1.0, 0.0, 0.0),
                    RoadVec3::new(0.0, 0.0, -1.0),
                ],
            ),
            endpoint_only_bridge,
            manual_region_with_kind(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(0.5, 0.12, -1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![asphalt_handoff, curb_step],
    };

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("endpoint-only bridge ownership must not authorize a height conflict");
    assert!(report_has_cross_region_height_conflict(&error.report));
}

#[test]
fn rejects_same_height_handoff_with_gapped_split_bridge_edge_coverage() {
    let mouth_asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let joined_asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mouth_asphalt_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let joined_asphalt_field = NodeBandHeightFieldId::new(1, 0, RoadSurfaceBandKind::Carriageway);
    let curb_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::CurbOrShoulder);
    let start = NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0));
    let end = NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0));
    let asphalt_handoff =
        NodeExplicitVerticalStepSegment::new(start, end, mouth_asphalt_owner, joined_asphalt_owner)
            .expect("non-degenerate asphalt handoff segment");
    let curb_step =
        NodeExplicitVerticalStepSegment::new(start, end, joined_asphalt_owner, curb_owner)
            .expect("non-degenerate curb step segment");
    let solution = NodeTriangulationSolution {
        node_id: 105,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            manual_region_with_kind(
                RoadSurfaceBandKind::Carriageway,
                0,
                mouth_asphalt_field,
                vec![
                    RoadVec3::new(0.0, 0.0, 0.0),
                    RoadVec3::new(1.0, 0.0, 0.0),
                    RoadVec3::new(0.0, 0.0, -1.0),
                ],
            ),
            gapped_bridge_region(2, joined_asphalt_field),
            manual_region_with_kind(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(0.5, 0.12, -1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![asphalt_handoff, curb_step],
    };

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("gapped bridge ownership must not authorize a full-edge height conflict");
    assert!(report_has_cross_region_height_conflict(&error.report));
}

#[test]
fn accepts_cross_region_cdt_edge_height_conflict_on_canonical_asphalt_sidewalk_step() {
    let carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let carriageway_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let sidewalk_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::Sidewalk);
    let step_segment = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0)),
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
        carriageway_owner,
        sidewalk_owner,
    )
    .expect("non-degenerate test step segment");
    let solution = NodeTriangulationSolution {
        node_id: 101,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        regions: vec![
            manual_region_with_kind(
                RoadSurfaceBandKind::Carriageway,
                0,
                carriageway_field,
                vec![
                    RoadVec3::new(0.0, 0.0, 0.0),
                    RoadVec3::new(1.0, 0.0, 0.0),
                    RoadVec3::new(0.0, 0.0, -1.0),
                ],
            ),
            manual_region_with_kind(
                RoadSurfaceBandKind::Sidewalk,
                1,
                sidewalk_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![step_segment],
    };

    NodeValidationReport::from_triangulation_solution(&solution)
        .expect("canonical asphalt-sidewalk vertical step should allow the height delta");
}
