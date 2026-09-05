// SPDX-License-Identifier: GPL-2.0-only

//! Generated contact materialization tests.

use super::*;

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
            && constraint_opposite_owner(constraint, side_join_owner)
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
        constraints.len(),
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
        junction_constraints.len(),
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
        constraints.len(),
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
fn junction_mouth_band_point_contact_reowns_exact_source_endpoint_by_band_kind() {
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 7);
    let source_sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
    let target_sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 6);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        0,
        Some(2),
        Some(target_sidewalk_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
            RoadVec2::new(1.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("target sidewalk contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(2),
        source_boundary_index: Some(1),
        owner: Some(curb_owner),
        opposite_owner: Some(source_sidewalk_owner),
        points_xz: vec![RoadVec2::new(-1.0, 0.0), RoadVec2::new(0.0, 0.0)],
    });

    append_source_authorized_raised_step_point_contacts(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        constraints.len(),
        &mut constraints,
    );

    let point = road_point_key(RoadVec2::new(0.0, 0.0));
    assert!(constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                curb_owner,
                target_sidewalk_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == point
            && road_point_key(constraint.points_xz[1]) == point
    }));
}

#[test]
fn junction_mouth_band_edge_contact_reowns_exact_source_rail_by_band_kind() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let target_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(target_curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
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
    .expect("target curb mouth-band contour is valid");
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

    append_source_authorized_raised_step_point_contacts(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        constraints.len(),
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
                target_curb_owner,
            )
            && constraint.points_xz.len() == 2
            && GeneratedContourEdgeKey::new(
                road_point_key(constraint.points_xz[0]),
                road_point_key(constraint.points_xz[1]),
            ) == GeneratedContourEdgeKey::new(start, end)
    }));
}

#[test]
fn source_authorized_contact_cache_matches_cold_and_invalidates_changed_source() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let target_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
    let mut contours = Vec::new();
    let mut source_constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(target_curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
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
        &mut source_constraints,
    )
    .expect("target curb mouth-band contour is valid");
    source_constraints.push(NodeRailConstraint {
        constraint_index: source_constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(source_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
    });
    source_constraints.push(NodeRailConstraint {
        constraint_index: source_constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 1,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(source_curb_owner),
        points_xz: vec![RoadVec2::new(3.0, -1.0), RoadVec2::new(3.0, 3.0)],
    });
    let source_constraint_count = source_constraints.len();

    let mut first_constraints = source_constraints.clone();
    let mut first_cache = NodeSourceAuthorizedContactCache::default();
    let (_, first_stats) = append_source_authorized_raised_step_point_contacts_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraint_count,
        &mut first_constraints,
        None,
        &mut first_cache,
    );
    assert!(first_stats.source_cache_misses >= 2);
    assert!(first_stats.source_pair_cache_misses > 0);

    let mut reused_constraints = source_constraints.clone();
    let mut reused_cache = NodeSourceAuthorizedContactCache::default();
    let (_, reused_stats) = append_source_authorized_raised_step_point_contacts_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraint_count,
        &mut reused_constraints,
        Some(&first_cache),
        &mut reused_cache,
    );
    assert_eq!(reused_constraints, first_constraints);
    assert!(reused_stats.target_group_cache_hits > 0);
    assert!(reused_stats.source_cache_hits >= 2);
    assert!(reused_stats.source_pair_cache_hits > 0);

    let mut second_pass_input = first_constraints.clone();
    second_pass_input.push(NodeRailConstraint {
        constraint_index: second_pass_input.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 2,
        source_band_index: Some(1),
        source_boundary_index: None,
        owner: Some(asphalt_owner),
        opposite_owner: Some(source_curb_owner),
        points_xz: vec![RoadVec2::new(2.5, -1.0), RoadVec2::new(2.5, 3.0)],
    });
    let mut second_pass_reused = second_pass_input.clone();
    let mut current_generation_cache = first_cache.clone();
    let (_, second_pass_reused_stats) =
        append_source_authorized_raised_step_point_contacts_with_reuse(
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &contours,
            source_constraint_count,
            &mut second_pass_reused,
            None,
            &mut current_generation_cache,
        );
    let mut second_pass_cold = second_pass_input;
    let mut second_pass_cold_cache = NodeSourceAuthorizedContactCache::default();
    let (_, second_pass_cold_stats) =
        append_source_authorized_raised_step_point_contacts_with_reuse(
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &contours,
            source_constraint_count,
            &mut second_pass_cold,
            None,
            &mut second_pass_cold_cache,
        );
    assert_eq!(second_pass_reused, second_pass_cold);
    assert_eq!(
        second_pass_reused_stats.source_cache_hits, 0,
        "unchanged source/group contributors should not be enumerated again"
    );
    assert!(
        second_pass_reused_stats.source_pair_cache_misses
            < second_pass_cold_stats.source_pair_cache_misses,
        "the second pass should inspect only pairs touching its new source contributors"
    );
    assert!(
        second_pass_reused_stats.source_cache_misses < second_pass_cold_stats.source_cache_misses,
        "unchanged source/group contributions should not rerun"
    );

    let mut changed_sources = source_constraints;
    changed_sources[2].points_xz = vec![RoadVec2::new(2.5, -1.0), RoadVec2::new(2.5, 3.0)];
    let mut changed_reused_constraints = changed_sources.clone();
    let mut changed_reused_cache = NodeSourceAuthorizedContactCache::default();
    let (_, changed_reused_stats) = append_source_authorized_raised_step_point_contacts_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraint_count,
        &mut changed_reused_constraints,
        Some(&first_cache),
        &mut changed_reused_cache,
    );
    let mut changed_cold_constraints = changed_sources;
    let mut changed_cold_cache = NodeSourceAuthorizedContactCache::default();
    append_source_authorized_raised_step_point_contacts_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraint_count,
        &mut changed_cold_constraints,
        None,
        &mut changed_cold_cache,
    );

    assert_eq!(changed_reused_constraints, changed_cold_constraints);
    assert!(changed_reused_stats.target_group_cache_hits > 0);
    assert!(changed_reused_stats.source_cache_hits > 0);
    assert!(changed_reused_stats.source_cache_misses > 0);
    assert!(changed_reused_stats.source_pair_cache_misses > 0);
}

