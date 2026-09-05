// SPDX-License-Identifier: GPL-2.0-only

//! Validation diagnostic mapping tests.

use super::*;

#[test]
fn maps_vertex_outside_height_field_to_source_rich_blocking_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 4);
    let height_field_id = NodeBandHeightFieldId::new(2, 3, RoadSurfaceBandKind::Sidewalk);
    let report = NodeValidationReport::from_height_field_error(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeHeightFieldError::VertexOutsideHeightField {
            mouth_order_index: 2,
            band_index: 3,
            source_kind: RoadSurfaceBandKind::Sidewalk,
            height_field_id,
            owner: Some(owner),
            point_x_mm: 12_345,
            point_z_mm: -6_789,
            axis: "canonical_authority",
            raw_parameter: f64::NAN,
        },
    );

    assert!(report.has_blocking_diagnostics());
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::HeightEvaluation);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::HeightCarrier);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::HeightFieldFailure {
            reason: "vertex_outside_height_field",
            mouth_order_index: Some(2),
            band_index: Some(3),
            source_kind: Some(RoadSurfaceBandKind::Sidewalk),
            height_field_id: Some(id),
            owner: Some(mapped_owner),
            point_x_mm: Some(12_345),
            point_z_mm: Some(-6_789),
            axis: Some("canonical_authority"),
            ..
        } if id == height_field_id && mapped_owner == owner
    ));
    let dump = report.debug_dump();
    assert!(dump.contains("\"kind\":\"height_field_failure\""));
    assert!(dump.contains("height_field_id"));
    assert!(dump.contains("owner"));
}

#[test]
fn maps_missing_source_band_to_owner_scoped_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 12);
    let report = NodeValidationReport::from_height_field_error(
        6,
        RoadSurfaceVisualNodePieceKind::Bend,
        &NodeHeightFieldError::MissingSourceBand {
            mouth_order_index: 3,
            band_index: 7,
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner: Some(owner),
        },
    );

    assert!(report.has_blocking_diagnostics());
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::HeightEvaluation);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::HeightCarrier);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::HeightFieldFailure {
            reason: "missing_source_band",
            mouth_order_index: Some(3),
            band_index: Some(7),
            kind: Some(RoadSurfaceBandKind::CurbOrShoulder),
            source_kind: Some(RoadSurfaceBandKind::CurbOrShoulder),
            height_field_id: None,
            owner: Some(mapped_owner),
            ..
        } if mapped_owner == owner
    ));

    let parsed: serde_json::Value =
        serde_json::from_str(&report.debug_dump()).expect("diagnostic dump must be valid JSON");
    let diagnostic = &parsed["diagnostics"][0];
    assert_eq!(diagnostic["stage"], "height_evaluation");
    assert_eq!(diagnostic["backend"], "height_carrier");
    assert_eq!(diagnostic["reason"], "missing_source_band");
    assert_eq!(diagnostic["region_kind"], "CurbOrShoulder");
    assert_eq!(diagnostic["source_kind"], "CurbOrShoulder");
    assert_eq!(diagnostic["owner"]["kind"], "CurbOrShoulder");
    assert_eq!(diagnostic["owner"]["owner_index"], 12);
    assert!(diagnostic["height_field_id"].is_null());
}

