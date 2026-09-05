//! Rail cap and side-join ownership tests.

use super::*;
use crate::simulation::network::surface::{
    NodeOverlayContour, NodeOverlayShapes, RoadSurfaceSystem,
};
use i_overlay::core::overlay_rule::OverlayRule;

#[test]
fn junction_topology_reuse_uses_node_local_mouth_identity() {
    let input = side_join_input(RoadSurfaceVisualNodePieceKind::JunctionN);
    let (_, _, _, topology) =
        NodeRailContourSet::from_input_with_profile_and_topology_reuse(&input, false, None)
            .expect("cold junction rails");
    let mut remapped_input = input.clone();
    remapped_input.mouths[0].edge_idx = 70;
    remapped_input.mouths[1].edge_idx = 80;

    let (reused, _, reuse_status, _) =
        NodeRailContourSet::from_input_with_profile_and_topology_reuse(
            &remapped_input,
            false,
            Some(&topology),
        )
        .expect("edge-id-remapped junction rails");
    let (cold, _, _, _) = NodeRailContourSet::from_input_with_profile_and_topology_reuse(
        &remapped_input,
        false,
        None,
    )
    .expect("independent remapped junction rails");

    assert!(
        reuse_status.rail_topology_reused,
        "raw graph edge ids are publication metadata, not node-local rail topology"
    );
    assert!(
        reuse_status.ownership_reuse_safe,
        "an edge-id-only remap preserves every ownership predicate"
    );
    assert!(reuse_status.arrangement_reuse_safe);
    assert_eq!(reused.side_join_gaps, cold.side_join_gaps);
    assert!(!reused.side_join_gaps.is_empty());
    assert!(
        reused.side_join_gaps.iter().all(|gap| {
            [70, 80].contains(&gap.from_edge_idx) && [70, 80].contains(&gap.to_edge_idx)
        }),
        "projected joins must carry the current generation's graph edge ids"
    );
}

#[test]
fn terminal_topology_reuse_uses_node_local_mouth_identity() {
    let input = terminal_input_with_endpoint_x(0.0);
    let (_, _, _, topology) =
        NodeRailContourSet::from_input_with_profile_and_topology_reuse(&input, false, None)
            .expect("cold terminal rails");
    let mut remapped_input = input.clone();
    remapped_input.node_id = 99;
    remapped_input.mouths[0].edge_idx = 70;

    let (reused, _, reuse_status, _) =
        NodeRailContourSet::from_input_with_profile_and_topology_reuse(
            &remapped_input,
            false,
            Some(&topology),
        )
        .expect("edge-id-remapped terminal rails");
    let (cold, _, _, _) = NodeRailContourSet::from_input_with_profile_and_topology_reuse(
        &remapped_input,
        false,
        None,
    )
    .expect("independent remapped terminal rails");

    assert!(reuse_status.rail_topology_reused);
    assert!(reuse_status.ownership_reuse_safe);
    assert!(reuse_status.arrangement_reuse_safe);
    assert_eq!(reused.contours.len(), cold.contours.len());
    assert_eq!(reused.constraints, cold.constraints);
}

