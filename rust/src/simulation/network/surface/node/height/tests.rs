//! Tests for canonical node-height field construction and evaluation.

use super::super::arrangement::NodeSeamSource;
use super::build::height_fields_by_source;
use super::grade::apply_junctionn_height_authority_normalization;
use super::model::*;
use super::seams::*;
use super::source_edges::{
    height_edges_from_vertices, height_source_point_key, terminal_edge_height_at,
    terminal_edge_height_at_exact,
};
use super::triangles::height_triangles_from_contour;
use super::*;
use crate::simulation::network::surface::backend::road_points_to_polyline;
use crate::simulation::network::surface::input::NodeInputMouth;
use crate::simulation::network::surface::ownership::{
    NodeBooleanOwnership, NodeOwnedRegionArrangement,
};
use crate::simulation::network::surface::rails::NodeRailContourSet;
use crate::simulation::network::surface::terminal::{
    TerminalCapBandProvenance, TerminalCapBandRole,
};
use crate::simulation::network::surface::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
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

fn profile(x: f32, base_height: f32) -> IncidentMouthProfile {
    let boundary_points_world = vec![
        Vector3::new(x, base_height, -4.0),
        Vector3::new(x, base_height + 0.1, -2.0),
        Vector3::new(x, base_height + 0.2, 0.0),
        Vector3::new(x, base_height + 0.3, 2.0),
        Vector3::new(x, base_height + 0.4, 4.0),
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

fn solved_input() -> NodeArrangementInput {
    let mouth = OrderedIncidentPieceMouth {
        profile: profile(10.0, 4.0),
        endpoint_profile: profile(0.0, 2.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_explicit_band_domain_paths: false,
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

fn solved_ownership(input: &NodeArrangementInput) -> NodeBooleanOwnership {
    let rails = NodeRailContourSet::from_input(input).expect("test input should produce rails");
    NodeBooleanOwnership::from_rails(&rails).expect("test rails should produce ownership")
}

#[test]
fn evaluates_owned_region_vertices_from_band_height_fields() {
    let input = solved_input();
    let ownership = solved_ownership(&input);
    let solution = NodeHeightSolution::from_ownership_and_input(&input, &ownership)
        .expect("valid ownership should height every canonical vertex");

    assert_eq!(solution.node_id, 42);
    assert_eq!(
        solution.piece_kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    assert_eq!(solution.regions.len(), ownership.owned_regions.len());

    let carriageway = solution
        .regions
        .iter()
        .find(|region| region.kind == RoadSurfaceBandKind::Carriageway)
        .expect("test input has a carriageway band");
    assert!(has_vertex_height(carriageway, 0.0, 0.0, 2.2));
    assert!(has_vertex_height(carriageway, 10.0, 2.0, 4.3));
}

#[test]
fn rejects_missing_source_band() {
    let input = conflicting_manual_input();
    let owned_regions = vec![manual_region(RoadSurfaceBandKind::Carriageway, 99, 2.0)];
    let ownership = NodeBooleanOwnership {
        node_id: 77,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        footprint_shapes: Vec::new(),
        asphalt_shapes: Vec::new(),
        non_road_shapes: Vec::new(),
        owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
            77,
            RoadSurfaceVisualNodePieceKind::Bend,
            &owned_regions,
            &Vec::new(),
            &[],
        ),
        owned_regions,
    };

    assert_eq!(
        NodeHeightSolution::from_ownership_and_input(&input, &ownership),
        Err(NodeHeightFieldError::MissingSourceBand {
            mouth_order_index: 0,
            band_index: 99,
        })
    );
}

#[test]
fn source_band_height_carrier_rejects_mismatched_explicit_paths() {
    let mut interval = manual_interval(0, RoadSurfaceBandKind::Sidewalk, 2.0, 4.0);
    interval.start_path_world = vec![
        interval.mouth_start_world,
        RoadVec3::new(5.0, 6.0, 0.0),
        interval.endpoint_start_world,
    ];
    interval.end_path_world = vec![
        interval.mouth_end_world,
        RoadVec3::new(7.5, 4.0, 2.0),
        RoadVec3::new(2.5, 2.0, 2.0),
        interval.endpoint_end_world,
    ];

    let result = NodeBandHeightField::from_interval(0, &interval, None);

    assert!(matches!(
        result,
        Err(NodeHeightFieldError::InvalidSourceBandHeightCarrier {
            reason: "mismatched_source_band_path_lengths",
            ..
        })
    ));
}

#[test]
fn source_band_height_carrier_rejects_one_sided_explicit_path_even_with_support_points() {
    let mut interval = manual_interval(0, RoadSurfaceBandKind::Sidewalk, 2.0, 4.0);
    interval.start_path_world = vec![
        interval.mouth_start_world,
        RoadVec3::new(5.0, 6.0, 0.0),
        interval.endpoint_start_world,
    ];
    interval.end_path_world = vec![interval.mouth_end_world, interval.endpoint_end_world];
    let source_support = vec![
        RoadVec3::new(10.0, 4.0, 2.0),
        RoadVec3::new(5.0, 3.0, 2.0),
        RoadVec3::new(0.0, 2.0, 2.0),
    ];

    let result = NodeBandHeightField::from_interval(0, &interval, Some(&source_support));

    assert!(matches!(
        result,
        Err(NodeHeightFieldError::InvalidSourceBandHeightCarrier {
            reason: "mismatched_source_band_path_lengths",
            ..
        })
    ));
}

#[test]
fn source_band_height_carrier_accepts_materialized_two_sided_explicit_paths() {
    let mut interval = manual_interval(0, RoadSurfaceBandKind::Sidewalk, 2.0, 4.0);
    interval.start_path_world = vec![
        interval.mouth_start_world,
        RoadVec3::new(5.0, 6.0, 0.0),
        interval.endpoint_start_world,
    ];
    interval.end_path_world = vec![
        interval.mouth_end_world,
        RoadVec3::new(5.0, 3.0, 2.0),
        interval.endpoint_end_world,
    ];
    let field = NodeBandHeightField::from_interval(0, &interval, None)
        .expect("paired explicit source rails are a valid carrier");

    let height_m = field
        .evaluate_height(RoadVec2::new(5.0, 1.0))
        .expect("canonical point inside explicit source carrier should evaluate");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(height_m),
        SurfaceHeightMmKey::from_m_f64(4.5)
    );
}

#[test]
fn source_band_height_field_uses_rail_materialized_outer_chord() {
    let mouth = OrderedIncidentPieceMouth {
        profile: profile(10.0, 4.0),
        endpoint_profile: profile(0.0, 2.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: vec![vec![
            Vector3::new(10.0, 4.0, -4.0),
            Vector3::new(5.0, 3.0, -4.0),
            Vector3::new(0.0, 2.0, -4.0),
        ]],
        band_end_paths_world: Vec::new(),
        uses_explicit_band_domain_paths: true,
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
    let rails = NodeRailContourSet::from_input(&input).expect("rails should materialize carriers");
    let fields = height_fields_by_source(&input, Some(&rails))
        .expect("height fields should consume rail-materialized carrier paths");
    let field = fields
        .get(&NodeSourceBandKey {
            mouth_order_index: 0,
            band_index: 0,
        })
        .expect("test input has a first sidewalk field");

    let height_m = field
        .evaluate_height(RoadVec2::new(5.0, -3.0))
        .expect("materialized outer chord should make the band interior evaluable");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(height_m),
        SurfaceHeightMmKey::from_m_f64(3.05)
    );
}

#[test]
fn height_carrier_rejects_duplicate_canonical_xz_with_different_height() {
    let points = [
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.25, 0.0),
        RoadVec3::new(10.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, 2.0),
    ];
    assert!(matches!(
        height_edges_from_vertices(&points),
        Err(HeightCarrierContourError::ConflictingDuplicateHeightVertex)
    ));

    let id = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk);
    assert!(matches!(
        height_triangles_from_contour(
            id,
            RoadSurfaceBandKind::Sidewalk,
            NodeHeightPatchAuthority::source_interval(),
            &points,
        ),
        Err(NodeHeightFieldError::InvalidHeightCarrierContour {
            reason: "conflicting_duplicate_height_vertex",
            ..
        })
    ));
}

#[test]
fn rejects_owned_region_vertex_outside_explicit_height_carrier() {
    let input = conflicting_manual_input();
    let mut region = manual_region(RoadSurfaceBandKind::Carriageway, 0, 2.0);
    region.shape = vec![vec![[0.0, 0.0], [10.0, 0.0], [11.0, 1.0], [0.0, 2.0]]];
    let owned_regions = vec![region];
    let ownership = NodeBooleanOwnership {
        node_id: 77,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        footprint_shapes: Vec::new(),
        asphalt_shapes: Vec::new(),
        non_road_shapes: Vec::new(),
        owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
            77,
            RoadSurfaceVisualNodePieceKind::Bend,
            &owned_regions,
            &Vec::new(),
            &[],
        ),
        owned_regions,
    };

    assert!(matches!(
        NodeHeightSolution::from_ownership_and_input(&input, &ownership),
        Err(NodeHeightFieldError::VertexOutsideHeightField {
            mouth_order_index: 0,
            band_index: 0,
            source_kind: RoadSurfaceBandKind::Carriageway,
            ..
        })
    ));
}

#[test]
fn junctionn_canonical_height_authority_rejects_vertex_outside_explicit_carrier() {
    let mut input = conflicting_manual_input();
    input.piece_kind = RoadSurfaceVisualNodePieceKind::JunctionN;
    let mut region = manual_region(RoadSurfaceBandKind::Carriageway, 0, 2.0);
    region.shape = vec![vec![[0.0, 0.0], [10.0, 0.0], [11.0, 1.0], [0.0, 2.0]]];
    let owned_regions = vec![region];
    let ownership = NodeBooleanOwnership {
        node_id: 77,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        footprint_shapes: Vec::new(),
        asphalt_shapes: Vec::new(),
        non_road_shapes: Vec::new(),
        owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
            77,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &owned_regions,
            &Vec::new(),
            &[],
        ),
        owned_regions,
    };

    assert!(matches!(
        NodeHeightSolution::from_ownership_and_input(&input, &ownership),
        Err(NodeHeightFieldError::VertexOutsideHeightField { .. })
    ));
}

#[test]
fn junctionn_canonical_height_authority_prefers_owner_generated_carrier_over_base_interval() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Carriageway, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let patch = NodeBandHeightPatch::from_heighted_contour(
        field.id,
        field.kind,
        &[
            RoadVec3::new(0.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 2.0),
            RoadVec3::new(0.0, 1.0, 2.0),
        ],
        NodeHeightPatchAuthority {
            owner: Some(owner),
            role: NodeHeightPatchAuthorityRole::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            },
        },
    )
    .expect("test generated contour is a valid height carrier");
    field.patches.push(patch);

    assert!(matches!(
        field.evaluate_height(RoadVec2::new(5.0, 1.0)),
        Err(NodeHeightFieldError::SourceHeightFieldConflict { .. })
    ));
    let height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 1.0),
        )
        .expect("owner-generated carrier is explicit height authority for JunctionN");
    assert!((height.height_m - 1.0).abs() <= 1.0e-6);
    assert_eq!(
        height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );
}

