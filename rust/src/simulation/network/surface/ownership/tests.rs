//! Tests for node boolean ownership.

use super::rail_authority::{
    NodeRailCanonicalPointSet, canonical_points_by_mm_key_by_owner, constraint_authority_owners,
    insert_open_source_segments, validate_owned_region_vertices_against_source_authority,
};
use super::rings::{
    canonicalize_final_owned_region_boundary_edges,
    canonicalize_owned_region_rings_with_rail_point_set,
};
use super::seams::{canonicalize_seam_constraints, owned_shape_is_discardable_numeric_dust};
use super::topology_keys::{
    NodeOwnershipPointKey, OwnedRegionEdgeKey, overlay_point_from_key,
    ownership_key_from_overlay_point, ownership_key_from_road_point,
};
use super::*;
use crate::simulation::network::surface::backend::{RoadVec2, road_points_to_polyline};
use crate::simulation::network::surface::input::NodeArrangementInput;
use crate::simulation::network::surface::rails::{
    NodeGeneratedContour, NodeGeneratedContourKind, NodeGeneratedContourPurpose,
    NodeRailConstraint, NodeRailConstraintKind, NodeRailContourSet,
};
use crate::simulation::network::surface::validation::NodeValidationReport;
use crate::simulation::network::surface::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, NodeOverlayContour,
    OrderedIncidentPieceMouth,
};
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

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

fn contour_set() -> NodeRailContourSet {
    let mouth = OrderedIncidentPieceMouth {
        profile: profile(10.0),
        endpoint_profile: profile(0.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_sampled_band_domain_paths: false,
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
    NodeRailContourSet::from_input(&input).expect("test input should produce contours")
}

fn test_rail_canonical_points_from_constraints(
    rail_constraints: &[NodeRailConstraint],
) -> NodeRailCanonicalPointSet {
    let mut all_points = rail_constraints
        .iter()
        .flat_map(|constraint| constraint.points_xz.iter().copied())
        .map(ownership_key_from_road_point)
        .collect::<Vec<_>>();
    all_points.sort_unstable();
    all_points.dedup();

    let mut points_by_owner = BTreeMap::<NodeBandOwner, Vec<NodeOwnershipPointKey>>::new();
    let mut segments_by_owner = BTreeMap::<NodeBandOwner, Vec<OwnedRegionEdgeKey>>::new();
    for constraint in rail_constraints {
        let path = constraint
            .points_xz
            .iter()
            .copied()
            .map(ownership_key_from_road_point)
            .collect::<Vec<_>>();
        for owner in constraint_authority_owners(constraint) {
            points_by_owner
                .entry(owner)
                .or_default()
                .extend(path.iter().copied());
            insert_open_source_segments(&mut segments_by_owner, owner, &path);
        }
    }
    for points in points_by_owner.values_mut() {
        points.sort_unstable();
        points.dedup();
    }
    for segments in segments_by_owner.values_mut() {
        segments.sort_unstable();
        segments.dedup();
    }
    let canonical_points_by_mm_key_by_owner = canonical_points_by_mm_key_by_owner(&points_by_owner);
    NodeRailCanonicalPointSet {
        all_points,
        points_by_owner,
        segments_by_owner,
        canonical_points_by_mm_key_by_owner,
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    }
}

#[test]
fn boolean_ownership_produces_asphalt_and_band_owned_regions() {
    let ownership =
        NodeBooleanOwnership::from_rails(&contour_set()).expect("valid ownership solve");

    assert_eq!(ownership.node_id, 42);
    assert_eq!(
        ownership.piece_kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    assert_eq!(ownership.footprint_shapes.len(), 1);
    assert_eq!(ownership.asphalt_shapes.len(), 1);
    assert_eq!(ownership.non_road_shapes.len(), 2);
    assert_eq!(ownership.owned_regions.len(), 4);
    assert_eq!(ownership.owned_region_arrangement.region_count(), 4);
    assert!(ownership.owned_region_arrangement.diagnostics().is_empty());
    assert!(!ownership.owned_region_arrangement.edges().is_empty());
    assert!(
        ownership
            .owned_regions
            .iter()
            .any(|region| region.kind == RoadSurfaceBandKind::Carriageway
                && region.owner == NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2)
                && !region.seam_constraints.is_empty())
    );
    assert!(
        ownership.owned_regions.iter().any(|region| {
            region.seam_constraints.iter().any(|constraint| {
                matches!(
                    constraint.seam_source,
                    NodeSeamSource::RaisedStepContact { .. }
                        | NodeSeamSource::FootprintBoundary { .. }
                )
            })
        }),
        "owned regions must preserve source rail seam constraints"
    );
    assert_eq!(
        ownership
            .owned_regions
            .iter()
            .filter(|region| region.kind == RoadSurfaceBandKind::Sidewalk)
            .count(),
        2
    );
}

#[test]
fn boolean_ownership_rejects_unowned_non_road_residual() {
    let mut rails = contour_set();
    rails.contours.retain(|contour| {
        contour.kind == NodeGeneratedContourKind::FullRoadbed
            || contour.kind
                == NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::Carriageway,
                }
    });

    let error = NodeBooleanOwnership::from_rails(&rails)
        .expect_err("non-road footprint without band contours must be rejected");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::UnownedNonRoadResidual { .. }
    ));
}