#[test]
fn maps_missing_owned_region_carrier_support_to_source_rich_blocking_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 17);
    let height_field_id = NodeBandHeightFieldId::new(2, 5, RoadSurfaceBandKind::Sidewalk);
    let point_x_key = -1_785_000;
    let point_z_key = -5_439_600;
    let report = NodeValidationReport::from_height_field_error(
        2,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeHeightFieldError::MissingOwnedRegionCarrierSupport {
            mouth_order_index: 2,
            band_index: 5,
            source_kind: RoadSurfaceBandKind::Sidewalk,
            height_field_id,
            owner,
            point_x_key,
            point_z_key,
            point_x_mm: -17_850,
            point_z_mm: -54_396,
        },
    );

    assert!(report.has_blocking_diagnostics());
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::HeightEvaluation);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::HeightCarrier);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::HeightFieldFailure {
            reason: "missing_owned_region_carrier_support",
            mouth_order_index: Some(2),
            band_index: Some(5),
            source_kind: Some(RoadSurfaceBandKind::Sidewalk),
            height_field_id: Some(id),
            owner: Some(mapped_owner),
            point_x_key: Some(mapped_x_key),
            point_z_key: Some(mapped_z_key),
            point_x_mm: Some(-17_850),
            point_z_mm: Some(-54_396),
            axis: None,
            raw_parameter: None,
            ..
        } if id == height_field_id
            && mapped_owner == owner
            && mapped_x_key == point_x_key
            && mapped_z_key == point_z_key
    ));
    let dump = report.debug_dump();
    assert!(dump.contains("missing_owned_region_carrier_support"));
    assert!(dump.contains("height_field_id"));
    assert!(dump.contains("owner"));
    let parsed: serde_json::Value =
        serde_json::from_str(&dump).expect("diagnostic dump must be valid JSON");
    let diagnostic = &parsed["diagnostics"][0];
    assert_eq!(diagnostic["kind"], "height_field_failure");
    assert_eq!(diagnostic["reason"], "missing_owned_region_carrier_support");
    assert_eq!(diagnostic["source_kind"], "Sidewalk");
    assert_eq!(diagnostic["owner"]["kind"], "Sidewalk");
    assert_eq!(diagnostic["owner"]["owner_index"], 17);
    assert_eq!(diagnostic["height_field_id"]["mouth_order_index"], 2);
    assert_eq!(diagnostic["height_field_id"]["band_index"], 5);
    assert_eq!(diagnostic["point_x_key"], point_x_key);
    assert_eq!(diagnostic["point_z_key"], point_z_key);
    assert_eq!(diagnostic["point_x_mm"], -17_850);
    assert_eq!(diagnostic["point_z_mm"], -54_396);
    assert!(diagnostic.get("detail").is_none());
}

#[test]
fn maps_missing_carrier_provenance_height_to_source_segment_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 17);
    let height_field_id = NodeBandHeightFieldId::new(2, 5, RoadSurfaceBandKind::Sidewalk);
    let source_segment_id = NodeSourceCarrierSegmentId {
        owner,
        source_kind: RoadSurfaceBandKind::Sidewalk,
        source_mouth_order_index: 2,
        source_band_index: 5,
        segment_start: NodeOwnedRegionArrangementKey::from_ownership_key((-1_785_000, -5_439_600)),
        segment_end: NodeOwnedRegionArrangementKey::from_ownership_key((-1_785_000, -5_430_000)),
    };
    let report = NodeValidationReport::from_height_field_error(
        2,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeHeightFieldError::MissingCarrierProvenanceHeight {
            mouth_order_index: 2,
            band_index: 5,
            source_kind: RoadSurfaceBandKind::Sidewalk,
            height_field_id,
            point_x_key: -1_785_000,
            point_z_key: -5_439_600,
            point_x_mm: -17_850,
            point_z_mm: -54_396,
            source_segment_id,
        },
    );

    assert!(report.has_blocking_diagnostics());
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::HeightEvaluation);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::HeightCarrier);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::MissingCarrierProvenanceHeight {
            source_kind: RoadSurfaceBandKind::Sidewalk,
            source_mouth_order_index: 2,
            source_band_index: 5,
            height_field_id: mapped_height_field_id,
            source_segment_id: mapped_source_segment_id,
            ..
        } if mapped_height_field_id == height_field_id
            && mapped_source_segment_id == source_segment_id
    ));
    let parsed: serde_json::Value =
        serde_json::from_str(&report.debug_dump()).expect("diagnostic dump must be valid JSON");
    let diagnostic = &parsed["diagnostics"][0];
    assert_eq!(diagnostic["kind"], "missing_carrier_provenance_height");
    assert_eq!(diagnostic["source_segment_id"]["owner"]["owner_index"], 17);
    assert_eq!(
        diagnostic["source_segment_id"]["source"]["kind"],
        "Sidewalk"
    );
    assert_eq!(
        diagnostic["source_segment_id"]["segment_start"]["x_key"],
        -1_785_000
    );
}