#[test]
fn junctionn_canonical_height_authority_scopes_generated_carriers_to_owned_region_claim() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::CurbOrShoulder, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
    for (height_m, purpose, claim_priority) in [
        (
            1.0,
            NodeGeneratedContourPurpose::NonRoadBand,
            NodeGeneratedContourClaimPriority::MouthBand,
        ),
        (
            2.0,
            NodeGeneratedContourPurpose::JunctionSideJoin,
            NodeGeneratedContourClaimPriority::SideJoin,
        ),
    ] {
        let patch = NodeBandHeightPatch::from_heighted_contour(
            field.id,
            field.kind,
            &[
                RoadVec3::new(0.0, height_m, 0.0),
                RoadVec3::new(10.0, height_m, 0.0),
                RoadVec3::new(10.0, height_m, 2.0),
                RoadVec3::new(0.0, height_m, 2.0),
            ],
            NodeHeightPatchAuthority {
                owner: Some(owner),
                role: NodeHeightPatchAuthorityRole::GeneratedContour {
                    purpose,
                    claim_priority,
                },
            },
        )
        .expect("test generated contour is a valid height carrier");
        field.patches.push(patch);
    }

    let mouth_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::MouthBand,
            RoadVec2::new(5.0, 1.0),
        )
        .expect("mouth-owned region should use mouth-band generated carrier");
    assert_eq!(
        mouth_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        }
    );
    assert!((mouth_height.height_m - 1.0).abs() <= 1.0e-6);

    let side_join_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 1.0),
        )
        .expect("side-join-owned region should use side-join generated carrier");
    assert_eq!(
        side_join_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );
    assert!((side_join_height.height_m - 2.0).abs() <= 1.0e-6);
}