#[test]
fn bend_topology_reuse_uses_node_local_mouth_identity() {
    let input = side_join_input(RoadSurfaceVisualNodePieceKind::Bend);
    let (_, _, _, topology) =
        NodeRailContourSet::from_input_with_profile_and_topology_reuse(&input, false, None)
            .expect("cold bend rails");
    let mut remapped_input = input.clone();
    remapped_input.node_id = 99;
    remapped_input.mouths[0].edge_idx = 70;
    remapped_input.mouths[1].edge_idx = 80;

    let (reused, _, reuse_status, _) =
        NodeRailContourSet::from_input_with_profile_and_topology_reuse(
            &remapped_input,
            false,
            Some(&topology),
        )
        .expect("edge-id-remapped bend rails");
    let (cold, _, _, _) = NodeRailContourSet::from_input_with_profile_and_topology_reuse(
        &remapped_input,
        false,
        None,
    )
    .expect("independent remapped bend rails");

    assert!(reuse_status.rail_topology_reused);
    assert!(reuse_status.ownership_reuse_safe);
    assert!(reuse_status.arrangement_reuse_safe);
    assert_eq!(reused.contours.len(), cold.contours.len());
    assert_eq!(reused.constraints, cold.constraints);
    assert_eq!(reused.side_join_gaps, cold.side_join_gaps);
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
    assert_eq!(
        raw_full_roadbed_corridors.len(),
        input.mouths.len(),
        "nonterminal roadbed arms still define footprint, but asphalt must use material ownership"
    );
    let raw_carriageway_corridors = contours
        .contours
        .iter()
        .filter(|contour| {
            contour.purpose == NodeGeneratedContourPurpose::CarriagewayCorridor
                && contour.source_band_index.is_none()
        })
        .collect::<Vec<_>>();
    assert!(
        raw_carriageway_corridors.is_empty(),
        "nonterminal asphalt must not be reintroduced through straight raw corridors"
    );
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
                    && contour.contributes_to_asphalt()
                    && !contour.contributes_to_footprint()
            })
            .count(),
        expected_carriageway_owner_carriers
    );

    assert!(contours.contours.iter().any(|contour| {
        contour.kind == NodeGeneratedContourKind::FullRoadbed
            && contour.purpose == NodeGeneratedContourPurpose::JunctionSideJoin
            && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
    }));
    assert!(contours.contours.iter().any(|contour| {
        contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Sidewalk,
            }
            && contour.purpose == NodeGeneratedContourPurpose::JunctionSideJoin
            && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
            && contour.source_band_index == Some(5)
            && !contour.contributes_to_footprint()
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

    assert_rounded_carriageway_side_join_contour(
        &contours,
        NodeGeneratedContourPurpose::BendSideJoin,
    );
    assert!(contours.contours.iter().any(|contour| {
        contour.kind == NodeGeneratedContourKind::FullRoadbed
            && contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
            && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
    }));
    assert!(contours.contours.iter().any(|contour| {
        contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            }
            && contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
            && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
            && contour.source_band_index == Some(4)
            && !contour.contributes_to_footprint()
    }));
}

#[test]
fn junction_side_join_bands_round_inner_carriageway_corners() {
    let contours = NodeRailContourSet::from_input(&nonterminal_input_with_side_join_candidate())
        .expect("valid contours");

    assert_rounded_carriageway_side_join_contour(
        &contours,
        NodeGeneratedContourPurpose::JunctionSideJoin,
    );
}

#[test]
fn bend_side_join_contours_do_not_route_through_shared_endpoint_center() {
    let contours = NodeRailContourSet::from_input(&side_join_input_with_shared_endpoint_center(
        RoadSurfaceVisualNodePieceKind::Bend,
    ))
    .expect("valid contours");

    assert_side_join_contours_avoid_graph_center(
        &contours,
        NodeGeneratedContourPurpose::BendSideJoin,
    );
}

#[test]
fn junction_side_join_contours_do_not_route_through_shared_endpoint_center() {
    let contours = NodeRailContourSet::from_input(&side_join_input_with_shared_endpoint_center(
        RoadSurfaceVisualNodePieceKind::JunctionN,
    ))
    .expect("valid contours");

    assert_side_join_contours_avoid_graph_center(
        &contours,
        NodeGeneratedContourPurpose::JunctionSideJoin,
    );
}

#[test]
fn bend_boolean_footprint_exposes_rounded_outer_corner_not_miter() {
    let rails = NodeRailContourSet::from_input(&side_join_input_with_shared_endpoint_center(
        RoadSurfaceVisualNodePieceKind::Bend,
    ))
    .expect("valid Bend rails");
    let ownership = RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails)
        .expect("Bend ownership should compile");

    assert!(
        overlay_shapes_boundary_contains_point(
            &ownership.footprint_shapes,
            RoadVec2::new(4.43934, 4.43934),
        ),
        "Bend final footprint must expose the rounded sidewalk-to-terrain corner: {:?}",
        ownership.footprint_shapes
    );
    assert!(
        !overlay_shapes_boundary_contains_point(
            &ownership.footprint_shapes,
            RoadVec2::new(4.0, 4.0)
        ),
        "Bend final footprint must trim the old outer miter point: {:?}",
        ownership.footprint_shapes
    );
}