#[test]
fn maps_missing_grade_authority_to_blocking_node_grade_diagnostic() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 4);
    let height_field_id = NodeBandHeightFieldId::new(2, 3, RoadSurfaceBandKind::Sidewalk);
    let key = NodeArrangementKey::from_point(RoadVec2::new(12.345, -6.789));
    let report = NodeValidationReport::from_arrangement_error(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeArrangementError::MissingGradeAuthority {
            region_index: 5,
            contour_index: 1,
            key,
            owner,
            height_field_id,
            height_mm: 1750,
        },
    );

    assert!(report.has_blocking_diagnostics());
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::NodeGrade);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::HeightCarrier);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::MissingGradeAuthority {
            region_index: 5,
            contour_index: 1,
            owner: RoadSurfaceBandKind::Sidewalk,
            owner_index: 4,
            height_field_id: id,
            height_mm: 1750,
            ..
        } if id == height_field_id
    ));
}

#[test]
fn maps_source_height_conflict_to_source_rich_blocking_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 7);
    let height_field_id = NodeBandHeightFieldId::new(1, 2, RoadSurfaceBandKind::CurbOrShoulder);
    let incoming_authority = NodeHeightAuthoritySource::GeneratedContour {
        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
    };
    let report = NodeValidationReport::from_height_field_error(
        12,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeHeightFieldError::SourceHeightFieldConflict {
            mouth_order_index: 1,
            band_index: 2,
            source_kind: RoadSurfaceBandKind::CurbOrShoulder,
            height_field_id,
            owner: Some(owner),
            existing_authority: NodeHeightAuthoritySource::SourceInterval,
            incoming_authority,
            point_x_mm: 3_000,
            point_z_mm: 4_000,
            existing_height_mm: 120,
            incoming_height_mm: 180,
        },
    );

    assert!(report.has_blocking_diagnostics());
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::HeightEvaluation);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::HeightCarrier);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::SourceHeightFieldConflict {
            mouth_order_index: 1,
            band_index: 2,
            source_kind: RoadSurfaceBandKind::CurbOrShoulder,
            height_field_id: id,
            owner: Some(mapped_owner),
            existing_authority: NodeHeightAuthoritySource::SourceInterval,
            incoming_authority: mapped_incoming,
            x_mm: 3_000,
            z_mm: 4_000,
            existing_height_mm: 120,
            incoming_height_mm: 180,
        } if id == height_field_id
            && mapped_owner == owner
            && mapped_incoming == incoming_authority
    ));
    let dump = report.debug_dump();
    assert!(dump.contains("\"kind\":\"source_height_field_conflict\""));
    assert!(dump.contains("JunctionSideJoin"));
    assert!(dump.contains("height_field_id"));
}

#[test]
fn maps_shared_source_height_conflict_to_owner_pair_blocking_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let opposite_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3);
    let height_field_id = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::Carriageway);
    let report = NodeValidationReport::from_height_field_error(
        13,
        RoadSurfaceVisualNodePieceKind::Bend,
        &NodeHeightFieldError::SharedSourceHeightConflict {
            point_x_mm: -2_000,
            point_z_mm: 8_000,
            kind: RoadSurfaceBandKind::Carriageway,
            owner,
            opposite_owner: Some(opposite_owner),
            height_field_id: Some(height_field_id),
            incoming_owner: owner,
            incoming_height_field_id: Some(height_field_id),
            constraint_index: Some(9),
            existing_authority: Some(NodeHeightAuthoritySource::SourceInterval),
            incoming_authority: Some(NodeHeightAuthoritySource::TerminalCap),
            existing_height_mm: 0,
            incoming_height_mm: 125,
        },
    );

    assert!(report.has_blocking_diagnostics());
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::HeightEvaluation);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::HeightCarrier);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::SharedSourceHeightConflict {
            x_mm: -2_000,
            z_mm: 8_000,
            kind: RoadSurfaceBandKind::Carriageway,
            owner: mapped_owner,
            opposite_owner: Some(mapped_opposite_owner),
            height_field_id: Some(id),
            incoming_owner: mapped_incoming_owner,
            incoming_height_field_id: Some(incoming_id),
            constraint_index: Some(9),
            existing_authority: Some(NodeHeightAuthoritySource::SourceInterval),
            incoming_authority: Some(NodeHeightAuthoritySource::TerminalCap),
            existing_height_mm: 0,
            incoming_height_mm: 125,
            constraint_context: None,
        } if mapped_owner == owner
            && mapped_opposite_owner == opposite_owner
            && id == height_field_id
            && mapped_incoming_owner == owner
            && incoming_id == height_field_id
    ));
    let dump = report.debug_dump();
    assert!(dump.contains("\"kind\":\"shared_source_height_conflict\""));
    assert!(dump.contains("opposite_owner"));
    assert!(dump.contains("constraint_index"));
}

