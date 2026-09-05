// SPDX-License-Identifier: GPL-2.0-only

//! Rail contour and generated-edge tests.

use super::*;

#[test]
fn generates_backend_contours_and_constraints_from_solved_mouth_input() {
    let contours =
        NodeRailContourSet::from_input(&input_with_endpoint_x(0.0)).expect("valid contours");

    assert_eq!(contours.node_id, 42);
    assert_eq!(
        contours.piece_kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    assert_eq!(contours.contours.len(), 5);
    assert_eq!(contours.constraints.len(), 18);
    assert_eq!(
        contours.contours[0].kind,
        NodeGeneratedContourKind::FullRoadbed
    );
    assert_eq!(contours.contours[0].points_xz.len(), 4);
    assert!(!contours.contours.iter().any(|contour| {
        contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            }
            && contour.purpose == NodeGeneratedContourPurpose::CarriagewayCorridor
            && contour.source_band_index.is_none()
    }));
    assert!(contours.contours.iter().any(|contour| contour.kind
        == NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway
        }
        && contour.purpose == NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
        && contour.source_band_index == Some(2)
        && contour.contributes_to_asphalt()
        && contour.claims_asphalt_owner_region()));
    assert!(
        contours
            .constraints
            .iter()
            .any(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
    );
    assert!(
        contours
            .constraints
            .iter()
            .any(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
    );
    assert_eq!(
        contours.constraints[0].kind,
        NodeRailConstraintKind::FullRoadbedContour
    );
    assert_eq!(contours.constraints[0].constraint_index, 0);
}

#[test]
fn generated_edge_height_requires_canonical_segment_support() {
    let start = road_point_key(RoadVec2::new(0.0, 0.0));
    let end = road_point_key(RoadVec2::new(2.0, 0.0));
    let off_segment = road_point_key(RoadVec2::new(1.0, 0.5));

    assert_eq!(
        height_for_key_on_generated_edge(off_segment, start, end, 4.0, 6.0),
        None
    );
}

#[test]
fn explicit_band_domain_paths_reject_mismatched_height_carrier_lengths() {
    let mouth = OrderedIncidentPieceMouth {
        profile: profile(10.0),
        endpoint_profile: profile(0.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: vec![
            Vec::new(),
            vec![
                RoadVec3::new(10.0, 4.1, -2.0),
                RoadVec3::new(5.0, 4.2, -2.0),
                RoadVec3::new(0.0, 4.1, -2.0),
            ],
        ],
        band_end_paths_world: vec![
            Vec::new(),
            vec![
                RoadVec3::new(10.0, 4.2, 0.0),
                RoadVec3::new(7.5, 4.2, 0.0),
                RoadVec3::new(2.5, 4.2, 0.0),
                RoadVec3::new(0.0, 4.2, 0.0),
            ],
        ],
        uses_explicit_band_domain_paths: true,
        direction_angle_ccw: 0.0,
        direction_xz: RoadVec2::X,
        edge_idx: 7,
        side: IncidentEdgeSide::Start,
    };
    let input = NodeArrangementInput::from_ordered_mouths(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &[mouth],
    )
    .expect("test mouth should produce canonical input");

    let error = NodeRailContourSet::from_input(&input)
        .expect_err("mismatched explicit carriers must fail before ownership");

    assert!(matches!(
        error,
        NodeRailGenerationError::InvalidHeightCarrier {
            reason: "mismatched_path_height_carrier_lengths",
            ..
        }
    ));
}

#[test]
fn rejects_degenerate_backend_contours() {
    let error = NodeRailContourSet::from_input(&input_with_endpoint_x(10.0))
        .expect_err("zero-depth mouth should collapse its contours");

    assert!(matches!(
        error,
        NodeRailGenerationError::DegenerateContour {
            kind: NodeGeneratedContourKind::FullRoadbed,
            mouth_order_index: 0,
            band_index: None,
            ..
        }
    ));
}