#[test]
fn non_road_owner_regions_require_explicit_profile_seam_rails() {
    let mut rails = contour_set();
    rails.constraints.retain(|constraint| {
        matches!(
            constraint.kind,
            NodeRailConstraintKind::FullRoadbedContour | NodeRailConstraintKind::BandContour { .. }
        )
    });

    let error = NodeBooleanOwnership::from_rails(&rails)
        .expect_err("non-road owner carriers without profile seam rails must be rejected");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::UnownedBandResidual {
            kind: RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk,
            ..
        }
    ));
    let report =
        NodeValidationReport::from_boolean_ownership_error(rails.node_id, rails.piece_kind, &error);
    let dump = report.debug_dump();
    assert!(dump.contains("\"stage\":\"boolean_ownership\""));
    assert!(dump.contains("\"kind\":\"rejected_residual\""));
}

#[test]
fn side_join_non_road_authority_claims_before_mouth_band_carriers() {
    let mut rails = contour_set();
    let mut side_join = rails
        .contours
        .iter()
        .find(|contour| {
            contour.kind
                == (NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                })
                && contour.purpose == NodeGeneratedContourPurpose::NonRoadBand
        })
        .cloned()
        .expect("test rail set should include a curb/shoulder mouth carrier");
    side_join.purpose = NodeGeneratedContourPurpose::JunctionSideJoin;
    side_join.claim_priority = NodeGeneratedContourClaimPriority::SideJoin;
    let owner = side_join.owner.expect("band contour has an owner");
    let source_mouth_order_index = side_join.source_mouth_order_index;
    let source_band_index = side_join.source_band_index;
    rails.contours.push(side_join);

    let ownership =
        NodeBooleanOwnership::from_rails(&rails).expect("side-join ownership remains valid");

    assert!(
        ownership.owned_regions.iter().any(|region| {
            region.kind == RoadSurfaceBandKind::CurbOrShoulder
                && region.owner == owner
                && region.source_mouth_order_index == source_mouth_order_index
                && region.source_band_index == source_band_index
                && region.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
        }),
        "overlapping side-join contours must own their final non-road region before ordinary mouth carriers"
    );
    assert!(
        !ownership.owned_regions.iter().any(|region| {
            region.kind == RoadSurfaceBandKind::CurbOrShoulder
                && region.owner == owner
                && region.source_mouth_order_index == source_mouth_order_index
                && region.source_band_index == source_band_index
                && region.claim_priority == NodeGeneratedContourClaimPriority::MouthBand
        }),
        "the same final region must not remain mouth-band-owned after side-join authority claims it"
    );
}

