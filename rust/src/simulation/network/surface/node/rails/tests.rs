//! Rails stage contract tests.

use super::*;
use crate::simulation::network::surface::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
};
use godot::prelude::{Vector2, Vector3};

fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
    IncidentMouthBand {
        kind,
        start_point_world: start,
        end_point_world: end,
    }
}

fn profile(x: f32) -> IncidentMouthProfile {
    let boundary_points_world = vec![
        Vector3::new(x, 4.0, -4.0),
        Vector3::new(x, 4.1, -2.0),
        Vector3::new(x, 4.2, 0.0),
        Vector3::new(x, 4.3, 2.0),
        Vector3::new(x, 4.4, 4.0),
    ];
    let bands = vec![
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[0],
            boundary_points_world[1],
        ),
        band(
            RoadSurfaceBandKind::CurbOrShoulder,
            boundary_points_world[1],
            boundary_points_world[2],
        ),
        band(
            RoadSurfaceBandKind::Carriageway,
            boundary_points_world[2],
            boundary_points_world[3],
        ),
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[3],
            boundary_points_world[4],
        ),
    ];
    IncidentMouthProfile {
        inward_direction_xz: Vector2::RIGHT,
        boundary_points_world,
        bands,
    }
}

fn terminal_profile(x: f32) -> IncidentMouthProfile {
    let boundary_points_world = vec![
        Vector3::new(x, 4.0, -4.0),
        Vector3::new(x, 4.1, -3.0),
        Vector3::new(x, 4.2, -1.0),
        Vector3::new(x, 4.0, 0.0),
        Vector3::new(x, 4.2, 1.0),
        Vector3::new(x, 4.1, 3.0),
        Vector3::new(x, 4.0, 4.0),
    ];
    let bands = vec![
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[0],
            boundary_points_world[1],
        ),
        band(
            RoadSurfaceBandKind::CurbOrShoulder,
            boundary_points_world[1],
            boundary_points_world[2],
        ),
        band(
            RoadSurfaceBandKind::Carriageway,
            boundary_points_world[2],
            boundary_points_world[3],
        ),
        band(
            RoadSurfaceBandKind::Carriageway,
            boundary_points_world[3],
            boundary_points_world[4],
        ),
        band(
            RoadSurfaceBandKind::CurbOrShoulder,
            boundary_points_world[4],
            boundary_points_world[5],
        ),
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[5],
            boundary_points_world[6],
        ),
    ];
    IncidentMouthProfile {
        inward_direction_xz: Vector2::RIGHT,
        boundary_points_world,
        bands,
    }
}

fn terminal_profile_z(z: f32) -> IncidentMouthProfile {
    let boundary_points_world = vec![
        Vector3::new(4.0, 4.0, z),
        Vector3::new(3.0, 4.1, z),
        Vector3::new(1.0, 4.2, z),
        Vector3::new(0.0, 4.0, z),
        Vector3::new(-1.0, 4.2, z),
        Vector3::new(-3.0, 4.1, z),
        Vector3::new(-4.0, 4.0, z),
    ];
    let bands = vec![
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[0],
            boundary_points_world[1],
        ),
        band(
            RoadSurfaceBandKind::CurbOrShoulder,
            boundary_points_world[1],
            boundary_points_world[2],
        ),
        band(
            RoadSurfaceBandKind::Carriageway,
            boundary_points_world[2],
            boundary_points_world[3],
        ),
        band(
            RoadSurfaceBandKind::Carriageway,
            boundary_points_world[3],
            boundary_points_world[4],
        ),
        band(
            RoadSurfaceBandKind::CurbOrShoulder,
            boundary_points_world[4],
            boundary_points_world[5],
        ),
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[5],
            boundary_points_world[6],
        ),
    ];
    IncidentMouthProfile {
        inward_direction_xz: Vector2::DOWN,
        boundary_points_world,
        bands,
    }
}

fn input_with_endpoint_x(endpoint_x: f32) -> NodeArrangementInput {
    let mouth = OrderedIncidentPieceMouth {
        profile: profile(10.0),
        endpoint_profile: profile(endpoint_x),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_sampled_band_domain_paths: false,
        direction_angle_ccw: 0.0,
        direction_xz: Vector2::RIGHT,
        edge_idx: 7,
        side: IncidentEdgeSide::Start,
    };
    NodeArrangementInput::from_ordered_mouths(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &[mouth],
    )
    .expect("test mouth should produce canonical input")
}