#[test]
fn side_join_height_authority_reuses_source_rail_only_at_canonical_handoff_vertices() {
    let source_support = [RoadVec3::new(5.0, 0.5, 0.0)];
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        Some(&source_support),
    )
    .expect("manual interval is a valid source height carrier");
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let points_xz = vec![
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(5.0, 0.0),
        RoadVec2::new(10.0, 0.0),
        RoadVec2::new(10.0, 2.0),
        RoadVec2::new(0.0, 2.0),
    ];
    let contour = NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        owner: Some(owner),
        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        backend_polyline: road_points_to_polyline(points_xz.clone(), true),
        points_xz,
        height_points_world: Some(vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(5.0, 0.5, 0.0),
            RoadVec3::new(10.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 2.0),
            RoadVec3::new(0.0, 0.0, 2.0),
        ]),
    };
    field
        .extend_with_generated_contour(&contour)
        .expect("test generated contour is a valid height carrier");

    let handoff_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 0.0),
        )
        .expect("side-join vertex on source rail should reuse source rail height");
    assert_eq!(
        handoff_height.authority,
        NodeHeightAuthoritySource::SourceInterval
    );
    assert!((handoff_height.height_m - 0.5).abs() <= 1.0e-6);

    let source_edge_non_handoff_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(2.5, 0.0),
        )
        .expect(
            "source-edge point without explicit topology handoff should evaluate generated carrier",
        );
    assert_eq!(
        source_edge_non_handoff_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );
    assert!(
        (source_edge_non_handoff_height.height_m - 0.25).abs() <= 1.0e-6,
        "exact boundary vertices may evaluate the generated contour, not a drifted source fallback"
    );

    let dedup_drifted_handoff_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 0.00005),
        )
        .expect("drifted side-join vertex should use generated authority, not source-edge handoff");
    assert_eq!(
        dedup_drifted_handoff_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );

    assert!(
        matches!(
            field.evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, -0.00005)
            ),
            Err(NodeHeightFieldError::VertexOutsideHeightField { .. })
        ),
        "canonical drift outside the contour must not be accepted as edge support"
    );

    assert!(
        matches!(
            field.evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, -0.001)
            ),
            Err(NodeHeightFieldError::VertexOutsideHeightField { .. })
        ),
        "near-edge vertices outside the generated contour must not inherit source-rail height"
    );

    let near_generated_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 0.0002),
        )
        .expect("inside generated contour but off exact handoff should evaluate generated carrier");
    assert_eq!(
        near_generated_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );

    let interior_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 1.0),
        )
        .expect("side-join interior should still use generated contour authority");
    assert_eq!(
        interior_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );
}