#[test]
fn shared_source_height_conflict_dump_includes_constraint_context() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
    let curb_field = NodeBandHeightFieldId::new(0, 4, RoadSurfaceBandKind::CurbOrShoulder);
    let sidewalk_field = NodeBandHeightFieldId::new(0, 5, RoadSurfaceBandKind::Sidewalk);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 21,
        seam_source: NodeSeamSource::RaisedStepContact { owner_index: 4 },
        owner: Some(curb),
        opposite_owner: Some(sidewalk),
        constrains_shared_height: true,
        is_material_transition: true,
        start_xz: RoadVec2::new(0.0, 0.0),
        end_xz: RoadVec2::new(1.0, 0.0),
    };
    let regions = vec![
        NodeBooleanOwnedRegion {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner: curb,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            source_mouth_order_index: 0,
            source_band_index: Some(4),
            shape: Vec::new(),
            area_m2: 0.0,
            seam_constraints: vec![seam.clone()],
        },
        NodeBooleanOwnedRegion {
            kind: RoadSurfaceBandKind::Sidewalk,
            owner: sidewalk,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            source_mouth_order_index: 0,
            source_band_index: Some(5),
            shape: Vec::new(),
            area_m2: 0.0,
            seam_constraints: vec![seam],
        },
    ];
    let ownership = NodeBooleanOwnership {
        node_id: 13,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        footprint_shapes: Vec::new(),
        asphalt_shapes: Vec::new(),
        non_road_shapes: Vec::new(),
        owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
            13,
            RoadSurfaceVisualNodePieceKind::Bend,
            &regions,
            &Vec::new(),
            &Vec::new(),
        ),
        carrier_provenance: NodeCarrierProvenanceClosure {
            records: vec![
                NodeCarrierProvenanceRecord {
                    owner: curb,
                    source_kind: RoadSurfaceBandKind::CurbOrShoulder,
                    source_mouth_order_index: 0,
                    source_band_index: 4,
                    height_field_id: curb_field,
                    claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                    point: NodeOwnedRegionArrangementKey::from_ownership_key((0, 0)),
                    origin: NodeCarrierProvenanceOrigin::GeneratedCarrierVertex {
                        contour_index: 2,
                        purpose: NodeGeneratedContourPurpose::BendSideJoin,
                        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                    },
                },
                NodeCarrierProvenanceRecord {
                    owner: sidewalk,
                    source_kind: RoadSurfaceBandKind::Sidewalk,
                    source_mouth_order_index: 0,
                    source_band_index: 5,
                    height_field_id: sidewalk_field,
                    claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                    point: NodeOwnedRegionArrangementKey::from_ownership_key((0, 0)),
                    origin: NodeCarrierProvenanceOrigin::GeneratedCarrierVertex {
                        contour_index: 3,
                        purpose: NodeGeneratedContourPurpose::BendSideJoin,
                        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                    },
                },
            ],
        },
        owned_regions: regions,
    };
    let rails = NodeRailContourSet {
        node_id: 13,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        contours: Vec::new(),
        corner_trims: Vec::new(),
        side_join_gaps: Vec::new(),
        constraints: vec![NodeRailConstraint {
            constraint_index: 21,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(4),
            source_boundary_index: None,
            owner: Some(curb),
            opposite_owner: Some(sidewalk),
            points_xz: vec![RoadVec2::new(0.0, 0.0), RoadVec2::new(1.0, 0.0)],
        }],
        height_carrier_paths_by_source: std::collections::BTreeMap::new(),
        height_carrier_points_by_source: std::collections::BTreeMap::new(),
        source_carriers: NodeSourceCarrierRegistry::default(),
    };
    let report = NodeValidationReport::from_height_field_error(
        13,
        RoadSurfaceVisualNodePieceKind::Bend,
        &NodeHeightFieldError::SharedSourceHeightConflict {
            point_x_mm: 0,
            point_z_mm: 0,
            kind: RoadSurfaceBandKind::Sidewalk,
            owner: curb,
            opposite_owner: Some(sidewalk),
            height_field_id: Some(curb_field),
            incoming_owner: sidewalk,
            incoming_height_field_id: Some(sidewalk_field),
            constraint_index: Some(21),
            existing_authority: None,
            incoming_authority: None,
            existing_height_mm: 137_383,
            incoming_height_mm: 137_365,
        },
    )
    .with_height_failure_context(&rails, &ownership);

    let parsed: serde_json::Value =
        serde_json::from_str(&report.debug_dump()).expect("debug dump must be valid JSON");
    let context = &parsed["diagnostics"][0]["constraint_context"];
    assert_eq!(context["rail_constraint"]["constraint_index"], 21);
    assert_eq!(
        context["rail_constraint"]["kind"]["type"],
        "RaisedStepContact"
    );
    assert_eq!(context["rail_constraint"]["source_mouth_order_index"], 0);
    assert_eq!(context["rail_constraint"]["source_band_index"], 4);
    assert_eq!(
        context["seam_constraints"][0]["seam_source"]["type"],
        "RaisedStepContact"
    );
    assert_eq!(
        context["existing_vertex"]["provenance_records"][0]["origin"]["purpose"],
        "BendSideJoin"
    );
    assert_eq!(
        context["incoming_vertex"]["provenance_records"][0]["origin"]["purpose"],
        "BendSideJoin"
    );
}