fn terminal_input_with_endpoint_x(endpoint_x: f32) -> NodeArrangementInput {
    let mouth = OrderedIncidentPieceMouth {
        profile: terminal_profile(10.0),
        endpoint_profile: terminal_profile(endpoint_x),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_sampled_band_domain_paths: false,
        direction_angle_ccw: 0.0,
        direction_xz: Vector2::RIGHT,
        edge_idx: 7,
        side: IncidentEdgeSide::Start,
    };
    NodeArrangementInput::from_ordered_mouths(
        42,
        RoadSurfaceVisualNodePieceKind::Terminal,
        &[mouth],
    )
    .expect("test terminal mouth should produce canonical input")
}

fn side_join_input(piece_kind: RoadSurfaceVisualNodePieceKind) -> NodeArrangementInput {
    let first = OrderedIncidentPieceMouth {
        profile: terminal_profile(10.0),
        endpoint_profile: terminal_profile(0.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_sampled_band_domain_paths: false,
        direction_angle_ccw: 0.0,
        direction_xz: Vector2::RIGHT,
        edge_idx: 7,
        side: IncidentEdgeSide::Start,
    };
    let second = OrderedIncidentPieceMouth {
        profile: terminal_profile_z(12.0),
        endpoint_profile: terminal_profile_z(2.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_sampled_band_domain_paths: false,
        direction_angle_ccw: std::f32::consts::FRAC_PI_2,
        direction_xz: Vector2::DOWN,
        edge_idx: 8,
        side: IncidentEdgeSide::Start,
    };
    NodeArrangementInput::from_ordered_mouths(42, piece_kind, &[first, second])
        .expect("test side-join mouths should produce canonical input")
}

fn nonterminal_input_with_side_join_candidate() -> NodeArrangementInput {
    side_join_input(RoadSurfaceVisualNodePieceKind::JunctionN)
}

fn bend_input_with_curb_side_join() -> NodeArrangementInput {
    side_join_input(RoadSurfaceVisualNodePieceKind::Bend)
}

fn same_owner_side_join_band() -> NodeInputSideJoinBand {
    NodeInputSideJoinBand {
        source_band_index: 3,
        band_kind: RoadSurfaceBandKind::Sidewalk,
        boundary_mode: NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap,
        inner_path_world: vec![RoadVec3::new(0.0, 4.4, 4.0), RoadVec3::new(2.0, 4.4, 4.0)],
        outer_path_world: vec![RoadVec3::new(0.9, 4.4, 6.0), RoadVec3::new(1.1, 4.4, 6.0)],
        contour_world: vec![
            RoadVec3::new(0.0, 4.4, 4.0),
            RoadVec3::new(2.0, 4.4, 4.0),
            RoadVec3::new(1.0, 4.4, 6.0),
        ],
    }
}

#[test]
fn generates_backend_contours_and_constraints_from_solved_mouth_input() {
    let contours =
        NodeRailContourSet::from_input(&input_with_endpoint_x(0.0)).expect("valid contours");

    assert_eq!(contours.node_id, 42);
    assert_eq!(
        contours.piece_kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    assert_eq!(contours.contours.len(), 6);
    assert_eq!(contours.constraints.len(), 19);
    assert_eq!(
        contours.contours[0].kind,
        NodeGeneratedContourKind::FullRoadbed
    );
    assert_eq!(contours.contours[0].points_xz.len(), 4);
    assert!(contours.contours.iter().any(|contour| {
        contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            }
            && contour.purpose == NodeGeneratedContourPurpose::CarriagewayCorridor
            && contour.source_mouth_order_index == 0
            && contour.source_band_index.is_none()
            && contour.owner.is_none()
            && contour.contributes_to_asphalt()
            && !contour.claims_asphalt_owner_region()
    }));
    assert!(contours.contours.iter().any(|contour| contour.kind
        == NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway
        }
        && contour.purpose == NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
        && contour.source_band_index == Some(2)
        && !contour.contributes_to_asphalt()
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
fn nonterminal_side_join_bands_emit_canonical_ownership_candidates() {
    let input = nonterminal_input_with_side_join_candidate();
    let contours = NodeRailContourSet::from_input(&input).expect("valid contours");
    let junction_side_join_contours = contours
        .contours
        .iter()
        .filter(|contour| contour.purpose == NodeGeneratedContourPurpose::JunctionSideJoin)
        .collect::<Vec<_>>();
    let raw_full_roadbed_corridors = contours
        .contours
        .iter()
        .filter(|contour| contour.purpose == NodeGeneratedContourPurpose::FullRoadbedCorridor)
        .collect::<Vec<_>>();
    assert_eq!(raw_full_roadbed_corridors.len(), input.mouths.len());
    let raw_carriageway_corridors = contours
        .contours
        .iter()
        .filter(|contour| {
            contour.purpose == NodeGeneratedContourPurpose::CarriagewayCorridor
                && contour.source_band_index.is_none()
        })
        .collect::<Vec<_>>();
    assert_eq!(raw_carriageway_corridors.len(), input.mouths.len());
    for mouth in &input.mouths {
        assert!(
            raw_full_roadbed_corridors
                .iter()
                .any(|contour| contour.source_mouth_order_index == mouth.order_index),
            "each non-terminal mouth must emit exactly one raw full-roadbed authority corridor"
        );
        assert!(
            raw_carriageway_corridors
                .iter()
                .any(|contour| contour.source_mouth_order_index == mouth.order_index),
            "each non-terminal mouth must emit exactly one raw carriageway authority corridor"
        );
    }
    let expected_carriageway_owner_carriers = input
        .mouths
        .iter()
        .flat_map(|mouth| mouth.band_intervals.iter())
        .filter(|interval| interval.band_kind == RoadSurfaceBandKind::Carriageway)
        .count();
    assert_eq!(
        contours
            .contours
            .iter()
            .filter(|contour| {
                contour.purpose == NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
                    && contour.claims_asphalt_owner_region()
                    && !contour.contributes_to_asphalt()
            })
            .count(),
        expected_carriageway_owner_carriers
    );

    assert!(contours.contours.iter().any(|contour| {
        contour.kind == NodeGeneratedContourKind::FullRoadbed
            && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
            && contour.source_mouth_order_index == 0
    }));
    assert!(contours.contours.iter().any(|contour| {
        contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Sidewalk,
            }
            && contour.purpose == NodeGeneratedContourPurpose::JunctionSideJoin
            && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
            && contour.source_mouth_order_index == 0
            && contour.source_band_index == Some(5)
    }));
    assert!(!junction_side_join_contours.is_empty());
    assert!(junction_side_join_contours.iter().all(
        |contour| !contour.contributes_to_footprint() && !contour.contributes_to_asphalt()
    ));
    assert!(contours.constraints.iter().any(|constraint| {
        matches!(
            constraint.kind,
            NodeRailConstraintKind::FootprintSeam {
                adjacent_kind: RoadSurfaceBandKind::Sidewalk
            }
        ) && constraint.source_band_index == Some(5)
    }));
}

#[test]
fn bend_curb_side_join_bands_contribute_canonical_footprint() {
    let contours =
        NodeRailContourSet::from_input(&bend_input_with_curb_side_join()).expect("valid contours");

    assert!(contours.contours.iter().any(|contour| {
        contour.kind == NodeGeneratedContourKind::FullRoadbed
            && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
            && contour.source_mouth_order_index == 0
    }));
    assert!(contours.contours.iter().any(|contour| {
        contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            }
            && contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
            && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
            && contour.source_band_index == Some(4)
    }));
}

#[test]
fn bend_side_join_contacts_name_exact_adjacent_owner_pair() {
    let contours =
        NodeRailContourSet::from_input(&bend_input_with_curb_side_join()).expect("valid contours");

    let side_join_owner = contours
        .contours
        .iter()
        .find(|contour| {
            contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
                && contour.source_band_index == Some(4)
        })
        .and_then(|contour| contour.owner)
        .expect("side-join contour should have a band owner");
    assert!(contours.constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && constraint.source_band_index == Some(4)
            && (constraint.owner == Some(side_join_owner)
                || constraint.opposite_owner == Some(side_join_owner))
            && generated_constraint_opposite_owner(constraint, side_join_owner)
                .is_some_and(|owner| owner.kind() == RoadSurfaceBandKind::Carriageway)
    }));
    assert!(!contours.constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && constraint.source_band_index == Some(4)
            && (constraint.owner == Some(side_join_owner)
                || constraint.opposite_owner == Some(side_join_owner))
            && constraint.opposite_owner.is_none()
    }));
}

