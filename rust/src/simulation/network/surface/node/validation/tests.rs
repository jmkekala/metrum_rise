//! Validation tests.

use super::crossings::canonical_key_segments_strictly_intersect;
use super::report::{
    NodeGeometryBackend, NodeGeometryStage, NodeInvalidConstraintReason, NodeRejectedResidualKind,
    NodeSeamConstraintFailureReason,
};
use super::*;
use crate::simulation::network::surface::arrangement::{
    NodeArrangement, NodeArrangementDiagnostic, NodeArrangementError, NodeArrangementKey,
    NodeBandHeightFieldId, NodeBandOwner, NodeExplicitVerticalStepSegment,
};
use crate::simulation::network::surface::backend::{RoadVec2, RoadVec3};
use crate::simulation::network::surface::height::{
    NodeHeightAuthoritySource, NodeHeightFieldError, NodeHeightSolution,
};
use crate::simulation::network::surface::input::NodeArrangementInput;
use crate::simulation::network::surface::node::grade::{
    NodeGradeCarrierDecision, NodeGradeVertexAuthority,
};
use crate::simulation::network::surface::ownership::{
    NodeBooleanOwnership, NodeBooleanOwnershipError, NodeOwnedRegionArrangementDiagnostic,
    NodeOwnedRegionArrangementKey,
};
use crate::simulation::network::surface::rails::{
    NodeGeneratedContourClaimPriority, NodeGeneratedContourPurpose, NodeRailContourSet,
};
use crate::simulation::network::surface::triangulation::{
    NodeTriangulatedRegion, NodeTriangulatedTriangle, NodeTriangulatedVertex,
    NodeTriangulationSolution,
};
use crate::simulation::network::surface::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind,
};
use godot::prelude::{Vector2, Vector3};

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

fn solved_triangulation() -> NodeTriangulationSolution {
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
    let input = NodeArrangementInput::from_ordered_mouths(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &[mouth],
    )
    .expect("test mouth should produce canonical input");
    let rails = NodeRailContourSet::from_input(&input).expect("test input should produce rails");
    let ownership =
        NodeBooleanOwnership::from_rails(&rails).expect("test rails should produce ownership");
    let heights = NodeHeightSolution::from_ownership_and_input(&input, &ownership)
        .expect("test ownership should height canonical regions");
    let arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("test heights should produce canonical arrangement");
    NodeTriangulationSolution::from_arrangement(&arrangement)
        .expect("test arrangement should triangulate")
}

fn manual_region_with_kind(
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    height_field_id: NodeBandHeightFieldId,
    vertices: Vec<RoadVec3>,
) -> NodeTriangulatedRegion {
    manual_region_with_constraints_and_triangles(
        kind,
        owner_index,
        height_field_id,
        vertices,
        vec![[0, 1], [1, 2], [0, 2]],
        vec![NodeTriangulatedTriangle {
            vertices: [0, 1, 2],
        }],
        0.5,
    )
}

