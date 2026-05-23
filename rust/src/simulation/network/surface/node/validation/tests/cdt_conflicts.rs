//! Cross-region CDT edge-height validation tests.

use super::*;

#[test]
fn validates_clean_triangulated_solution() {
    let solution = solved_triangulation();
    let report = NodeValidationReport::from_triangulation_solution(&solution)
        .expect("fresh triangulation should validate");

    assert_eq!(report.node_id, 42);
    assert_eq!(report.piece_kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_eq!(report.region_count, solution.regions.len());
    assert!(report.triangle_count > 0);
    assert!(report.exposed_edge_count > 0);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn rejects_cross_region_cdt_edge_height_conflict() {
    let carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let wrong_carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let carriageway_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let curb_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::CurbOrShoulder);
    let owner_matching_wrong_span = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(0.0, 2.0)),
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 2.0)),
        carriageway_owner,
        curb_owner,
    )
    .expect("non-degenerate test step segment");
    let geometry_matching_wrong_owner = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0)),
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
        wrong_carriageway_owner,
        curb_owner,
    )
    .expect("non-degenerate test step segment");
    let solution = NodeTriangulationSolution {
        node_id: 99,
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
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![
            owner_matching_wrong_span,
            geometry_matching_wrong_owner,
        ],
    };

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("same XZ CDT edge with different endpoint heights must reject");

    let diagnostic = error
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.stage == NodeGeometryStage::Validation
                && diagnostic.backend == NodeGeometryBackend::Spade
                && matches!(
                    diagnostic.kind,
                    NodeGeometryDiagnosticKind::CrossRegionHeightConflict { .. }
                )
        })
        .expect("cross-region height conflict should be reported with edge context");
    let NodeGeometryDiagnosticKind::CrossRegionHeightConflict {
        edge_start_x_key,
        edge_start_z_key,
        edge_end_x_key,
        edge_end_z_key,
        conflict_x_key,
        conflict_z_key,
        existing_owner,
        existing_owner_index,
        incoming_owner,
        incoming_owner_index,
        existing_conflict_height_mm,
        incoming_conflict_height_mm,
        matching_explicit_step_segments,
        non_matching_explicit_step_segments,
        ..
    } = &diagnostic.kind
    else {
        unreachable!("diagnostic was filtered above");
    };
    assert_eq!((*edge_start_x_key, *edge_start_z_key), (0, 0));
    assert_eq!((*edge_end_x_key, *edge_end_z_key), (1_000_000, 0));
    assert_eq!((*conflict_x_key, *conflict_z_key), (0, 0));
    assert_eq!(
        (*existing_owner, *existing_owner_index),
        (RoadSurfaceBandKind::Carriageway, 0)
    );
    assert_eq!(
        (*incoming_owner, *incoming_owner_index),
        (RoadSurfaceBandKind::CurbOrShoulder, 1)
    );
    assert_eq!(
        (*existing_conflict_height_mm, *incoming_conflict_height_mm),
        (0, 120)
    );
    assert!(matching_explicit_step_segments.is_empty());
    assert_eq!(non_matching_explicit_step_segments.len(), 2);
    assert!(
        non_matching_explicit_step_segments
            .iter()
            .any(|segment| { segment.owners_match_regions && !segment.edge_lies_on_segment })
    );
    assert!(
        non_matching_explicit_step_segments
            .iter()
            .any(|segment| { !segment.owners_match_regions && segment.edge_lies_on_segment })
    );

    let dump = error.report.debug_dump();
    assert!(dump.contains("edge_start_x_key"));
    assert!(dump.contains("matching_explicit_step_segments"));
    assert!(dump.contains("non_matching_explicit_step_segments"));
}

#[test]
fn accepts_cross_region_cdt_edge_height_conflict_on_canonical_asphalt_curb_step() {
    let carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let carriageway_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let curb_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::CurbOrShoulder);
    let step_segment = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0)),
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
        carriageway_owner,
        curb_owner,
    )
    .expect("non-degenerate test step segment");
    let solution = NodeTriangulationSolution {
        node_id: 100,
        piece_kind: RoadSurfaceVisualNodePieceKind::Terminal,
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
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
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
        .expect("canonical asphalt-curb vertical step should allow the curb height delta");
}

#[test]
fn accepts_explicit_step_across_same_height_asphalt_owner_handoff() {
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
        node_id: 102,
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
            manual_region_with_kind(
                RoadSurfaceBandKind::Carriageway,
                2,
                joined_asphalt_field,
                vec![
                    RoadVec3::new(0.0, 0.0, 0.0),
                    RoadVec3::new(1.0, 0.0, 0.0),
                    RoadVec3::new(0.0, 0.0, 1.0),
                ],
            ),
            manual_region_with_kind(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![asphalt_handoff, curb_step],
    };

    NodeValidationReport::from_triangulation_solution(&solution)
        .expect("same-height asphalt owner handoff should carry the explicit curb step authority");
}