#[test]
fn contour_purpose_gates_junction_footprint_and_asphalt_authority() {
    let mut rails = contour_set();
    let baseline =
        NodeBooleanOwnership::from_rails(&rails).expect("baseline ownership solve is valid");
    let ignored_footprint_points = vec![
        RoadVec2::new(100.0, 100.0),
        RoadVec2::new(102.0, 100.0),
        RoadVec2::new(102.0, 102.0),
        RoadVec2::new(100.0, 102.0),
    ];
    rails.contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::FullRoadbed,
        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
        source_mouth_order_index: 0,
        source_band_index: None,
        owner: None,
        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        points_xz: ignored_footprint_points.clone(),
        height_points_world: None,
        backend_polyline: road_points_to_polyline(ignored_footprint_points, true),
    });

    let outside_asphalt_points = vec![
        RoadVec2::new(110.0, 100.0),
        RoadVec2::new(112.0, 100.0),
        RoadVec2::new(112.0, 102.0),
        RoadVec2::new(110.0, 102.0),
    ];
    rails.contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        purpose: NodeGeneratedContourPurpose::CarriagewayCorridor,
        source_mouth_order_index: 99,
        source_band_index: Some(99),
        owner: Some(NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 99)),
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        points_xz: outside_asphalt_points.clone(),
        height_points_world: None,
        backend_polyline: road_points_to_polyline(outside_asphalt_points, true),
    });
    let owner_carrier_only_points = vec![
        RoadVec2::new(1.0, -3.5),
        RoadVec2::new(3.0, -3.5),
        RoadVec2::new(3.0, -2.5),
        RoadVec2::new(1.0, -2.5),
    ];
    rails.contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        purpose: NodeGeneratedContourPurpose::CarriagewayOwnerCarrier,
        source_mouth_order_index: 98,
        source_band_index: Some(98),
        owner: Some(NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 98)),
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        points_xz: owner_carrier_only_points.clone(),
        height_points_world: None,
        backend_polyline: road_points_to_polyline(owner_carrier_only_points, true),
    });

    let ownership =
        NodeBooleanOwnership::from_rails(&rails).expect("extra gated contours remain valid");
    assert_eq!(ownership.footprint_shapes, baseline.footprint_shapes);
    assert_eq!(ownership.asphalt_shapes, baseline.asphalt_shapes);
    let asphalt_outside = overlay_difference(
        &ownership.asphalt_shapes,
        &ownership.footprint_shapes,
        "test_asphalt_outside_footprint",
    )
    .expect("test overlay difference succeeds");
    assert!(
        asphalt_outside.is_empty(),
        "asphalt authority must be clipped to node_footprint"
    );
}

#[test]
fn protected_span_handoff_dust_stays_owned() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3);
    let shape = vec![vec![[0.0, 0.0], [0.0001, 0.0], [0.0, 0.0001]]];
    let constraints = vec![NodeRailConstraint {
        constraint_index: 7,
        kind: NodeRailConstraintKind::SpanHandoff {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: None,
        owner: Some(owner),
        opposite_owner: None,
        points_xz: vec![RoadVec2::new(0.0, 0.0), RoadVec2::new(0.0001, 0.0)],
    }];

    assert!(
        !owned_shape_is_discardable_numeric_dust(
            &shape,
            RoadSurfaceSystem::overlay_shape_area_m2(&shape),
            owner,
            &constraints,
        ),
        "span-handoff dust must remain an owned top region so mouth/skirt seams cannot point at missing top mesh"
    );
}

#[test]
fn unprotected_numeric_dust_can_still_be_discarded() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3);
    let shape = vec![vec![[0.0, 0.0], [0.0001, 0.0], [0.0, 0.0001]]];

    assert!(owned_shape_is_discardable_numeric_dust(
        &shape,
        RoadSurfaceSystem::overlay_shape_area_m2(&shape),
        owner,
        &[],
    ));
}