#[test]
fn maps_boolean_residual_to_structured_debug_record() {
    let report = NodeValidationReport::from_boolean_ownership_error(
        8,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeBooleanOwnershipError::UnownedNonRoadResidual {
            shape_count: 2,
            area_m2: 0.5,
        },
    );

    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::BooleanOwnership);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::IOverlay);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::RejectedResidual {
            residual: NodeRejectedResidualKind::NonRoad,
            ..
        }
    ));
    assert!(
        report
            .debug_dump()
            .contains("\"kind\":\"rejected_residual\"")
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&report.debug_dump()).expect("diagnostic dump must be valid JSON");
    let diagnostic = &parsed["diagnostics"][0];
    assert_eq!(diagnostic["stage"], "boolean_ownership");
    assert_eq!(diagnostic["backend"], "i_overlay");
    assert_eq!(diagnostic["residual"]["type"], "non_road");
    assert_eq!(diagnostic["shape_count"], 2);
    assert_eq!(diagnostic["area_m2"], 0.5);
    assert!(diagnostic.get("detail").is_none());
}

#[test]
fn maps_band_residual_to_material_scoped_debug_record() {
    let report = NodeValidationReport::from_boolean_ownership_error(
        8,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeBooleanOwnershipError::UnownedBandResidual {
            kind: RoadSurfaceBandKind::Sidewalk,
            shape_count: 3,
            area_m2: 0.75,
        },
    );

    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::BooleanOwnership);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::IOverlay);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::RejectedResidual {
            residual: NodeRejectedResidualKind::Band(RoadSurfaceBandKind::Sidewalk),
            shape_count: 3,
            area_m2,
        } if area_m2 == 0.75
    ));

    let parsed: serde_json::Value =
        serde_json::from_str(&report.debug_dump()).expect("diagnostic dump must be valid JSON");
    let diagnostic = &parsed["diagnostics"][0];
    assert_eq!(diagnostic["stage"], "boolean_ownership");
    assert_eq!(diagnostic["backend"], "i_overlay");
    assert_eq!(diagnostic["residual"]["type"], "band");
    assert_eq!(diagnostic["residual"]["kind"], "Sidewalk");
    assert_eq!(diagnostic["shape_count"], 3);
}

#[test]
fn maps_ambiguous_canonical_owned_region_vertex_to_source_rich_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
    let report = NodeValidationReport::from_boolean_ownership_error(
        2,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex {
            owner,
            point_x_key: -62_874_250,
            point_z_key: -60_856_125,
            candidates: vec![(-62_874_000, -60_856_000), (-62_874_500, -60_856_250)],
        },
    );

    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::BooleanOwnership);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::IOverlay);
    assert!(matches!(
        &diagnostic.kind,
        NodeGeometryDiagnosticKind::AmbiguousCanonicalOwnedRegionVertex {
            owner: diagnostic_owner,
            point_x_key: -62_874_250,
            point_z_key: -60_856_125,
            candidates,
            ..
        } if *diagnostic_owner == owner
            && candidates.len() == 2
            && candidates[0].x_key == -62_874_000
            && candidates[1].x_key == -62_874_500
    ));
    let dump = report.debug_dump();
    assert!(dump.contains("\"kind\":\"ambiguous_canonical_owned_region_vertex\""));
    assert!(dump.contains("point_x_mm"));
    assert!(dump.contains("candidates"));
}