#[test]
fn contour_edge_height_requires_precomputed_support_key() {
    let edges = vec![NodeBandHeightEdge {
        start_xz: RoadVec2::new(0.0, 0.0),
        end_xz: RoadVec2::new(10.0, 0.0),
        start_height_m: 0.0,
        end_height_m: 1.0,
    }];
    let point_xz = RoadVec2::new(5.0, 0.0);
    let empty_support = BTreeSet::new();
    assert_eq!(
        terminal_edge_height_at(point_xz, &edges, &empty_support)
            .expect("single edge has no height conflict"),
        None
    );

    let mut explicit_support = BTreeSet::new();
    explicit_support.insert(height_source_point_key(point_xz));
    let height_m = terminal_edge_height_at(point_xz, &edges, &explicit_support)
        .expect("single edge has no height conflict")
        .expect("explicit support key should allow exact contour-edge height");
    assert!((height_m - 0.5).abs() <= 1.0e-6);
}

#[test]
fn contour_edge_height_rejects_conflicting_exact_edge_candidates() {
    let edges = vec![
        NodeBandHeightEdge {
            start_xz: RoadVec2::new(0.0, 0.0),
            end_xz: RoadVec2::new(1.0, 0.0),
            start_height_m: 1.0,
            end_height_m: 1.0,
        },
        NodeBandHeightEdge {
            start_xz: RoadVec2::new(0.0, 0.0),
            end_xz: RoadVec2::new(0.0, 1.0),
            start_height_m: 2.0,
            end_height_m: 2.0,
        },
    ];

    let conflict = terminal_edge_height_at_exact(RoadVec2::new(0.0, 0.0), &edges)
        .expect_err("same canonical edge key with different heights must reject");
    assert_eq!(conflict.existing_height_mm, 1000);
    assert_eq!(conflict.incoming_height_mm, 2000);
}

#[test]
fn junctionn_canonical_height_authority_rejects_conflicting_owner_generated_carriers() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Carriageway, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let authority = NodeHeightPatchAuthority {
        owner: Some(owner),
        role: NodeHeightPatchAuthorityRole::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        },
    };
    for height_m in [1.0, 2.0] {
        let patch = NodeBandHeightPatch::from_heighted_contour(
            field.id,
            field.kind,
            &[
                RoadVec3::new(0.0, height_m, 0.0),
                RoadVec3::new(10.0, height_m, 0.0),
                RoadVec3::new(10.0, height_m, 2.0),
                RoadVec3::new(0.0, height_m, 2.0),
            ],
            authority,
        )
        .expect("test generated contour is a valid height carrier");
        field.patches.push(patch);
    }

    assert!(matches!(
        field.evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 1.0)
        ),
        Err(NodeHeightFieldError::SourceHeightFieldConflict { .. })
    ));
}

#[test]
fn height_solution_has_no_post_overlay_height_repair_path() {
    let source = [
        include_str!("../height.rs"),
        include_str!("build.rs"),
        include_str!("carriers.rs"),
        include_str!("evaluate.rs"),
        include_str!("field.rs"),
        include_str!("grade.rs"),
        include_str!("model.rs"),
        include_str!("seams.rs"),
        include_str!("source_edges.rs"),
        include_str!("triangles.rs"),
        include_str!("vertices.rs"),
    ]
    .join("\n");
    for forbidden in [
        concat!("heighted_shape_with_", "canonical_contour_insertions"),
        concat!("heighted_contour_with_", "canonical_insertions"),
        concat!("fill_canonical_contour_", "height_insertions"),
        concat!("reheight_terminal_", "cap_band_from_base"),
        concat!("reheight_point_", "from_base"),
        concat!("from_terminal_cap_band_", "with_base"),
        concat!("evaluate_region_", "scoped_height"),
        concat!("bounded_region_", "scoped_edge_height"),
        concat!("region_scoped_", "carrier"),
        concat!("HEIGHT_SOURCE_EDGE_", "NEIGHBOR_UNITS"),
        concat!("HEIGHT_SOURCE_EDGE_", "DEDUP_DRIFT_UNITS"),
        concat!("allow_missing_height_points_", "backfill"),
        concat!("subdivided_", "height_chord"),
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical arrangement vertices must be inside their explicit height carrier, not repaired by `{forbidden}`"
        );
    }
}