#[test]
fn generated_contact_rejects_non_exact_owner_pair_authority() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
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
        Some(actual_curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 1.0),
            RoadVec2::new(2.0, 1.0),
            RoadVec2::new(2.0, 2.0),
            RoadVec2::new(0.0, 2.0),
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
        source_band_index: None,
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(source_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(2.0, 1.0)],
    });

    append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    assert!(!constraints.iter().any(|constraint| {
        let start = road_point_key(RoadVec2::new(0.0, 1.0));
        let end = road_point_key(RoadVec2::new(2.0, 1.0));
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                asphalt_owner,
                actual_curb_owner,
            )
            && road_point_key(constraint.points_xz[0]) == start
            && road_point_key(constraint.points_xz[1]) == end
    }));
}

#[test]
fn sampled_band_domain_paths_reject_mismatched_height_carrier_lengths() {
    let mouth = OrderedIncidentPieceMouth {
        profile: profile(10.0),
        endpoint_profile: profile(0.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: vec![
            Vec::new(),
            vec![
                Vector3::new(10.0, 4.1, -2.0),
                Vector3::new(5.0, 4.2, -2.0),
                Vector3::new(0.0, 4.1, -2.0),
            ],
        ],
        band_end_paths_world: vec![
            Vec::new(),
            vec![
                Vector3::new(10.0, 4.2, 0.0),
                Vector3::new(7.5, 4.2, 0.0),
                Vector3::new(2.5, 4.2, 0.0),
                Vector3::new(0.0, 4.2, 0.0),
            ],
        ],
        uses_sampled_band_domain_paths: true,
        direction_angle_ccw: 0.0,
        direction_xz: Vector2::RIGHT,
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
        .expect_err("mismatched sampled carriers must fail before ownership");

    assert!(matches!(
        error,
        NodeRailGenerationError::InvalidHeightCarrier {
            reason: "mismatched_path_height_carrier_lengths",
            ..
        }
    ));
}

#[test]
fn bend_side_join_point_contact_reowns_exact_source_rail_by_band_kind() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(actual_curb_owner),
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(2.0, 0.5),
            RoadVec2::new(4.0, 0.5),
            RoadVec2::new(4.0, 1.5),
            RoadVec2::new(2.0, 1.5),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("curb side join contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(source_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
    });

    let mut junction_constraints = constraints.clone();
    append_source_authorized_raised_step_point_contacts(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        &mut junction_constraints,
    );
    assert!(!junction_constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                asphalt_owner,
                actual_curb_owner,
            )
    }));

    append_source_authorized_raised_step_point_contacts(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    let start = road_point_key(RoadVec2::new(2.0, 1.0));
    let end = road_point_key(RoadVec2::new(4.0, 1.0));
    for point in [start, end] {
        assert!(constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
                && constraint.points_xz.len() == 2
                && road_point_key(constraint.points_xz[0]) == point
                && road_point_key(constraint.points_xz[1]) == point
        }));
    }
    assert!(!constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                asphalt_owner,
                actual_curb_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == start
            && road_point_key(constraint.points_xz[1]) == end
    }));
}

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