fn manual_region_with_constraints_and_triangles(
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    height_field_id: NodeBandHeightFieldId,
    vertices: Vec<RoadVec3>,
    boundary_constraints: Vec<[usize; 2]>,
    triangles: Vec<NodeTriangulatedTriangle>,
    area_m2: f32,
) -> NodeTriangulatedRegion {
    NodeTriangulatedRegion {
        kind,
        owner: NodeBandOwner::new(kind, owner_index),
        height_field_id,
        vertices: vertices
            .into_iter()
            .map(|point_world| NodeTriangulatedVertex {
                point_world,
                height_field_id,
                grade_authority: NodeGradeVertexAuthority::new(
                    super::super::backend::RoadVec2::new(point_world.x, point_world.z),
                    point_world.y,
                    NodeBandOwner::new(kind, owner_index),
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
            })
            .collect(),
        boundary_constraints,
        triangles,
        area_m2,
    }
}

fn split_bridge_region(
    owner_index: usize,
    height_field_id: NodeBandHeightFieldId,
) -> NodeTriangulatedRegion {
    manual_region_with_constraints_and_triangles(
        RoadSurfaceBandKind::Carriageway,
        owner_index,
        height_field_id,
        vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(0.5, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(0.0, 0.0, 1.0),
            RoadVec3::new(0.5, 0.0, 1.0),
            RoadVec3::new(1.0, 0.0, 1.0),
        ],
        vec![[0, 1], [1, 2], [2, 5], [3, 5], [0, 3]],
        vec![
            NodeTriangulatedTriangle {
                vertices: [0, 1, 3],
            },
            NodeTriangulatedTriangle {
                vertices: [1, 4, 3],
            },
            NodeTriangulatedTriangle {
                vertices: [1, 2, 4],
            },
            NodeTriangulatedTriangle {
                vertices: [2, 5, 4],
            },
        ],
        1.0,
    )
}

fn long_bridge_region(
    owner_index: usize,
    height_field_id: NodeBandHeightFieldId,
) -> NodeTriangulatedRegion {
    manual_region_with_constraints_and_triangles(
        RoadSurfaceBandKind::Carriageway,
        owner_index,
        height_field_id,
        vec![
            RoadVec3::new(-1.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            RoadVec3::new(0.0, 0.0, 1.0),
        ],
        vec![[0, 1], [1, 2], [0, 2]],
        vec![NodeTriangulatedTriangle {
            vertices: [0, 1, 2],
        }],
        1.5,
    )
}

fn gapped_bridge_region(
    owner_index: usize,
    height_field_id: NodeBandHeightFieldId,
) -> NodeTriangulatedRegion {
    manual_region_with_constraints_and_triangles(
        RoadSurfaceBandKind::Carriageway,
        owner_index,
        height_field_id,
        vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(0.4, 0.0, 0.0),
            RoadVec3::new(0.0, 0.0, 1.0),
            RoadVec3::new(0.4, 0.0, 1.0),
            RoadVec3::new(0.6, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(0.6, 0.0, 1.0),
            RoadVec3::new(1.0, 0.0, 1.0),
        ],
        vec![
            [0, 1],
            [1, 3],
            [2, 3],
            [0, 2],
            [4, 5],
            [5, 7],
            [6, 7],
            [4, 6],
        ],
        vec![
            NodeTriangulatedTriangle {
                vertices: [0, 1, 2],
            },
            NodeTriangulatedTriangle {
                vertices: [1, 3, 2],
            },
            NodeTriangulatedTriangle {
                vertices: [4, 5, 6],
            },
            NodeTriangulatedTriangle {
                vertices: [5, 7, 6],
            },
        ],
        0.8,
    )
}

fn report_has_cross_region_height_conflict(report: &NodeValidationReport) -> bool {
    report.diagnostics.iter().any(|diagnostic| {
        diagnostic.stage == NodeGeometryStage::Validation
            && diagnostic.backend == NodeGeometryBackend::Spade
            && matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::CrossRegionHeightConflict { .. }
            )
    })
}

fn key_point(x: f64, z: f64) -> NodeValidationPointKey {
    let key = SurfaceXzKey::from_road_xz(super::super::backend::RoadVec2::new(x, z));
    NodeValidationPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
    }
}

fn key_edge(a: [f64; 2], b: [f64; 2]) -> NodeValidationEdgeKey {
    NodeValidationEdgeKey::new(key_point(a[0], a[1]), key_point(b[0], b[1]))
}

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