fn raised_step_pair_cache_input(
    carriageway_points: Vec<RoadVec2>,
    curb_points: Vec<RoadVec2>,
    authority_points: Vec<RoadVec2>,
) -> (
    Vec<NodeGeneratedContour>,
    Vec<NodeRailConstraint>,
    NodeBandOwner,
    NodeBandOwner,
) {
    let carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        0,
        Some(0),
        Some(carriageway_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        carriageway_points,
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("carriageway contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        1,
        Some(1),
        Some(curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        curb_points,
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
        owner: Some(carriageway_owner),
        opposite_owner: Some(curb_owner),
        points_xz: authority_points,
    });
    (contours, constraints, carriageway_owner, curb_owner)
}

fn adjacent_raised_step_pair_cache_input() -> (
    Vec<NodeGeneratedContour>,
    Vec<NodeRailConstraint>,
    NodeBandOwner,
    NodeBandOwner,
) {
    raised_step_pair_cache_input(
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(2.0, 0.0),
            RoadVec2::new(2.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        vec![
            RoadVec2::new(0.0, 1.0),
            RoadVec2::new(2.0, 1.0),
            RoadVec2::new(2.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(2.0, 1.0)],
    )
}

#[test]
fn raised_step_pair_cache_matches_cold_and_drops_removed_pairs() {
    let (contours, source_constraints, carriageway_owner, curb_owner) =
        adjacent_raised_step_pair_cache_input();
    let source_constraint_count = source_constraints.len();
    let mut first = source_constraints.clone();
    let (first_stats, pair_cache) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraint_count,
        &mut first,
        None,
    );
    assert_eq!(first_stats.raised_step_pair_cache_previous_hits, 0);
    assert_eq!(first_stats.raised_step_pair_cache_misses, 1);
    assert!(
        first
            .iter()
            .skip(source_constraint_count)
            .any(|constraint| {
                constraint.kind == NodeRailConstraintKind::RaisedStepContact
                    && owners_match_unordered(
                        constraint.owner,
                        constraint.opposite_owner,
                        carriageway_owner,
                        curb_owner,
                    )
            })
    );

    let mut warm = source_constraints.clone();
    let (warm_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraint_count,
        &mut warm,
        Some(&pair_cache),
    );
    let mut cold = source_constraints.clone();
    let (cold_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraint_count,
        &mut cold,
        None,
    );
    assert_eq!(warm, cold);
    assert_eq!(warm, first);
    assert_eq!(warm_stats.raised_step_pair_cache_previous_hits, 1);
    assert_eq!(warm_stats.raised_step_pair_cache_misses, 0);
    assert_eq!(cold_stats.raised_step_pair_cache_previous_hits, 0);
    assert_eq!(cold_stats.raised_step_pair_cache_misses, 1);

    let reversed_contours = vec![contours[1].clone(), contours[0].clone()];
    let mut reversed_warm = source_constraints.clone();
    let (reversed_warm_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &reversed_contours,
        source_constraint_count,
        &mut reversed_warm,
        Some(&pair_cache),
    );
    let mut reversed_cold = source_constraints.clone();
    append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &reversed_contours,
        source_constraint_count,
        &mut reversed_cold,
        None,
    );
    assert_eq!(reversed_warm, reversed_cold);
    assert_eq!(
        reversed_warm_stats.raised_step_pair_cache_previous_hits, 1,
        "contour slice order must not change the canonical pair key"
    );

    let removed_contours = vec![contours[0].clone()];
    let removed_source_constraints = vec![source_constraints[0].clone()];
    let mut removed_warm = removed_source_constraints.clone();
    let (removed_warm_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &removed_contours,
        removed_source_constraints.len(),
        &mut removed_warm,
        Some(&pair_cache),
    );
    let mut removed_cold = removed_source_constraints.clone();
    append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &removed_contours,
        removed_source_constraints.len(),
        &mut removed_cold,
        None,
    );
    assert_eq!(removed_warm, removed_cold);
    assert_eq!(removed_warm, removed_source_constraints);
    assert_eq!(removed_warm_stats.raised_step_pair_cache_previous_hits, 0);
    assert_eq!(removed_warm_stats.raised_step_pair_cache_misses, 0);
}