#[test]
fn owned_region_rings_are_noded_before_explicit_seam_validation() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![[1.0, 0.0], [3.0, 0.0], [3.0, -1.0], [1.0, -1.0]],
        ),
    ];
    let footprint_shapes = Vec::new();

    canonicalize_owned_region_rings(&mut regions, &footprint_shapes);
    for region in &mut regions {
        region.seam_constraints.push(NodeRegionSeamConstraint {
            constraint_index: 0,
            seam_source: NodeSeamSource::AsphaltBoundary {
                owner_index: region.owner.owner_index(),
            },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(3.0, 0.0),
        });
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }

    let carriageway_contour = &regions[0].shape[0];
    assert!(
        carriageway_contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_overlay_point([1.0, 0.0]))
    );
    assert!(
        carriageway_contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_overlay_point([3.0, 0.0]))
    );
    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                let start = ownership_key_from_road_point(constraint.start_xz);
                let end = ownership_key_from_road_point(constraint.end_xz);
                start == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0))
                    && end == ownership_key_from_road_point(RoadVec2::new(3.0, 0.0))
                    && constraint.is_material_transition
                    && !constraint.constrains_shared_height
            }),
            "region {:?} must own the exact shared sub-edge seam before height/CDT without inventing height authority",
            region.owner
        );
    }
    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &regions,
        &footprint_shapes,
        &[],
    );
    assert!(arrangement.diagnostics().is_empty());
    assert!(arrangement.edges().iter().any(|edge| {
        edge.owner == carriageway
            && edge.opposite_owner == Some(sidewalk)
            && edge.start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(1.0, 0.0))
            && edge.end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(3.0, 0.0))
            && !edge.source_constraint_indices.is_empty()
    }));
}

#[test]
fn materializes_seam_constraints_for_final_noded_owned_edges() {
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![[1.0, 0.0], [2.0, 0.0], [2.0, 2.0], [1.0, 2.0]],
        ),
    ];
    let footprint_shapes = vec![vec![vec![
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 2.0],
        [1.0, 1.0],
        [0.0, 2.0],
    ]]];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 33,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: Some(sidewalk),
        points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(1.0, 2.0)],
    }];

    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
    canonicalize_final_owned_region_boundary_edges(
        &mut regions,
        &footprint_shapes,
        &rail_canonical_points,
    );
    materialize_noded_region_seam_constraints(
        &mut regions,
        &footprint_shapes,
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0))
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(RoadVec2::new(1.0, 1.0))
                    && constraint.owner == Some(curb)
                    && constraint.opposite_owner == Some(sidewalk)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "first final subedge must carry the original raised-step seam"
        );
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(RoadVec2::new(1.0, 1.0))
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(RoadVec2::new(1.0, 2.0))
                    && constraint.owner == Some(curb)
                    && constraint.opposite_owner == Some(sidewalk)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "second final subedge must carry the original raised-step seam"
        );
    }
    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::Terminal,
        &regions,
        &footprint_shapes,
        &rail_constraints,
    );

    assert!(arrangement.diagnostics().is_empty());
}

#[test]
fn source_local_owned_boundary_preserves_explicit_height_endpoint_authority() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let local_endpoint = (1_000_001, 0);
    let canonical_endpoint = (1_000_000, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            [0.0, 0.0],
            overlay_point_from_key(local_endpoint),
            [0.0, 1.0],
        ],
    )];
    let mut height_points_by_source = BTreeMap::new();
    height_points_by_source.insert(
        (
            RoadSurfaceBandKind::Carriageway,
            carriageway.owner_index(),
            carriageway.owner_index(),
        ),
        vec![local_endpoint],
    );
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: vec![canonical_endpoint],
        points_by_owner: BTreeMap::from([(carriageway, vec![canonical_endpoint])]),
        segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, vec![canonical_endpoint])],
        )),
        height_points_by_source,
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points);

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&canonical_endpoint));
    assert!(contour_keys.contains(&local_endpoint));
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
}