#[test]
fn maps_ambiguous_source_segment_authorization_to_source_rich_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let report = NodeValidationReport::from_boolean_ownership_error(
        2,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
            owner,
            point_x_key: -39_339_253,
            point_z_key: -57_072_177,
            source_kind: RoadSurfaceBandKind::Carriageway,
            source_mouth_order_index: 1,
            source_band_index: 2,
            candidates: vec![
                NodeSourceSegmentAuthorizationCandidate {
                    source_segment_id: NodeSourceCarrierSegmentId {
                        owner,
                        source_kind: RoadSurfaceBandKind::Carriageway,
                        source_mouth_order_index: 1,
                        source_band_index: 2,
                        segment_start: NodeOwnedRegionArrangementKey::from_ownership_key((
                            -39_339_263,
                            -57_072_175,
                        )),
                        segment_end: NodeOwnedRegionArrangementKey::from_ownership_key((
                            -39_340_263,
                            -57_072_175,
                        )),
                    },
                    source_kind: RoadSurfaceBandKind::Carriageway,
                    source_mouth_order_index: 1,
                    source_band_index: 2,
                    canonical_point: (-39_339_263, -57_072_175),
                    segment_start: (-39_339_263, -57_072_175),
                    segment_end: (-39_340_263, -57_072_175),
                    distance_key_units_sq: 104,
                    dust_budget_key_units: 256,
                },
                NodeSourceSegmentAuthorizationCandidate {
                    source_segment_id: NodeSourceCarrierSegmentId {
                        owner,
                        source_kind: RoadSurfaceBandKind::Carriageway,
                        source_mouth_order_index: 1,
                        source_band_index: 2,
                        segment_start: NodeOwnedRegionArrangementKey::from_ownership_key((
                            -39_339_147,
                            -57_071_688,
                        )),
                        segment_end: NodeOwnedRegionArrangementKey::from_ownership_key((
                            -39_339_147,
                            -57_070_688,
                        )),
                    },
                    source_kind: RoadSurfaceBandKind::Carriageway,
                    source_mouth_order_index: 1,
                    source_band_index: 2,
                    canonical_point: (-39_339_147, -57_071_688),
                    segment_start: (-39_339_147, -57_071_688),
                    segment_end: (-39_339_147, -57_070_688),
                    distance_key_units_sq: 250_000,
                    dust_budget_key_units: 256,
                },
            ],
        },
    );

    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::BooleanOwnership);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::IOverlay);
    assert!(matches!(
        &diagnostic.kind,
        NodeGeometryDiagnosticKind::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
            owner: diagnostic_owner,
            point_x_key: -39_339_253,
            point_z_key: -57_072_177,
            source_kind: RoadSurfaceBandKind::Carriageway,
            source_mouth_order_index: 1,
            source_band_index: 2,
            candidates,
            ..
        } if *diagnostic_owner == owner
            && candidates.len() == 2
            && candidates[0].canonical_point == (-39_339_263, -57_072_175)
    ));
    let dump = report.debug_dump();
    assert!(dump.contains("\"kind\":\"ambiguous_source_segment_authorization\""));
    assert!(dump.contains("source_segment_id"));
    assert!(dump.contains("dust_budget_key_units"));
    assert!(dump.contains("segment_start"));
}