#[test]
fn generated_raised_step_owner_pair_splits_carriageway_boundary_at_overlay_contact() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
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
            RoadVec2::new(4.0, 0.0),
            RoadVec2::new(4.0, 1.0),
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
        Some(actual_curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(3.0, 0.5),
            RoadVec2::new(4.0, 0.5),
            RoadVec2::new(4.0, 1.5),
            RoadVec2::new(3.0, 1.5),
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
        source_band_index: None,
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(actual_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
    });

    append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    let start = road_point_key(RoadVec2::new(3.0, 1.0));
    let end = road_point_key(RoadVec2::new(4.0, 1.0));
    assert!(constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                asphalt_owner,
                actual_curb_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == start
            && road_point_key(constraint.points_xz[1]) == end
    }));
}

#[test]
fn generated_raised_step_owner_pair_splits_curb_sidewalk_boundary_at_overlay_contact() {
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(0),
        Some(curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(4.0, 0.0),
            RoadVec2::new(4.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("curb contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        0,
        Some(1),
        Some(sidewalk_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        vec![
            RoadVec2::new(3.0, 0.5),
            RoadVec2::new(4.0, 0.5),
            RoadVec2::new(4.0, 1.5),
            RoadVec2::new(3.0, 1.5),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("sidewalk contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(sidewalk_owner),
        opposite_owner: Some(curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
    });

    append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    let start = road_point_key(RoadVec2::new(3.0, 1.0));
    let end = road_point_key(RoadVec2::new(4.0, 1.0));
    assert!(constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                curb_owner,
                sidewalk_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == start
            && road_point_key(constraint.points_xz[1]) == end
    }));
}

#[test]
fn generated_raised_step_owner_pair_uses_source_authority_union_for_split_domains() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
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
            RoadVec2::new(4.0, 0.0),
            RoadVec2::new(4.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("asphalt contour is valid");
    for points in [
        vec![
            RoadVec2::new(2.0, 0.5),
            RoadVec2::new(3.2, 0.5),
            RoadVec2::new(3.2, 1.5),
            RoadVec2::new(2.0, 1.5),
        ],
        vec![
            RoadVec2::new(2.8, 0.5),
            RoadVec2::new(4.0, 0.5),
            RoadVec2::new(4.0, 1.5),
            RoadVec2::new(2.8, 1.5),
        ],
    ] {
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            0,
            Some(1),
            Some(actual_curb_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            points,
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb contour is valid");
    }
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(actual_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
    });

    append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    let start = road_point_key(RoadVec2::new(2.0, 1.0));
    let end = road_point_key(RoadVec2::new(4.0, 1.0));
    assert!(constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                asphalt_owner,
                actual_curb_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == start
            && road_point_key(constraint.points_xz[1]) == end
    }));
}

#[test]
fn nonterminal_same_owner_caps_emit_canonical_side_join_fill() {
    let input = input_with_endpoint_x(0.0);
    let side_join_bands = vec![same_owner_side_join_band()];
    let owners_by_mouth = owners_by_mouth(&input, &[], std::slice::from_ref(&side_join_bands));
    let mut contours = Vec::new();
    let mut constraints = Vec::new();
    push_side_join_band_contours(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &input.mouths[0],
        &side_join_bands,
        &owners_by_mouth[0],
        &owners_by_mouth[0].side_join_band_owners,
        &mut contours,
        &mut constraints,
    )
    .expect("valid side-join contours");
    let cap_tip = road_point_key(RoadVec2::new(1.0, 6.0));

    assert!(
        NodeGeneratedContourClaimPriority::SideJoin < NodeGeneratedContourClaimPriority::MouthBand,
        "side-join candidates must stay ahead of mouth bands during non-road ownership cleanup"
    );
    assert!(!contours.iter().any(|contour| {
        contour.kind == NodeGeneratedContourKind::FullRoadbed
            && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
            && contour
                .points_xz
                .iter()
                .any(|point| road_point_key(*point) == cap_tip)
    }));
    assert!(contours.iter().any(|contour| {
        contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Sidewalk,
            }
            && contour.purpose == NodeGeneratedContourPurpose::JunctionSideJoin
            && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
            && contour.source_band_index == Some(3)
            && contour
                .points_xz
                .iter()
                .any(|point| road_point_key(*point) == cap_tip)
    }));
}