#[test]
fn generated_band_contour_requires_explicit_height_points() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::CurbOrShoulder, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let contour = generated_band_contour(
        RoadSurfaceBandKind::CurbOrShoulder,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        None,
    );

    assert_eq!(
        field.extend_with_generated_contour(&contour),
        Err(NodeHeightFieldError::MissingGeneratedContourHeightPoints {
            mouth_order_index: 0,
            band_index: 0,
            source_kind: RoadSurfaceBandKind::CurbOrShoulder,
            height_field_id: field.id,
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        })
    );
    assert_eq!(
        field.patches.len(),
        1,
        "missing generated heights must not add a sampled fallback patch"
    );
}

#[test]
fn generated_band_contour_requires_source_band_index_for_height_carrier() {
    let input = conflicting_manual_input();
    let mut contour = generated_band_contour(
        RoadSurfaceBandKind::Sidewalk,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 5.0, 0.0),
            RoadVec3::new(10.0, 7.0, 0.0),
            RoadVec3::new(10.0, 7.0, 2.0),
            RoadVec3::new(0.0, 5.0, 2.0),
        ]),
    );
    contour.source_band_index = None;
    let rails = manual_rail_contours(input.node_id, input.piece_kind, vec![contour]);

    assert!(matches!(
        height_fields_by_source(&input, Some(&rails)),
        Err(
            NodeHeightFieldError::GeneratedContourMissingSourceBandIndex {
                mouth_order_index: 0,
                source_kind: RoadSurfaceBandKind::Sidewalk,
                purpose: NodeGeneratedContourPurpose::NonRoadBand,
                claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
                owner: Some(_),
            }
        )
    ));
}

#[test]
fn generated_band_contour_rejects_missing_source_band() {
    let input = conflicting_manual_input();
    let mut contour = generated_band_contour(
        RoadSurfaceBandKind::Sidewalk,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 5.0, 0.0),
            RoadVec3::new(10.0, 7.0, 0.0),
            RoadVec3::new(10.0, 7.0, 2.0),
            RoadVec3::new(0.0, 5.0, 2.0),
        ]),
    );
    contour.source_band_index = Some(99);
    let rails = manual_rail_contours(input.node_id, input.piece_kind, vec![contour]);

    assert!(matches!(
        height_fields_by_source(&input, Some(&rails)),
        Err(NodeHeightFieldError::GeneratedContourMissingSourceBand {
            mouth_order_index: 0,
            band_index: 99,
            source_kind: RoadSurfaceBandKind::Sidewalk,
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            owner: Some(_),
        })
    ));
}

#[test]
fn generated_contour_source_handoff_height_mismatch_rejects() {
    let source_support = [RoadVec3::new(5.0, 0.5, 0.0)];
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        Some(&source_support),
    )
    .expect("manual interval is a valid source height carrier");
    let contour = generated_band_contour(
        RoadSurfaceBandKind::Sidewalk,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(5.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(5.0, 0.75, 0.0),
            RoadVec3::new(10.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 2.0),
            RoadVec3::new(0.0, 0.0, 2.0),
        ]),
    );

    assert_eq!(
        field.extend_with_generated_contour(&contour),
        Err(
            NodeHeightFieldError::GeneratedContourSourceHandoffMismatch {
                mouth_order_index: 0,
                band_index: 0,
                source_kind: RoadSurfaceBandKind::Sidewalk,
                height_field_id: field.id,
                purpose: NodeGeneratedContourPurpose::NonRoadBand,
                claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
                owner: Some(NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0)),
                point_x_mm: 5000,
                point_z_mm: 0,
                source_height_mm: 500,
                contour_height_mm: 750,
            }
        )
    );
}

#[test]
fn generated_band_contour_rejects_invalid_height_carrier_contour() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::CurbOrShoulder, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let points_xz = vec![
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(10.0, 2.0),
        RoadVec2::new(0.0, 2.0),
        RoadVec2::new(10.0, 0.0),
    ];
    let height_points_world = points_xz
        .iter()
        .map(|point| RoadVec3::new(point.x, 0.0, point.y))
        .collect();
    let contour = generated_band_contour(
        RoadSurfaceBandKind::CurbOrShoulder,
        points_xz,
        Some(height_points_world),
    );

    assert!(matches!(
        field.extend_with_generated_contour(&contour),
        Err(NodeHeightFieldError::InvalidHeightCarrierContour {
            mouth_order_index: 0,
            band_index: 0,
            source_kind: RoadSurfaceBandKind::CurbOrShoulder,
            height_field_id,
            authority: NodeHeightAuthoritySource::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::NonRoadBand,
                claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            },
            ..
        }) if height_field_id == field.id
    ));
}

fn manual_rail_contours(
    node_id: u32,
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: Vec<NodeGeneratedContour>,
) -> NodeRailContourSet {
    NodeRailContourSet {
        node_id,
        piece_kind,
        contours,
        constraints: Vec::new(),
        height_carrier_paths_by_source: BTreeMap::new(),
        height_carrier_points_by_source: BTreeMap::new(),
    }
}