#[test]
fn bend_boolean_asphalt_does_not_claim_sidewalk_side_join() {
    let rails = NodeRailContourSet::from_input(&side_join_input_with_shared_endpoint_center(
        RoadSurfaceVisualNodePieceKind::Bend,
    ))
    .expect("valid Bend rails");
    let sidewalk_side_join_contours = rails
        .contours
        .iter()
        .filter(|contour| {
            contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
                && contour.contributes_to_non_road_band()
                && matches!(
                    contour.kind,
                    NodeGeneratedContourKind::Band {
                        kind: RoadSurfaceBandKind::Sidewalk,
                    }
                )
        })
        .map(overlay_contour_from_generated_contour)
        .collect::<Vec<_>>();
    assert!(
        !sidewalk_side_join_contours.is_empty(),
        "Bend fixture must emit sidewalk side-join ownership contours"
    );
    let sidewalk_side_join_shapes =
        RoadSurfaceSystem::overlay_union_contours(&sidewalk_side_join_contours)
            .expect("sidewalk side-join contours must union");
    let ownership = RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails)
        .expect("Bend ownership should compile");
    let overlap = RoadSurfaceSystem::overlay_binary_shapes(
        &ownership.asphalt_shapes,
        &sidewalk_side_join_shapes,
        OverlayRule::Intersect,
    )
    .expect("asphalt / sidewalk side-join intersection should solve");
    let overlap_area_m2: f32 = overlap
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum();

    assert!(
        overlap_area_m2 <= 0.001,
        "Bend asphalt must not claim sidewalk side-join authority: overlap_area_m2={overlap_area_m2:.6}, asphalt={:?}, sidewalk_side_join={:?}",
        ownership.asphalt_shapes,
        sidewalk_side_join_shapes
    );
}

#[test]
fn bend_sidewalk_side_join_edges_emit_profile_handoff_constraints() {
    let rails = NodeRailContourSet::from_input(&side_join_input_with_shared_endpoint_center(
        RoadSurfaceVisualNodePieceKind::Bend,
    ))
    .expect("valid Bend rails");
    let sidewalk_side_join_contours = rails
        .contours
        .iter()
        .filter(|contour| {
            contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
                && matches!(
                    contour.kind,
                    NodeGeneratedContourKind::Band {
                        kind: RoadSurfaceBandKind::Sidewalk,
                    }
                )
        })
        .collect::<Vec<_>>();

    assert!(
        !sidewalk_side_join_contours.is_empty(),
        "Bend fixture must emit sidewalk side-join ownership contours"
    );
    for contour in sidewalk_side_join_contours {
        let source_band_index = contour
            .source_band_index
            .expect("sidewalk side join contour must name its source band");
        let owner = contour
            .owner
            .expect("sidewalk side join contour must name its owner");
        let side_join_handoffs = rails
            .constraints
            .iter()
            .filter(|constraint| {
                constraint.kind
                    == (NodeRailConstraintKind::SpanHandoff {
                        kind: RoadSurfaceBandKind::Sidewalk,
                    })
                    && constraint.source_mouth_order_index == contour.source_mouth_order_index
                    && constraint.source_band_index == Some(source_band_index)
                    && constraint.owner == Some(owner)
                    && constraint.opposite_owner.is_none()
                    && constraint.points_xz.len() == 2
            })
            .collect::<Vec<_>>();

        assert!(
            side_join_handoffs.len() >= 3,
            "Bend sidewalk side join must expose both generated side edges in addition to the source profile handoff: contour={contour:?}, handoffs={side_join_handoffs:?}"
        );
    }
}

