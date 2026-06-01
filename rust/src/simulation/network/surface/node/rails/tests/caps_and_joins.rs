//! Rail cap and side-join ownership tests.

use super::*;

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
            && contour.source_band_index == Some(5)
    }));
    assert!(!junction_side_join_contours.is_empty());
    assert!(
        junction_side_join_contours
            .iter()
            .filter(|contour| matches!(
                contour.kind,
                NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk,
                }
            ))
            .all(|contour| !contour.contributes_to_footprint()
                && !contour.contributes_to_asphalt())
    );
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
            let opposite_owner = constraint_opposite_owner(constraint, terminal_curb_owner)?;
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