#[test]
fn carrier_provenance_closure_maps_missing_diagnostic_to_json() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 17);
    let height_field_id = NodeBandHeightFieldId::new(2, 5, RoadSurfaceBandKind::Sidewalk);
    let report = NodeValidationReport::from_boolean_ownership_error(
        2,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &NodeBooleanOwnershipError::MissingCarrierProvenance {
            owner,
            point_x_key: -1_785_000,
            point_z_key: -5_439_600,
            source_kind: RoadSurfaceBandKind::Sidewalk,
            source_mouth_order_index: 2,
            source_band_index: 5,
            height_field_id,
        },
    );

    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.stage, NodeGeometryStage::BooleanOwnership);
    assert_eq!(diagnostic.backend, NodeGeometryBackend::IOverlay);
    assert!(matches!(
        diagnostic.kind,
        NodeGeometryDiagnosticKind::MissingCarrierProvenance {
            owner: mapped_owner,
            source_kind: RoadSurfaceBandKind::Sidewalk,
            source_mouth_order_index: 2,
            source_band_index: 5,
            height_field_id: mapped_height_field,
            ..
        } if mapped_owner == owner && mapped_height_field == height_field_id
    ));

    let parsed: serde_json::Value =
        serde_json::from_str(&report.debug_dump()).expect("diagnostic dump must be valid JSON");
    let diagnostic = &parsed["diagnostics"][0];
    assert_eq!(diagnostic["kind"], "missing_carrier_provenance");
    assert_eq!(diagnostic["stage"], "boolean_ownership");
    assert_eq!(diagnostic["backend"], "i_overlay");
    assert_eq!(diagnostic["owner"]["kind"], "Sidewalk");
    assert_eq!(diagnostic["owner"]["owner_index"], 17);
    assert_eq!(diagnostic["source"]["kind"], "Sidewalk");
    assert_eq!(diagnostic["source"]["mouth_order_index"], 2);
    assert_eq!(diagnostic["source"]["band_index"], 5);
    assert_eq!(diagnostic["height_field_id"]["mouth_order_index"], 2);
    assert_eq!(diagnostic["height_field_id"]["band_index"], 5);
    assert_eq!(diagnostic["point_x_mm"], -1_785);
    assert_eq!(diagnostic["point_z_mm"], -5_440);
}

#[test]
fn maps_arrangement_seam_diagnostic_to_structured_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let opposite_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let diagnostic = NodeArrangementDiagnostic::MissingSeamConstraint {
        region_index: 3,
        owner,
        opposite_owner,
        start: NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
        end: NodeArrangementKey::from_point(RoadVec2::new(1.0, 2.0)),
    };

    let mapped = NodeGeometryDiagnostic::from_arrangement_diagnostic(
        9,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &diagnostic,
    );

    assert_eq!(mapped.stage, NodeGeometryStage::Validation);
    assert_eq!(mapped.backend, NodeGeometryBackend::Parry2d);
    assert!(matches!(
        mapped.kind,
        NodeGeometryDiagnosticKind::SeamConstraintFailure {
            region_index: 3,
            owner: RoadSurfaceBandKind::Carriageway,
            owner_index: 0,
            opposite_owner: RoadSurfaceBandKind::Sidewalk,
            opposite_owner_index: 1,
            start_x_mm: 1000,
            start_z_mm: 0,
            end_x_mm: 1000,
            end_z_mm: 2000,
            reason: NodeSeamConstraintFailureReason::Missing,
        }
    ));
    assert!(
        mapped
            .debug_record()
            .contains("\"kind\":\"seam_constraint_failure\"")
    );
}

#[test]
fn maps_owned_region_arrangement_diagnostic_to_boolean_stage_debug_record() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let opposite_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let diagnostic = NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
        region_index: 2,
        owner,
        opposite_owner,
        start: NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(2.0, 0.0)),
        end: NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(2.0, 3.0)),
    };

    let mapped = NodeGeometryDiagnostic::from_owned_region_arrangement_diagnostic(
        10,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &diagnostic,
    );

    assert_eq!(mapped.stage, NodeGeometryStage::BooleanOwnership);
    assert_eq!(mapped.backend, NodeGeometryBackend::IOverlay);
    assert!(matches!(
        mapped.kind,
        NodeGeometryDiagnosticKind::SeamConstraintFailure {
            region_index: 2,
            owner: RoadSurfaceBandKind::Carriageway,
            owner_index: 0,
            opposite_owner: RoadSurfaceBandKind::Sidewalk,
            opposite_owner_index: 1,
            start_x_mm: 2000,
            start_z_mm: 0,
            end_x_mm: 2000,
            end_z_mm: 3000,
            reason: NodeSeamConstraintFailureReason::Missing,
        }
    ));
    assert!(
        mapped
            .debug_record()
            .contains("\"stage\":\"boolean_ownership\"")
    );
}