#[test]
fn junction_boolean_asphalt_exposes_rounded_curb_corner_not_miter() {
    let rails = NodeRailContourSet::from_input(&side_join_input_with_shared_endpoint_center(
        RoadSurfaceVisualNodePieceKind::JunctionN,
    ))
    .expect("valid JunctionN rails");
    let ownership = RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails)
        .expect("JunctionN ownership should compile");

    assert!(
        overlay_shapes_boundary_contains_point(
            &ownership.asphalt_shapes,
            RoadVec2::new(1.65901, 1.65901),
        ),
        "JunctionN asphalt must expose the rounded asphalt-to-curb corner: {:?}",
        ownership.asphalt_shapes
    );
    assert!(
        !overlay_shapes_boundary_contains_point(&ownership.asphalt_shapes, RoadVec2::new(1.0, 1.0)),
        "JunctionN asphalt must trim the old asphalt miter point: {:?}",
        ownership.asphalt_shapes
    );
}

fn overlay_contour_from_generated_contour(contour: &NodeGeneratedContour) -> NodeOverlayContour {
    contour
        .points_xz
        .iter()
        .map(|point| [point.x, point.y])
        .collect()
}

fn assert_rounded_carriageway_side_join_contour(
    contours: &NodeRailContourSet,
    purpose: NodeGeneratedContourPurpose,
) {
    assert!(
        contours.contours.iter().any(|contour| {
            contour.kind
                == (NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::Carriageway,
                })
                && contour.purpose == purpose
                && contour.contributes_to_asphalt()
                && contour.claims_asphalt_owner_region()
                && contour.points_xz.len() > 4
        }),
        "{purpose:?} must emit a rounded carriageway side-join contour for the visible inner asphalt corner"
    );
}

fn assert_side_join_contours_avoid_graph_center(
    contours: &NodeRailContourSet,
    purpose: NodeGeneratedContourPurpose,
) {
    let center_key = road_point_key(RoadVec2::new(0.0, 0.0));
    let side_join_contours = contours
        .contours
        .iter()
        .filter(|contour| contour.purpose == purpose)
        .collect::<Vec<_>>();
    assert!(
        !side_join_contours.is_empty(),
        "{purpose:?} must emit side-join contours"
    );
    for contour in side_join_contours {
        assert!(
            !side_join_contour_is_visible_corner_band(contour)
                || contour
                    .points_xz
                    .iter()
                    .all(|point| road_point_key(*point) != center_key),
            "{purpose:?} visible curb/sidewalk side-join contour must be rounded, not routed through the shared graph endpoint: {contour:?}"
        );
    }
}

fn side_join_contour_is_visible_corner_band(contour: &NodeGeneratedContour) -> bool {
    matches!(
        contour.kind,
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk,
        }
    )
}

fn overlay_shapes_boundary_contains_point(shapes: &NodeOverlayShapes, point: RoadVec2) -> bool {
    shapes
        .iter()
        .flat_map(|shape| shape.iter())
        .flat_map(|contour| contour.iter())
        .any(|candidate| {
            let dx = candidate[0] - point.x;
            let dz = candidate[1] - point.y;
            (dx * dx + dz * dz).sqrt() <= 0.02
        })
}

#[test]
fn nonterminal_same_owner_caps_emit_canonical_side_join_fill() {
    let input = input_with_endpoint_x(0.0);
    let side_join_bands = vec![same_owner_side_join_band()];
    let owners_by_mouth = owners_by_mouth(&input, &[], std::slice::from_ref(&side_join_bands));
    let mut contours = Vec::new();
    let mut corner_trims = Vec::new();
    let mut constraints = Vec::new();
    push_side_join_band_contours(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &input.mouths[0],
        &side_join_bands,
        &owners_by_mouth[0],
        &owners_by_mouth[0].side_join_band_owners,
        &mut contours,
        &mut corner_trims,
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