#[test]
fn noncanonical_owned_region_vertex_reports_source_authority_error() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let drifted_endpoint = [1.000004, 0.0];
    let regions = vec![test_owned_region(
        RoadSurfaceBandKind::CurbOrShoulder,
        curb,
        vec![drifted_endpoint, [2.0, 0.0], [2.0, 2.0]],
    )];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 33,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: None,
        points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(1.0, 2.0)],
    }];
    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);

    assert!(matches!(
        validate_owned_region_vertices_against_source_authority(
            &regions,
            &rail_canonical_points
        ),
        Err(NodeBooleanOwnershipError::NonCanonicalOwnedRegionVertex {
            owner,
            point_x_key,
            point_z_key,
            canonical_x_key,
            canonical_z_key,
        }) if owner == curb
            && point_x_key == ownership_key_from_overlay_point(drifted_endpoint).0
            && point_z_key == ownership_key_from_overlay_point(drifted_endpoint).1
            && canonical_x_key == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0)).0
            && canonical_z_key == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0)).1
    ));
}

#[test]
fn materializes_owner_explicit_step_for_final_edge_on_exact_constraint_span() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 1.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 1.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [3.0, -1.0], [2.0, 1.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 34,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![start, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::Bend,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && ((constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(curb))
                        || (constraint.owner == Some(curb)
                            && constraint.opposite_owner == Some(carriageway)))
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "final shared asphalt-curb edge must carry the owner-explicit step seam"
        );
    }
}

#[test]
fn materializes_asymmetric_asphalt_curb_boundary_from_final_noded_edges() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let first_split = RoadVec2::new(1.0, 0.0);
    let second_split = RoadVec2::new(2.0, 0.0);
    let end = RoadVec2::new(3.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [3.0, 0.0], [3.0, -1.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![
                [0.0, 0.0],
                [1.0, 0.0],
                [2.0, 0.0],
                [3.0, 0.0],
                [3.0, 1.0],
                [0.0, 1.0],
            ],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 37,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![start, first_split, second_split, end],
    }];
    let footprint_shapes = Vec::new();

    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
    canonicalize_final_owned_region_boundary_edges(
        &mut regions,
        &footprint_shapes,
        &rail_canonical_points,
    );
    materialize_noded_region_seam_constraints(
        &mut regions,
        &footprint_shapes,
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::Bend,
    );

    let carriageway_contour = &regions[0].shape[0];
    assert!(
        carriageway_contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_road_point(first_split))
    );
    assert!(
        carriageway_contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_road_point(second_split))
    );
    for (subedge_start, subedge_end) in [
        (start, first_split),
        (first_split, second_split),
        (second_split, end),
    ] {
        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    ownership_key_from_road_point(constraint.start_xz)
                        == ownership_key_from_road_point(subedge_start)
                        && ownership_key_from_road_point(constraint.end_xz)
                            == ownership_key_from_road_point(subedge_end)
                        && constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(curb)
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "final owned asphalt-curb subedge must carry the exact explicit step seam"
            );
        }
    }

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::Bend,
        &regions,
        &footprint_shapes,
        &rail_constraints,
    );
    assert!(arrangement.diagnostics().is_empty());
    assert!(!arrangement.edges().iter().any(|edge| {
        edge.owner == carriageway
            && edge.opposite_owner == Some(curb)
            && edge.start == NodeOwnedRegionArrangementKey::from_point(start)
            && edge.end == NodeOwnedRegionArrangementKey::from_point(end)
    }));
}