#[test]
fn raised_step_pair_cache_invalidates_relevant_authority_only() {
    let (contours, source_constraints, carriageway_owner, curb_owner) =
        adjacent_raised_step_pair_cache_input();
    let mut first = source_constraints.clone();
    let (_, pair_cache) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraints.len(),
        &mut first,
        None,
    );

    let mut changed_sources = source_constraints.clone();
    changed_sources
        .last_mut()
        .expect("raised-step authority")
        .source_mouth_order_index = 7;
    let mut changed_warm = changed_sources.clone();
    let (changed_warm_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        changed_sources.len(),
        &mut changed_warm,
        Some(&pair_cache),
    );
    let mut changed_cold = changed_sources.clone();
    append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        changed_sources.len(),
        &mut changed_cold,
        None,
    );
    assert_eq!(changed_warm, changed_cold);
    assert_eq!(changed_warm_stats.raised_step_pair_cache_previous_hits, 0);
    assert_eq!(changed_warm_stats.raised_step_pair_cache_misses, 1);
    let changed_generated = changed_warm
        .iter()
        .skip(changed_sources.len())
        .filter(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    carriageway_owner,
                    curb_owner,
                )
        })
        .collect::<Vec<_>>();
    assert!(!changed_generated.is_empty());
    assert!(
        changed_generated
            .into_iter()
            .all(|constraint| constraint.source_mouth_order_index == 7),
        "a relevant authority metadata change must not replay the previous source label"
    );

    let mut distant_sources = source_constraints.clone();
    distant_sources.push(NodeRailConstraint {
        constraint_index: distant_sources.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 9,
        source_band_index: Some(9),
        source_boundary_index: Some(9),
        owner: Some(carriageway_owner),
        opposite_owner: Some(curb_owner),
        points_xz: vec![RoadVec2::new(100.0, 100.0), RoadVec2::new(101.0, 100.0)],
    });
    let mut distant_warm = distant_sources.clone();
    let (distant_warm_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        distant_sources.len(),
        &mut distant_warm,
        Some(&pair_cache),
    );
    let mut distant_cold = distant_sources.clone();
    append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        distant_sources.len(),
        &mut distant_cold,
        None,
    );
    assert_eq!(distant_warm, distant_cold);
    assert_eq!(
        distant_warm_stats.raised_step_pair_cache_previous_hits, 1,
        "authority outside both contour bounds must not invalidate the local pair"
    );
    assert_eq!(distant_warm_stats.raised_step_pair_cache_misses, 0);

    let unrelated_carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 20);
    let unrelated_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 21);
    let mut unrelated_owner_sources = source_constraints;
    unrelated_owner_sources.push(NodeRailConstraint {
        constraint_index: unrelated_owner_sources.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 10,
        source_band_index: Some(10),
        source_boundary_index: Some(10),
        owner: Some(unrelated_carriageway_owner),
        opposite_owner: Some(unrelated_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(2.0, 1.0)],
    });
    let mut unrelated_owner_warm = unrelated_owner_sources.clone();
    let (unrelated_owner_warm_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        unrelated_owner_sources.len(),
        &mut unrelated_owner_warm,
        Some(&pair_cache),
    );
    let mut unrelated_owner_cold = unrelated_owner_sources.clone();
    append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        unrelated_owner_sources.len(),
        &mut unrelated_owner_cold,
        None,
    );
    assert_eq!(unrelated_owner_warm, unrelated_owner_cold);
    assert_eq!(
        unrelated_owner_warm_stats.raised_step_pair_cache_previous_hits, 1,
        "local authority for another owner pair must not enter this pair's fingerprint"
    );
    assert_eq!(unrelated_owner_warm_stats.raised_step_pair_cache_misses, 0);
}