fn terminal_cap_band_for_height_test(
    x: f64,
    height_m: f64,
    role: TerminalCapBandRole,
) -> NodeTerminalCapBand {
    let inner_start = RoadVec3::new(x, height_m, -1.0);
    let inner_center = RoadVec3::new(x, height_m, 0.0);
    let inner_end = RoadVec3::new(x, height_m, 1.0);
    let outer_start = RoadVec3::new(x + 0.15, height_m, -1.0);
    let outer_center = RoadVec3::new(x + 0.15, height_m, 0.0);
    let outer_end = RoadVec3::new(x + 0.15, height_m, 1.0);
    NodeTerminalCapBand {
        source_band_index: 0,
        band_kind: RoadSurfaceBandKind::CurbOrShoulder,
        provenance: TerminalCapBandProvenance {
            layer_index: 0,
            role,
            left_source_band_index: 0,
            right_source_band_index: 1,
            source_boundary_start_index: 0,
            source_boundary_end_index: 1,
            inner_offset_m: 0.0,
            outer_offset_m: 0.15,
        },
        inner_path_world: vec![inner_start, inner_center, inner_end],
        outer_path_world: vec![outer_start, outer_center, outer_end],
        contour_world: vec![
            inner_start,
            inner_center,
            inner_end,
            outer_end,
            outer_center,
            outer_start,
        ],
    }
}

fn generated_band_contour(
    kind: RoadSurfaceBandKind,
    points_xz: Vec<RoadVec2>,
    height_points_world: Option<Vec<RoadVec3>>,
) -> NodeGeneratedContour {
    NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band { kind },
        purpose: NodeGeneratedContourPurpose::NonRoadBand,
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        owner: Some(NodeBandOwner::new(kind, 0)),
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        backend_polyline: road_points_to_polyline(points_xz.clone(), true),
        points_xz,
        height_points_world,
    }
}

#[test]
fn terminal_material_band_height_field_keeps_curb_cap_inner_rail_raised() {
    let inner_start = RoadVec3::new(0.0, 0.12, -1.0);
    let inner_center = RoadVec3::new(0.0, 0.12, 0.0);
    let inner_end = RoadVec3::new(0.0, 0.12, 1.0);
    let outer_start = RoadVec3::new(0.15, 0.12, -1.0);
    let outer_center = RoadVec3::new(0.15, 0.12, 0.0);
    let outer_end = RoadVec3::new(0.15, 0.12, 1.0);
    let cap_band = NodeTerminalCapBand {
        source_band_index: 0,
        band_kind: RoadSurfaceBandKind::CurbOrShoulder,
        provenance: TerminalCapBandProvenance {
            layer_index: 0,
            role: TerminalCapBandRole::EndBand,
            left_source_band_index: 0,
            right_source_band_index: 0,
            source_boundary_start_index: 0,
            source_boundary_end_index: 1,
            inner_offset_m: 0.0,
            outer_offset_m: 0.15,
        },
        inner_path_world: vec![inner_start, inner_center, inner_end],
        outer_path_world: vec![outer_start, outer_center, outer_end],
        contour_world: vec![
            inner_start,
            inner_center,
            inner_end,
            outer_end,
            outer_center,
            outer_start,
        ],
    };
    let patch = NodeBandHeightPatch::from_terminal_cap_band(
        NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::CurbOrShoulder),
        RoadSurfaceBandKind::CurbOrShoulder,
        &cap_band,
    )
    .expect("test terminal cap is a valid height carrier");
    let height = match patch
        .evaluate_surface_height(
            NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::CurbOrShoulder),
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadVec2::new(0.0, 0.0),
        )
        .expect("center vertex should be evaluable")
    {
        NodeHeightPatchEvaluation::Inside(height) => height,
        NodeHeightPatchEvaluation::Outside(error) => {
            panic!("center vertex should be inside terminal material band: {error:?}")
        }
    };

    assert!(
        (height - 0.12).abs() <= 1.0e-6,
        "terminal curb cap inner rail must stay raised across the carriageway split"
    );
}

#[test]
fn terminal_cap_height_field_extends_with_explicit_cap_patches_only() {
    let first_cap = terminal_cap_band_for_height_test(0.0, 0.12, TerminalCapBandRole::EndBand);
    let second_cap = terminal_cap_band_for_height_test(1.0, 0.32, TerminalCapBandRole::RightSide);
    let mut field = NodeBandHeightField::from_terminal_cap_band(0, &first_cap)
        .expect("test terminal cap is a valid height carrier");

    field
        .extend_with_terminal_cap_band(0, &second_cap)
        .expect("same terminal source may carry multiple explicit cap patches");

    let second_height = field
        .evaluate_height(RoadVec2::new(1.0, 0.0))
        .expect("second terminal cap patch should be an explicit carrier");
    assert!((second_height - 0.32).abs() <= 1.0e-6);
    assert!(matches!(
        field.evaluate_height(RoadVec2::new(0.5, 0.0)),
        Err(NodeHeightFieldError::VertexOutsideHeightField { .. })
    ));
}

#[test]
fn shared_xz_vertices_keep_distinct_owner_source_heights() {
    let regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
        ),
    ];

    validate_shared_source_height_agreement(&regions)
        .expect("different owner/source contexts are explicit seams, not height repairs");

    assert_eq!(regions[0].shape[0][0].height_m, 0.0);
    assert_eq!(regions[1].shape[0][0].height_m, 0.25);
}