#[test]
fn junctionn_materializes_final_step_edge_from_exact_owner_pair_polyline_authority() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(3.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [3.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [0.0, 1.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 41,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![
            start,
            RoadVec2::new(1.0, 0.000001),
            RoadVec2::new(2.0, -0.000001),
            end,
        ],
    }];
    let footprint_shapes = Vec::new();

    materialize_noded_region_seam_constraints(
        &mut regions,
        &footprint_shapes,
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && constraint.owner == Some(carriageway)
                    && constraint.opposite_owner == Some(curb)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "JunctionN final asphalt-curb edge must materialize from exact source-pair polyline authority"
        );
    }

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &regions,
        &footprint_shapes,
        &rail_constraints,
    );
    assert!(arrangement.diagnostics().is_empty());
}

#[test]
fn junctionn_reports_unmaterialized_raised_step_authority_before_height_validation() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(3.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [3.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [0.0, 1.0]],
        ),
    ];
    for region in &mut regions {
        region.seam_constraints.push(NodeRegionSeamConstraint {
            constraint_index: 7,
            seam_source: NodeSeamSource::AsphaltBoundary {
                owner_index: region.owner.owner_index(),
            },
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            constrains_shared_height: true,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        });
    }
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 41,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![
            start,
            RoadVec2::new(1.0, 0.000001),
            RoadVec2::new(2.0, -0.000001),
            end,
        ],
    }];

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &regions,
        &Vec::new(),
        &rail_constraints,
    );

    assert!(arrangement.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic,
        NodeOwnedRegionArrangementDiagnostic::UnmaterializedRaisedStepAuthority {
            region_index: 0,
            owner,
            opposite_owner,
            start,
            end,
            source_constraint_indices,
        } if *owner == carriageway
            && *opposite_owner == curb
            && *start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(0.0, 0.0))
            && *end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(3.0, 0.0))
            && source_constraint_indices.as_slice() == [41]
    )));
    let report = NodeValidationReport::from_owned_region_arrangement_diagnostics(&arrangement)
        .expect("unmaterialized authority must block before height validation");
    let dump = report.debug_dump();
    assert!(dump.contains("\"kind\":\"unmaterialized_raised_step_authority\""));
    assert!(dump.contains("\"backend\":\"canonical_keys\""));
    assert!(dump.contains("source_constraint_indices: [41]"));
}

#[test]
fn materializes_role_only_raised_step_contact_as_exact_owned_edge_pair() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 1.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 1.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [3.0, -1.0], [2.0, 1.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 35,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: None,
        points_xz: vec![start, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::Bend,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && ((constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(curb))
                        || (constraint.owner == Some(curb)
                            && constraint.opposite_owner == Some(carriageway)))
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "role-only asphalt-curb contact must instantiate the actual owned edge pair"
        );
    }
}

#[test]
fn materializes_same_kind_reowned_raised_step_contact_as_exact_owned_edge_pair() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let source_curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let final_curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            final_curb,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, 1.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 35,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(source_curb),
        points_xz: vec![start, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && ((constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(final_curb))
                        || (constraint.owner == Some(final_curb)
                            && constraint.opposite_owner == Some(carriageway)))
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "final owned edge must instantiate its exact owner pair from a same-kind source rail"
        );
    }
}

#[test]
fn reowned_raised_step_contact_does_not_inherit_source_pair_shared_height_contract() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let source_sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let final_sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 2);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(0.0, 3.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [0.0, 3.0], [-1.0, 0.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            final_sidewalk,
            vec![[0.0, 0.0], [1.0, 0.0], [0.0, 3.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 35,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: Some(source_sidewalk),
        points_xz: vec![start, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && ((constraint.owner == Some(curb)
                        && constraint.opposite_owner == Some(final_sidewalk))
                        || (constraint.owner == Some(final_sidewalk)
                            && constraint.opposite_owner == Some(curb)))
                    && !constraint.constrains_shared_height
                    && constraint.is_material_transition
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "reowned side-join contact must authorize the exact final edge without forcing source-pair shared height"
        );
    }
}

#[test]
fn materializes_cross_material_contact_from_exact_final_owner_band_contour_edge() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 1.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 1.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [3.0, -1.0], [2.0, 1.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 41,
        kind: NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: None,
        owner: Some(carriageway),
        opposite_owner: None,
        points_xz: vec![start, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && ((constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(curb))
                        || (constraint.owner == Some(curb)
                            && constraint.opposite_owner == Some(carriageway)))
                    && !constraint.constrains_shared_height
                    && constraint.is_material_transition
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "exact final owner band contour edge must authorize the asphalt-curb step"
        );
    }
}

#[test]
fn projected_material_boundary_canonicalizes_source_authorized_endpoint() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let source_start = RoadVec2::new(1.0, 0.0);
    let drifted_start = [1.000004, 0.0];
    let end = RoadVec2::new(1.0, 2.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![drifted_start, [2.0, 0.0], [2.0, 2.0], [1.000004, 2.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 41,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![source_start, end],
    }];
    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points);

    let curb_points = regions[1].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(curb_points.contains(&ownership_key_from_road_point(source_start)));
    assert!(!curb_points.contains(&ownership_key_from_overlay_point(drifted_start)));
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
}