#[test]
fn raised_step_pair_cache_misses_on_positional_owner_rebind() {
    let (mut contours, mut source_constraints, carriageway_owner, curb_owner) =
        adjacent_raised_step_pair_cache_input();
    let mut first = source_constraints.clone();
    let (_, pair_cache) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraints.len(),
        &mut first,
        None,
    );

    let remapped_carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 10);
    let remapped_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11);
    for contour in &mut contours {
        contour.owner = match contour.owner {
            Some(owner) if owner == carriageway_owner => Some(remapped_carriageway_owner),
            Some(owner) if owner == curb_owner => Some(remapped_curb_owner),
            owner => owner,
        };
    }
    for constraint in &mut source_constraints {
        for owner in [&mut constraint.owner, &mut constraint.opposite_owner] {
            *owner = match *owner {
                Some(owner) if owner == carriageway_owner => Some(remapped_carriageway_owner),
                Some(owner) if owner == curb_owner => Some(remapped_curb_owner),
                owner => owner,
            };
        }
    }

    let mut warm = source_constraints.clone();
    let (warm_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraints.len(),
        &mut warm,
        Some(&pair_cache),
    );
    let mut cold = source_constraints.clone();
    append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraints.len(),
        &mut cold,
        None,
    );
    assert_eq!(warm, cold);
    assert_eq!(warm_stats.raised_step_pair_cache_previous_hits, 0);
    assert_eq!(warm_stats.raised_step_pair_cache_misses, 1);
    assert!(
        warm.iter()
            .skip(source_constraints.len())
            .any(|constraint| {
                owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    remapped_carriageway_owner,
                    remapped_curb_owner,
                )
            })
    );
}

#[test]
fn raised_step_pair_cache_reuses_empty_exact_contribution() {
    let (contours, source_constraints, _, _) = raised_step_pair_cache_input(
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(2.0, 0.0),
            RoadVec2::new(0.0, 2.0),
        ],
        vec![
            RoadVec2::new(2.0, 2.0),
            RoadVec2::new(0.6, 2.0),
            RoadVec2::new(2.0, 0.6),
        ],
        vec![RoadVec2::new(1.3, 1.3), RoadVec2::new(1.3, 1.3)],
    );
    let mut first = source_constraints.clone();
    let (first_stats, pair_cache) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraints.len(),
        &mut first,
        None,
    );
    assert_eq!(first_stats.raised_step_pair_cache_previous_hits, 0);
    assert_eq!(first_stats.raised_step_pair_cache_misses, 1);
    assert!(first_stats.overlay_calls > 0);
    assert_eq!(first, source_constraints);

    let mut warm = source_constraints.clone();
    let (warm_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraints.len(),
        &mut warm,
        Some(&pair_cache),
    );
    assert_eq!(warm, first);
    assert_eq!(warm_stats.raised_step_pair_cache_previous_hits, 1);
    assert_eq!(warm_stats.raised_step_pair_cache_misses, 0);
    assert_eq!(warm_stats.overlay_calls, 0);
}