#[test]
fn accepts_same_height_handoff_with_complete_split_bridge_edge_coverage() {
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
        node_id: 103,
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
            split_bridge_region(2, joined_asphalt_field),
            manual_region_with_kind(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(0.5, 0.12, -1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![asphalt_handoff, curb_step],
    };

    NodeValidationReport::from_triangulation_solution(&solution)
        .expect("split bridge edges fully covering the seam should carry handoff authority");
}

#[test]
fn accepts_same_height_handoff_when_bridge_edge_contains_conflict_edge() {
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
        node_id: 106,
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
            long_bridge_region(2, joined_asphalt_field),
            manual_region_with_kind(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(0.5, 0.12, -1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![asphalt_handoff, curb_step],
    };

    NodeValidationReport::from_triangulation_solution(&solution)
        .expect("a longer exact bridge edge may prove complete conflict-edge coverage");
}

#[test]
fn rejects_same_height_handoff_with_bridge_endpoints_but_no_bridge_edge() {
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
    let endpoint_only_bridge = manual_region_with_constraints_and_triangles(
        RoadSurfaceBandKind::Carriageway,
        2,
        joined_asphalt_field,
        vec![RoadVec3::new(0.0, 0.0, 0.0), RoadVec3::new(1.0, 0.0, 0.0)],
        Vec::new(),
        Vec::new(),
        0.0,
    );
    let solution = NodeTriangulationSolution {
        node_id: 104,
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
            endpoint_only_bridge,
            manual_region_with_kind(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(0.5, 0.12, -1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![asphalt_handoff, curb_step],
    };

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("endpoint-only bridge ownership must not authorize a height conflict");
    assert!(report_has_cross_region_height_conflict(&error.report));
}

#[test]
fn rejects_same_height_handoff_with_gapped_split_bridge_edge_coverage() {
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
        node_id: 105,
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
            gapped_bridge_region(2, joined_asphalt_field),
            manual_region_with_kind(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                curb_field,
                vec![
                    RoadVec3::new(0.0, 0.12, 0.0),
                    RoadVec3::new(1.0, 0.12, 0.0),
                    RoadVec3::new(0.5, 0.12, -1.0),
                ],
            ),
        ],
        explicit_vertical_step_segments: vec![asphalt_handoff, curb_step],
    };

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("gapped bridge ownership must not authorize a full-edge height conflict");
    assert!(report_has_cross_region_height_conflict(&error.report));
}

#[test]
fn accepts_cross_region_cdt_edge_height_conflict_on_canonical_asphalt_sidewalk_step() {
    let carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let carriageway_field = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let sidewalk_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::Sidewalk);
    let step_segment = NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0)),
        NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
        carriageway_owner,
        sidewalk_owner,
    )
    .expect("non-degenerate test step segment");
    let solution = NodeTriangulationSolution {
        node_id: 101,
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
                RoadSurfaceBandKind::Sidewalk,
                1,
                sidewalk_field,
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
        .expect("canonical asphalt-sidewalk vertical step should allow the height delta");
}

#[test]
fn reports_open_boundaries_with_stage_and_backend() {
    let mut solution = solved_triangulation();
    solution.regions[0].boundary_constraints.pop();

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("missing explicit boundary constraint must fail validation");

    assert!(error.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.stage == NodeGeometryStage::Validation
            && diagnostic.backend == NodeGeometryBackend::CanonicalKeys
            && matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::OpenBoundary { .. }
            )
    }));
    let dump = error.report.debug_dump();
    assert!(dump.contains("\"stage\":\"validation\""));
    assert!(dump.contains("\"backend\":\"canonical_keys\""));
    assert!(dump.contains("\"kind\":\"open_boundary\""));
}

#[test]
fn reports_crossing_constraints() {
    let mut solution = solved_triangulation();
    let region = &mut solution.regions[0];
    region.boundary_constraints = vec![[0, 2], [1, 3], [0, 1], [2, 3]];

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("crossing constraints must fail validation");

    assert!(error.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.backend == NodeGeometryBackend::CanonicalKeys
            && matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    reason: NodeInvalidConstraintReason::Crossing,
                    ..
                }
            )
    }));
    assert!(
        !error.report.has_blocking_diagnostics(),
        "crossing constraints remain diagnostic-only when CDT output and coverage are valid"
    );
}

#[test]
fn canonical_key_crossing_rejects_logged_microscopic_connector_false_positive() {
    let microscopic_connector = key_edge([-63.632900, -27.195601], [-63.632896, -27.195602]);
    let boundary = key_edge([-64.056534, -30.669868], [-58.100647, -31.396107]);

    assert!(
        !canonical_key_segments_strictly_intersect(microscopic_connector, boundary),
        "logged terminal sample is not a true canonical interior/interior crossing"
    );
}

#[test]
fn canonical_key_crossing_reports_only_true_interior_intersections() {
    assert!(canonical_key_segments_strictly_intersect(
        key_edge([0.0, 0.0], [2.0, 2.0]),
        key_edge([0.0, 2.0], [2.0, 0.0])
    ));
    assert!(!canonical_key_segments_strictly_intersect(
        key_edge([0.0, 0.0], [1.0, 1.0]),
        key_edge([1.0, 1.0], [2.0, 0.0])
    ));
    assert!(!canonical_key_segments_strictly_intersect(
        key_edge([0.0, 0.0], [2.0, 0.0]),
        key_edge([2.0, 0.0], [3.0, 0.0])
    ));
    assert!(canonical_key_segments_strictly_intersect(
        key_edge([0.0, 0.0], [3.0, 0.0]),
        key_edge([1.0, 0.0], [2.0, 0.0])
    ));
}

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