#[test]
fn does_not_materialize_cross_material_contact_from_band_contour_chord() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let middle = RoadVec2::new(1.0, 1.0);
    let end = RoadVec2::new(2.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, 1.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 42,
        kind: NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: None,
        owner: Some(carriageway),
        opposite_owner: None,
        points_xz: vec![start, middle, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::Bend,
    );

    for region in &regions {
        assert!(
            !region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && constraint.owner == Some(carriageway)
                    && constraint.opposite_owner == Some(curb)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "band contours authorize final contacts only on exact source segments"
        );
    }
}

#[test]
fn does_not_materialize_asphalt_curb_step_from_bend_polyline_coverage() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 2.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 35,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![start, RoadVec2::new(1.0, 1.0), end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::Bend,
    );

    for region in &regions {
        assert!(
            !region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && constraint.owner == Some(carriageway)
                    && constraint.opposite_owner == Some(curb)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "asphalt-curb vertical steps must come from an exact rail span, not Bend polyline coverage"
        );
    }
}

#[test]
fn asphalt_curb_shape_seams_use_exact_constraint_spans() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let middle = RoadVec2::new(1.0, 1.0);
    let end = RoadVec2::new(2.0, 2.0);
    let shape = vec![vec![[0.0, 0.0], [2.0, 2.0], [0.0, 2.0]]];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 36,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![start, middle, end],
    }];

    let seams = seam_constraints_for_shape(&shape, carriageway, &rail_constraints, false);

    assert!(
        !seams.iter().any(|constraint| {
            ownership_key_from_road_point(constraint.start_xz)
                == ownership_key_from_road_point(start)
                && ownership_key_from_road_point(constraint.end_xz)
                    == ownership_key_from_road_point(end)
        }),
        "asphalt-curb seams must not carry a full edge just because a rail polyline covers it"
    );
    assert!(
        seams.iter().any(|constraint| {
            ownership_key_from_road_point(constraint.start_xz)
                == ownership_key_from_road_point(start)
                && ownership_key_from_road_point(constraint.end_xz)
                    == ownership_key_from_road_point(middle)
        }),
        "first exact rail span should be preserved"
    );
    assert!(
        seams.iter().any(|constraint| {
            ownership_key_from_road_point(constraint.start_xz)
                == ownership_key_from_road_point(middle)
                && ownership_key_from_road_point(constraint.end_xz)
                    == ownership_key_from_road_point(end)
        }),
        "second exact rail span should be preserved"
    );
}

#[test]
fn canonicalizes_overlay_vertex_drift_to_unique_source_rail_key() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::CurbOrShoulder,
        curb,
        vec![[0.0, 0.0], [1.000004, 0.0], [1.000004, 2.0], [0.0, 2.0]],
    )];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 33,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: None,
        points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(1.0, 2.0)],
    }];

    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points);

    let contour = &regions[0].shape[0];
    assert!(
        contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0)))
    );
    assert!(
        contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_road_point(RoadVec2::new(1.0, 2.0)))
    );
    assert!(
        contour.iter().all(|point| {
            ownership_key_from_overlay_point(*point)
                != ownership_key_from_overlay_point([1.000004, 0.0])
                && ownership_key_from_overlay_point(*point)
                    != ownership_key_from_overlay_point([1.000004, 2.0])
        }),
        "owned region vertices must use the owner-authorized source rail keys, not backend drift"
    );
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
}