#[test]
fn same_material_point_contact_emits_height_split_constraint() {
    let first_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let second_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(first_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("first curb contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(4),
        Some(second_owner),
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(-1.0, 0.0),
            RoadVec2::new(0.0, -1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("second curb contour is valid");

    append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        constraints.len(),
        &mut constraints,
    );

    let point = road_point_key(RoadVec2::new(0.0, 0.0));
    assert!(constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                first_owner,
                second_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == point
            && road_point_key(constraint.points_xz[1]) == point
    }));
}

#[test]
fn same_material_edge_contact_emits_height_split_constraint() {
    let first_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let second_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(first_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
            RoadVec2::new(1.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("first curb contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(4),
        Some(second_owner),
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
            RoadVec2::new(1.0, -1.0),
            RoadVec2::new(0.0, -1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("second curb contour is valid");

    let source_constraint_count = constraints.len();
    let source_constraints = constraints.clone();
    let (first_stats, same_material_pair_cache) =
        append_generated_same_band_contact_constraints_with_reuse(
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &contours,
            source_constraint_count,
            &mut constraints,
            None,
        );

    let start = road_point_key(RoadVec2::new(0.0, 0.0));
    let end = road_point_key(RoadVec2::new(1.0, 0.0));
    assert!(constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                first_owner,
                second_owner,
            )
            && constraint.points_xz.len() == 2
            && GeneratedContourEdgeKey::new(
                road_point_key(constraint.points_xz[0]),
                road_point_key(constraint.points_xz[1]),
            ) == GeneratedContourEdgeKey::new(start, end)
    }));

    let first_constraints = constraints;
    let mut reused_constraints = source_constraints;
    let (reused_stats, _) = append_generated_same_band_contact_constraints_with_reuse(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        source_constraint_count,
        &mut reused_constraints,
        Some(&same_material_pair_cache),
    );
    assert_eq!(reused_constraints, first_constraints);
    assert_eq!(first_stats.same_material_pair_cache_hits, 0);
    assert!(first_stats.same_material_overlay_calls > 0);
    assert!(reused_stats.same_material_pair_cache_hits > 0);
    assert!(
        reused_stats.same_material_overlay_calls < first_stats.same_material_overlay_calls,
        "cache hit should avoid the same-material overlay"
    );
}

#[test]
fn same_material_same_source_authority_skips_duplicate_height_split() {
    let first_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let second_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(first_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
            RoadVec2::new(1.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 1.0),
            RoadVec3::new(0.0, 0.0, 1.0),
        ]),
        &mut contours,
        &mut constraints,
    )
    .expect("first curb contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(second_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
            RoadVec2::new(1.0, -1.0),
            RoadVec2::new(0.0, -1.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, -1.0),
            RoadVec3::new(0.0, 0.0, -1.0),
        ]),
        &mut contours,
        &mut constraints,
    )
    .expect("second curb contour is valid");

    let before_len = constraints.len();
    let stats = append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        constraints.len(),
        &mut constraints,
    );

    assert_eq!(stats.same_authority_skipped, 1);
    assert_eq!(stats.same_material_candidate_pairs, 0);
    assert_eq!(constraints.len(), before_len);
}

#[test]
fn source_authorized_point_contact_uses_deterministic_source_name() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let shared_point = RoadVec2::new(0.0, 0.0);
    let mut constraints = vec![
        NodeRailConstraint {
            constraint_index: 0,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(asphalt_owner),
            opposite_owner: Some(curb_owner),
            points_xz: vec![shared_point, RoadVec2::new(1.0, 0.0)],
        },
        NodeRailConstraint {
            constraint_index: 1,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 1,
            source_band_index: Some(2),
            source_boundary_index: Some(2),
            owner: Some(asphalt_owner),
            opposite_owner: Some(curb_owner),
            points_xz: vec![shared_point, RoadVec2::new(0.0, 1.0)],
        },
    ];

    append_source_authorized_raised_step_point_contacts(
        RoadSurfaceVisualNodePieceKind::Bend,
        &[],
        constraints.len(),
        &mut constraints,
    );

    assert!(constraints.iter().skip(2).any(|constraint| {
        constraint.source_mouth_order_index == 0
            && constraint.source_band_index == Some(1)
            && constraint.points_xz == vec![shared_point, shared_point]
    }));
}
