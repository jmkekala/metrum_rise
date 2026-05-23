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
use crate::simulation::network::surface::node::height::{
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

mod boundary_reports;
mod cdt_conflicts;
mod crossings;
mod handoff_bridge;
mod mapping;