#[test]
fn canonicalizes_closing_overlay_dust_to_source_rail_endpoint() {
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 6);
    let endpoint = RoadVec2::new(15.169048, 5.0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Sidewalk,
        sidewalk,
        vec![
            [15.169047, 5.0],
            [15.169048, 3.65],
            [15.979047, 3.65],
            [15.596568, 4.287465],
            [15.169048, 4.999998],
        ],
    )];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 34,
        kind: NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: RoadSurfaceBandKind::Sidewalk,
        },
        source_mouth_order_index: 1,
        source_band_index: Some(0),
        source_boundary_index: Some(0),
        owner: Some(sidewalk),
        opposite_owner: None,
        points_xz: vec![endpoint, RoadVec2::new(15.169048, 3.65)],
    }];

    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points);

    let contour = &regions[0].shape[0];
    let endpoint_key = ownership_key_from_road_point(endpoint);
    assert_eq!(ownership_key_from_overlay_point(contour[0]), endpoint_key);
    assert_eq!(
        contour
            .iter()
            .filter(|point| ownership_key_from_overlay_point(**point) == endpoint_key)
            .count(),
        1,
        "closing overlay dust must collapse onto the authorized source rail endpoint"
    );
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
}

#[test]
fn explicit_shared_point_constraints_preserve_endpoint_context_without_height_continuity() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]],
        ),
    ];

    for region in &mut regions {
        region.seam_constraints.push(NodeRegionSeamConstraint {
            constraint_index: 0,
            seam_source: NodeSeamSource::AsphaltBoundary {
                owner_index: region.owner.owner_index(),
            },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 1.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        });
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                let start = ownership_key_from_road_point(constraint.start_xz);
                let end = ownership_key_from_road_point(constraint.end_xz);
                start == ownership_key_from_road_point(RoadVec2::new(1.0, 1.0))
                    && end == start
                    && constraint.is_material_transition
                    && !constraint.constrains_shared_height
            }),
            "point-only material contacts must remain explicit seam endpoints without asserting one shared height"
        );
    }
}

#[test]
fn owned_region_arrangement_reports_shared_edge_without_seam_constraint() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![[4.0, 0.0], [6.0, 0.0], [6.0, 2.0], [4.0, 2.0]],
        ),
    ];

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        43,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &regions,
        &Vec::new(),
        &[],
    );

    assert!(arrangement.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic,
        NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
            region_index: 0,
            owner,
            opposite_owner,
            start,
            end,
        } if *owner == carriageway
            && *opposite_owner == sidewalk
            && *start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(4.0, 0.0))
            && *end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(4.0, 2.0))
    )));
}

#[test]
fn same_band_owned_region_edge_does_not_require_material_seam_constraint() {
    let first = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let second = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 1);
    let regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            first,
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            second,
            vec![[4.0, 0.0], [6.0, 0.0], [6.0, 2.0], [4.0, 2.0]],
        ),
    ];

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        44,
        RoadSurfaceVisualNodePieceKind::Bend,
        &regions,
        &Vec::new(),
        &[],
    );

    assert!(arrangement.diagnostics().is_empty());
}

fn test_owned_region(
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    contour: NodeOverlayContour,
) -> NodeBooleanOwnedRegion {
    NodeBooleanOwnedRegion {
        kind,
        owner,
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        source_mouth_order_index: owner.owner_index(),
        source_band_index: Some(owner.owner_index()),
        shape: vec![contour],
        area_m2: 1.0,
        seam_constraints: Vec::new(),
    }
}