#[test]
fn shared_xz_vertices_without_explicit_seam_are_not_height_constrained() {
    let regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
        ),
    ];

    validate_explicit_material_seam_heights(&regions)
        .expect("missing explicit seam must not trigger coincident-XZ height repair");

    assert_eq!(regions[0].shape[0][0].height_m, 0.0);
    assert_eq!(regions[1].shape[0][0].height_m, 0.25);
}

#[test]
fn junctionn_same_material_shared_vertices_reject_height_conflict() {
    let mut regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::Carriageway,
            9,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 2.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Carriageway,
            14,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 1.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            1,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 0.25)],
        ),
    ];

    assert!(matches!(
        apply_junctionn_height_authority_normalization(&mut regions),
        Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
    ));

    assert_eq!(regions[0].shape[0][0].height_m, 2.0);
    assert_eq!(
        regions[1].shape[0][0].height_m, 1.0,
        "same-material owner priority must not rewrite conflicting sampled heights"
    );
    assert_eq!(
        regions[2].shape[0][0].height_m, 0.25,
        "different materials must not be pulled into the same-material tie-break"
    );
}

#[test]
fn junctionn_same_material_raised_step_contact_allows_vertical_height_split() {
    let seam = manual_seam_constraint(
        88,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        false,
        true,
    );
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.25)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions).expect(
        "explicit same-material raised-step contacts are height splits, not shared-height repairs",
    );

    assert_eq!(regions[0].shape[0][0].height_m, 1.0);
    assert_eq!(regions[1].shape[0][0].height_m, 1.25);
}

#[test]
fn junctionn_same_material_shared_vertices_share_authority_when_height_keys_match() {
    let mut regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::Carriageway,
            9,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 2.0004)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Carriageway,
            14,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 2.00049)],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("matching height keys may share deterministic same-material authority");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m).as_i64(),
        2000
    );
    assert_eq!(
        regions[1].shape[0][0]
            .grade_authority
            .expect("carrier should record deterministic same-material authority")
            .decision,
        NodeGradeCarrierDecision::SameMaterialVertex
    );
}

#[test]
fn junctionn_node_grade_carrier_does_not_adopt_explicit_material_seam_for_same_material_vertex() {
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let other_sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 5);
    let seam = manual_owned_pair_seam_constraint(77, curb_owner, sidewalk_owner, true);
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            other_sidewalk_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 2.0)],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("unconstrained same-material vertex should remain independently heighted");

    assert_eq!(
        regions[0].shape[0][0].height_m, 1.0,
        "explicit curb/sidewalk seam containment must outrank same-material tie-breaks"
    );
    assert_eq!(
        regions[1].shape[0][0].height_m, 2.0,
        "unconstrained same-material vertices must not be pulled to explicit seam height"
    );
    assert!(regions[1].shape[0][0].grade_authority.is_none());
    assert_eq!(regions[2].shape[0][0].height_m, 1.0);
    validate_explicit_material_seam_heights(&regions)
        .expect("preserved seam heights should still validate");
}

#[test]
fn same_material_seam_rejects_shared_height_disagreement() {
    let seam = manual_seam_constraint(
        88,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        true,
        false,
    );
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.25)],
            vec![seam],
        ),
    ];

    assert!(matches!(
        apply_junctionn_height_authority_normalization(&mut regions),
        Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
    ));
}

#[test]
fn explicit_curb_sidewalk_seam_rejects_shared_height_disagreement() {
    let seam = manual_seam_constraint(
        12,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        true,
        true,
    );
    let regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            vec![seam],
        ),
    ];

    assert!(matches!(
        validate_explicit_material_seam_heights(&regions),
        Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
    ));
}

#[test]
fn explicit_curb_sidewalk_seam_accepts_matching_quantized_shared_height() {
    let seam = manual_seam_constraint(
        12,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        true,
        true,
    );
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.2504)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25049)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("explicit material seams may normalize only equal height keys");
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m),
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m)
    );
    validate_explicit_material_seam_heights(&regions)
        .expect("explicit seam authority may only accept matching height keys");
}

#[test]
fn same_source_constraint_index_keeps_distinct_owner_pair_height_contexts() {
    let first_pair = manual_owned_pair_seam_constraint(
        12,
        NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0),
        NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1),
        true,
    );
    let second_pair = manual_owned_pair_seam_constraint(
        12,
        NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2),
        NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3),
        true,
    );
    let regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![first_pair.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![first_pair],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            2,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 2.0)],
            vec![second_pair.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            3,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 2.0)],
            vec![second_pair],
        ),
    ];

    validate_explicit_material_seam_heights(&regions)
        .expect("same source rail index may materialize distinct final owner-pair seams");
}

#[test]
fn asphalt_curb_seams_allow_explicit_vertical_height_step() {
    let seam = manual_seam_constraint(
        3,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        false,
        true,
    );
    let regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Carriageway,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            vec![seam],
        ),
    ];

    validate_explicit_material_seam_heights(&regions)
        .expect("asphalt / curb contact is a vertical material step, not shared-height repair");
}

