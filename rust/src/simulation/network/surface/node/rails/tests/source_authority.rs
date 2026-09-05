// SPDX-License-Identifier: GPL-2.0-only

//! Source endpoint authority tests.

use super::*;

#[test]
fn source_endpoint_authority_rejects_noncanonical_generated_contact_endpoint() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        0,
        Some(0),
        Some(asphalt_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(2.0, 0.0),
            RoadVec2::new(2.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("asphalt contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.000001, 1.0),
            RoadVec2::new(2.000001, 1.0),
            RoadVec2::new(2.000001, 2.0),
            RoadVec2::new(0.000001, 2.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("curb contour is valid");
    let generated_constraint_start_index = constraints.len();
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(curb_owner),
        points_xz: vec![RoadVec2::new(0.000001, 1.0), RoadVec2::new(2.0, 1.0)],
    });

    let drifted_start = road_point_key(RoadVec2::new(0.000001, 1.0));
    let error = validate_generated_contact_constraint_endpoints_from_sources(
        &contours,
        &constraints,
        generated_constraint_start_index,
    )
    .expect_err("generated contact endpoints must be exact source keys");

    assert!(matches!(
        error,
        NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint {
            kind: NodeRailConstraintKind::RaisedStepContact,
            mouth_order_index: 0,
            band_index: Some(0),
            owner: Some(owner),
            opposite_owner: Some(opposite_owner),
            point_x_key,
            point_z_key,
        } if owner == asphalt_owner
            && opposite_owner == curb_owner
            && point_x_key == drifted_start.0
            && point_z_key == drifted_start.1
    ));
}

#[test]
fn source_endpoint_authority_rejects_interior_segment_without_source_key() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        0,
        Some(0),
        Some(asphalt_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(2.0, 0.0),
            RoadVec2::new(2.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("asphalt contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, -1.0),
            RoadVec2::new(2.0, -1.0),
            RoadVec2::new(2.0, 0.0),
            RoadVec2::new(0.0, 0.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("curb contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 0.0), RoadVec2::new(2.0, 0.0)],
    });
    let generated_constraint_start_index = constraints.len();
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: None,
        owner: Some(asphalt_owner),
        opposite_owner: Some(curb_owner),
        points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(2.0, 0.0)],
    });

    let error = validate_generated_contact_constraint_endpoints_from_sources(
        &contours,
        &constraints,
        generated_constraint_start_index,
    )
    .expect_err("interior source-segment contact endpoints must be explicit source keys");
    assert!(matches!(
        error,
        NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint {
            kind: NodeRailConstraintKind::RaisedStepContact,
            point_x_key,
            point_z_key,
            ..
        } if (point_x_key, point_z_key) == road_point_key(RoadVec2::new(1.0, 0.0))
    ));
}

#[test]
fn source_endpoint_authority_accepts_explicitly_noded_source_key() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        0,
        Some(0),
        Some(asphalt_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(2.0, 0.0),
            RoadVec2::new(2.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("asphalt contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(1.0, -1.0),
            RoadVec2::new(2.0, -1.0),
            RoadVec2::new(2.0, 0.0),
            RoadVec2::new(1.0, 0.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("curb contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 0.0), RoadVec2::new(2.0, 0.0)],
    });
    let generated_constraint_start_index = constraints.len();
    node_generated_contact_source_constraints(
        &contours,
        &mut constraints,
        generated_constraint_start_index,
    );
    let inserted_key = road_point_key(RoadVec2::new(1.0, 0.0));
    assert!(
        constraints[..generated_constraint_start_index]
            .iter()
            .flat_map(|constraint| constraint.points_xz.iter().copied())
            .map(road_point_key)
            .any(|key| key == inserted_key)
    );
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: None,
        owner: Some(asphalt_owner),
        opposite_owner: Some(curb_owner),
        points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(2.0, 0.0)],
    });

    validate_generated_contact_constraint_endpoints_from_sources(
        &contours,
        &constraints,
        generated_constraint_start_index,
    )
    .expect("explicitly noded source keys are valid generated contact endpoints");
}