#[test]
fn terminal_cap_contacts_name_adjacent_owner_pairs() {
    let input = terminal_input_with_endpoint_x(0.0);
    let terminal_curb_source = input.mouths[0].band_intervals.len();
    let contours = NodeRailContourSet::from_input(&input).expect("valid terminal contours");
    let terminal_curb_owner = contours
        .contours
        .iter()
        .find(|contour| {
            contour.purpose == NodeGeneratedContourPurpose::TerminalCap
                && contour.source_band_index == Some(terminal_curb_source)
        })
        .and_then(|contour| contour.owner)
        .expect("terminal cap should have an owner");
    let left_carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let right_carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 3);
    let left_segment = GeneratedContourEdgeKey::new(
        road_point_key(RoadVec2::new(0.0, -1.0)),
        road_point_key(RoadVec2::new(0.0, 0.0)),
    );
    let right_segment = GeneratedContourEdgeKey::new(
        road_point_key(RoadVec2::new(0.0, 0.0)),
        road_point_key(RoadVec2::new(0.0, 1.0)),
    );
    let contacts = contours
        .constraints
        .iter()
        .filter(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && constraint.source_band_index == Some(terminal_curb_source)
                && (constraint.owner == Some(terminal_curb_owner)
                    || constraint.opposite_owner == Some(terminal_curb_owner))
        })
        .filter_map(|constraint| {
            let opposite_owner =
                generated_constraint_opposite_owner(constraint, terminal_curb_owner)?;
            Some((
                GeneratedContourEdgeKey::new(
                    road_point_key(constraint.points_xz[0]),
                    road_point_key(constraint.points_xz[1]),
                ),
                opposite_owner,
            ))
        })
        .collect::<BTreeSet<_>>();

    assert!(contacts.contains(&(left_segment, left_carriageway)));
    assert!(contacts.contains(&(right_segment, right_carriageway)));
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