#[test]
fn shared_xz_vertices_reject_same_source_height_conflict() {
    let regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            0,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
        ),
    ];

    assert!(matches!(
        validate_shared_source_height_agreement(&regions),
        Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
    ));
}

fn has_vertex_height(
    region: &NodeHeightedRegion,
    expected_x: f64,
    expected_z: f64,
    expected_height: f64,
) -> bool {
    region.shape.iter().flatten().any(|vertex| {
        (vertex.point_xz.x - expected_x).abs() <= 1.0e-6
            && (vertex.point_xz.y - expected_z).abs() <= 1.0e-6
            && (vertex.height_m - expected_height).abs() <= 1.0e-6
    })
}

fn conflicting_manual_input() -> NodeArrangementInput {
    NodeArrangementInput {
        node_id: 77,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        mouths: vec![NodeInputMouth {
            order_index: 0,
            edge_idx: 9,
            side: IncidentEdgeSide::Start,
            direction_xz: RoadVec2::X,
            direction_angle_ccw: 0.0,
            conflict_handoff_distance_m: 10.0,
            mouth_rails: Vec::new(),
            endpoint_rails: Vec::new(),
            boundary_rails: Vec::new(),
            band_intervals: vec![
                manual_interval(0, RoadSurfaceBandKind::Carriageway, 2.0, 4.0),
                manual_interval(1, RoadSurfaceBandKind::Sidewalk, 5.0, 7.0),
            ],
            uses_explicit_band_domain_paths: false,
        }],
    }
}

fn manual_interval(
    band_index: usize,
    band_kind: RoadSurfaceBandKind,
    endpoint_height: f64,
    mouth_height: f64,
) -> NodeInputBandInterval {
    NodeInputBandInterval {
        band_index,
        band_kind,
        mouth_start_world: RoadVec3::new(10.0, mouth_height, 0.0),
        mouth_end_world: RoadVec3::new(10.0, mouth_height, 2.0),
        endpoint_start_world: RoadVec3::new(0.0, endpoint_height, 0.0),
        endpoint_end_world: RoadVec3::new(0.0, endpoint_height, 2.0),
        start_path_world: Vec::new(),
        end_path_world: Vec::new(),
    }
}

fn manual_region(
    kind: RoadSurfaceBandKind,
    band_index: usize,
    area_m2: f32,
) -> NodeBooleanOwnedRegion {
    NodeBooleanOwnedRegion {
        kind,
        owner: NodeBandOwner::new(kind, band_index),
        claim_priority:
            crate::simulation::network::surface::rails::NodeGeneratedContourClaimPriority::MouthBand,
        source_mouth_order_index: 0,
        source_band_index: Some(band_index),
        shape: vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 2.0], [0.0, 2.0]]],
        area_m2,
        seam_constraints: Vec::new(),
    }
}

fn manual_heighted_region(
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    area_m2: f32,
    contour: NodeHeightedContour,
) -> NodeHeightedRegion {
    manual_heighted_region_with_seams(kind, owner_index, area_m2, contour, Vec::new())
}

fn manual_heighted_region_with_seams(
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    area_m2: f32,
    contour: NodeHeightedContour,
    seam_constraints: Vec<NodeRegionSeamConstraint>,
) -> NodeHeightedRegion {
    let height_field_id = NodeBandHeightFieldId::new(owner_index, owner_index, kind);
    let contour = contour
        .into_iter()
        .map(|mut vertex| {
            vertex.height_field_id = height_field_id;
            vertex
        })
        .collect();
    NodeHeightedRegion {
        kind,
        owner: NodeBandOwner::new(kind, owner_index),
        height_field_id,
        shape: vec![contour],
        area_m2,
        seam_constraints,
    }
}

fn manual_seam_constraint(
    constraint_index: usize,
    seam_source: NodeSeamSource,
    constrains_shared_height: bool,
    is_material_transition: bool,
) -> NodeRegionSeamConstraint {
    NodeRegionSeamConstraint {
        constraint_index,
        seam_source,
        owner: None,
        opposite_owner: None,
        constrains_shared_height,
        is_material_transition,
        start_xz: RoadVec2::new(0.0, 0.0),
        end_xz: RoadVec2::new(1.0, 0.0),
    }
}

fn manual_owned_pair_seam_constraint(
    constraint_index: usize,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    constrains_shared_height: bool,
) -> NodeRegionSeamConstraint {
    NodeRegionSeamConstraint {
        constraint_index,
        seam_source: NodeSeamSource::RaisedStepContact {
            owner_index: owner.owner_index(),
        },
        owner: Some(owner),
        opposite_owner: Some(opposite_owner),
        constrains_shared_height,
        is_material_transition: true,
        start_xz: RoadVec2::new(0.0, 0.0),
        end_xz: RoadVec2::new(1.0, 0.0),
    }
}

fn manual_heighted_vertex(x: f64, z: f64, height_m: f64) -> NodeHeightedVertex {
    NodeHeightedVertex {
        point_xz: RoadVec2::new(x, z),
        height_m,
        height_field_id: NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk),
        height_authority: None,
        grade_authority: None,
    }
}
