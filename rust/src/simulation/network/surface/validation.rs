//! Structured validation and diagnostics for canonical node surface compilation.

use super::arrangement::{
    NodeArrangement, NodeArrangementDiagnostic, NodeArrangementError, NodeArrangementKey,
    NodeBandHeightFieldId, NodeBandOwner, NodeExplicitVerticalStepSegment,
    owners_form_explicit_vertical_step_pair,
};
use super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
use super::height::{NodeHeightAuthoritySource, NodeHeightFieldError};
use super::ownership::{
    NodeBooleanOwnershipError, NodeOwnedRegionArrangement, NodeOwnedRegionArrangementDiagnostic,
};
use super::rails::NodeRailGenerationError;
use super::triangulation::{
    NodeTriangulatedRegion, NodeTriangulatedTriangle, NodeTriangulatedVertex,
    NodeTriangulationError, NodeTriangulationSolution,
};
use super::{
    NodeOverlayContour, RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use parry2d::math::{Pose, Vector};
use parry2d::query::PointQuery;
use parry2d::shape::Segment;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const VALIDATION_KEY_SCALE: f64 = 1000.0;
const VALIDATION_POINT_KEY_SCALE: f64 = ROAD_OVERLAY_COORDINATE_SCALE;
const VALIDATION_MIN_SEGMENT_LENGTH_M: f32 = 0.000001;
const VALIDATION_DUPLICATE_EXPOSED_EDGE_CANONICAL_DRIFT_M: f64 = 0.01;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeValidationReport {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) region_count: usize,
    pub(crate) triangle_count: usize,
    pub(crate) exposed_edge_count: usize,
    pub(crate) diagnostics: Vec<NodeGeometryDiagnostic>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeValidationError {
    pub(crate) report: NodeValidationReport,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeGeometryDiagnostic {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) stage: NodeGeometryStage,
    pub(crate) backend: NodeGeometryBackend,
    pub(crate) kind: NodeGeometryDiagnosticKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeGeometryStage {
    ContourGeneration,
    BooleanOwnership,
    HeightEvaluation,
    Validation,
    CdtTriangulation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeGeometryBackend {
    CavalierContours,
    IOverlay,
    HeightCarrier,
    CanonicalKeys,
    Parry2d,
    Spade,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeGeometryDiagnosticKind {
    RejectedResidual {
        residual: NodeRejectedResidualKind,
        shape_count: usize,
        area_m2: f32,
    },
    NonExplicitBoundaryVertex {
        region_index: usize,
        x_mm: i64,
        z_mm: i64,
        min_boundary_distance_mm: i64,
    },
    HeightConflict {
        x_mm: i64,
        z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    SourceHeightFieldConflict {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        owner: Option<NodeBandOwner>,
        existing_authority: NodeHeightAuthoritySource,
        incoming_authority: NodeHeightAuthoritySource,
        x_mm: i64,
        z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    SharedSourceHeightConflict {
        x_mm: i64,
        z_mm: i64,
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        opposite_owner: Option<NodeBandOwner>,
        height_field_id: Option<NodeBandHeightFieldId>,
        incoming_owner: NodeBandOwner,
        incoming_height_field_id: Option<NodeBandHeightFieldId>,
        constraint_index: Option<usize>,
        existing_authority: Option<NodeHeightAuthoritySource>,
        incoming_authority: Option<NodeHeightAuthoritySource>,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    CrossRegionHeightConflict {
        edge_start_x_key: i64,
        edge_start_z_key: i64,
        edge_end_x_key: i64,
        edge_end_z_key: i64,
        edge_start_x_mm: i64,
        edge_start_z_mm: i64,
        edge_end_x_mm: i64,
        edge_end_z_mm: i64,
        conflict_x_key: i64,
        conflict_z_key: i64,
        conflict_x_mm: i64,
        conflict_z_mm: i64,
        existing_region_index: usize,
        existing_owner: RoadSurfaceBandKind,
        existing_owner_index: usize,
        existing_start_height_mm: i64,
        existing_end_height_mm: i64,
        existing_conflict_height_mm: i64,
        incoming_region_index: usize,
        incoming_owner: RoadSurfaceBandKind,
        incoming_owner_index: usize,
        incoming_start_height_mm: i64,
        incoming_end_height_mm: i64,
        incoming_conflict_height_mm: i64,
        matching_explicit_step_segments: Vec<NodeExplicitStepSegmentDiagnostic>,
        non_matching_explicit_step_segments: Vec<NodeExplicitStepSegmentDiagnostic>,
    },
    HeightFieldFailure {
        reason: &'static str,
        mouth_order_index: Option<usize>,
        band_index: Option<usize>,
        kind: Option<RoadSurfaceBandKind>,
        source_kind: Option<RoadSurfaceBandKind>,
        height_field_id: Option<NodeBandHeightFieldId>,
        owner: Option<NodeBandOwner>,
        point_x_mm: Option<i64>,
        point_z_mm: Option<i64>,
        axis: Option<&'static str>,
        raw_parameter: Option<f64>,
    },
    OpenBoundary {
        region_index: usize,
        vertex_index: Option<usize>,
        degree: usize,
    },
    DuplicateExposedEdge {
        region_index: Option<usize>,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
        count: usize,
    },
    InvalidConstraint {
        region_index: usize,
        constraint_index: Option<usize>,
        reason: NodeInvalidConstraintReason,
    },
    TriangleCoverageMismatch {
        region_index: usize,
        missing_area_m2: f32,
        extra_area_m2: f32,
    },
    TriangleOverlap {
        region_index: usize,
        overlap_area_m2: f32,
    },
    SeamConstraintFailure {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        opposite_owner: RoadSurfaceBandKind,
        opposite_owner_index: usize,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
        reason: NodeSeamConstraintFailureReason,
    },
    UnmaterializedRaisedStepAuthority {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        opposite_owner: RoadSurfaceBandKind,
        opposite_owner_index: usize,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
        source_constraint_indices: Vec<usize>,
    },
    BackendFailure {
        reason: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeRejectedResidualKind {
    Asphalt,
    Band(RoadSurfaceBandKind),
    NonRoad,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeInvalidConstraintReason {
    Degenerate,
    OutOfRange,
    Crossing,
    Duplicate,
    CdtRejected,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeSeamConstraintFailureReason {
    Missing,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeExplicitStepSegmentDiagnostic {
    pub(crate) segment_index: usize,
    pub(crate) start_x_key: i64,
    pub(crate) start_z_key: i64,
    pub(crate) end_x_key: i64,
    pub(crate) end_z_key: i64,
    pub(crate) start_x_mm: i64,
    pub(crate) start_z_mm: i64,
    pub(crate) end_x_mm: i64,
    pub(crate) end_z_mm: i64,
    pub(crate) owner: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) opposite_owner: RoadSurfaceBandKind,
    pub(crate) opposite_owner_index: usize,
    pub(crate) owners_match_regions: bool,
    pub(crate) edge_lies_on_segment: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeValidationPointKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeValidationEdgeKey {
    start: NodeValidationPointKey,
    end: NodeValidationPointKey,
}

#[derive(Clone, Copy)]
struct BoundarySegment {
    index: usize,
    edge: [usize; 2],
    key_edge: NodeValidationEdgeKey,
    segment: Segment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightedTriangleEdge {
    region_index: usize,
    start_height_mm: i64,
    end_height_mm: i64,
}

impl RoadSurfaceSystem {
    pub(super) fn validate_node_triangulation_solution(
        solution: &NodeTriangulationSolution,
    ) -> Result<NodeValidationReport, NodeValidationError> {
        NodeValidationReport::from_triangulation_solution(solution)
    }
}

fn validate_cross_region_triangle_edge_heights(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    let mut edges = BTreeMap::<NodeValidationEdgeKey, Vec<HeightedTriangleEdge>>::new();
    for (region_index, region) in solution.regions.iter().enumerate() {
        for triangle in &region.triangles {
            if !triangle_indices_valid(triangle, region.vertices.len()) {
                continue;
            }
            for edge in triangle_edges(triangle) {
                let (edge_key, heighted_edge) =
                    heighted_triangle_edge_for_indices(region_index, region, edge);
                edges.entry(edge_key).or_default().push(heighted_edge);
            }
        }
    }

    for (edge_key, mut heighted_edges) in edges {
        heighted_edges.sort_unstable();
        heighted_edges.dedup();
        'edge: for left_index in 0..heighted_edges.len() {
            for right_index in left_index + 1..heighted_edges.len() {
                let left = heighted_edges[left_index];
                let right = heighted_edges[right_index];
                if left.region_index == right.region_index
                    || (left.start_height_mm == right.start_height_mm
                        && left.end_height_mm == right.end_height_mm)
                    || cross_region_edges_form_explicit_vertical_step(
                        solution, edge_key, left, right,
                    )
                {
                    continue;
                }
                push_triangle_edge_height_conflict(solution, diagnostics, edge_key, left, right);
                break 'edge;
            }
        }
    }
}

fn heighted_triangle_edge_for_indices(
    region_index: usize,
    region: &NodeTriangulatedRegion,
    edge: [usize; 2],
) -> (NodeValidationEdgeKey, HeightedTriangleEdge) {
    let start = region.vertices[edge[0]].point_world;
    let end = region.vertices[edge[1]].point_world;
    let start_key = point_key_from_world(start);
    let end_key = point_key_from_world(end);
    let start_height_mm = quantize_m(start.y);
    let end_height_mm = quantize_m(end.y);
    if start_key <= end_key {
        let edge_key = NodeValidationEdgeKey {
            start: start_key,
            end: end_key,
        };
        (
            edge_key,
            HeightedTriangleEdge {
                region_index,
                start_height_mm,
                end_height_mm,
            },
        )
    } else {
        let edge_key = NodeValidationEdgeKey {
            start: end_key,
            end: start_key,
        };
        (
            edge_key,
            HeightedTriangleEdge {
                region_index,
                start_height_mm: end_height_mm,
                end_height_mm: start_height_mm,
            },
        )
    }
}

fn cross_region_edges_form_explicit_vertical_step(
    solution: &NodeTriangulationSolution,
    edge: NodeValidationEdgeKey,
    left: HeightedTriangleEdge,
    right: HeightedTriangleEdge,
) -> bool {
    let Some((left_region, right_region)) = solution
        .regions
        .get(left.region_index)
        .zip(solution.regions.get(right.region_index))
    else {
        return false;
    };
    if !owners_form_explicit_vertical_step_pair(left_region.owner, right_region.owner) {
        return false;
    }
    if solution
        .explicit_vertical_step_segments
        .iter()
        .copied()
        .any(|segment| {
            explicit_vertical_step_owners_match_regions(
                segment,
                left_region.owner,
                right_region.owner,
            ) && edge_lies_on_explicit_vertical_step(segment, edge)
        })
    {
        return true;
    }
    cross_region_edges_form_same_height_owner_handoff_explicit_vertical_step(
        solution,
        edge,
        left_region.owner,
        left,
        right_region.owner,
        right,
    )
}

fn cross_region_edges_form_same_height_owner_handoff_explicit_vertical_step(
    solution: &NodeTriangulationSolution,
    edge: NodeValidationEdgeKey,
    left_owner: super::arrangement::NodeBandOwner,
    left: HeightedTriangleEdge,
    right_owner: super::arrangement::NodeBandOwner,
    right: HeightedTriangleEdge,
) -> bool {
    solution
        .explicit_vertical_step_segments
        .iter()
        .copied()
        .filter(|segment| edge_lies_on_explicit_vertical_step(*segment, edge))
        .any(|step_segment| {
            if explicit_vertical_step_handoff_authorizes_owner(
                solution,
                edge,
                step_segment,
                left_owner,
                left,
                right_owner,
            ) {
                return true;
            }
            explicit_vertical_step_handoff_authorizes_owner(
                solution,
                edge,
                step_segment,
                right_owner,
                right,
                left_owner,
            )
        })
}

fn explicit_vertical_step_handoff_authorizes_owner(
    solution: &NodeTriangulationSolution,
    edge: NodeValidationEdgeKey,
    step_segment: NodeExplicitVerticalStepSegment,
    missing_owner: super::arrangement::NodeBandOwner,
    missing_edge: HeightedTriangleEdge,
    direct_owner: super::arrangement::NodeBandOwner,
) -> bool {
    let Some(bridge_owner) = explicit_step_segment_bridge_owner(step_segment, direct_owner) else {
        return false;
    };
    if bridge_owner.kind() != missing_owner.kind() || bridge_owner == missing_owner {
        return false;
    }
    if !solution
        .explicit_vertical_step_segments
        .iter()
        .copied()
        .any(|segment| {
            explicit_vertical_step_owners_match_regions(segment, bridge_owner, missing_owner)
                && edge_lies_on_explicit_vertical_step(segment, edge)
        })
    {
        return false;
    }
    heighted_triangle_edge_for_owner_on_validation_edge(solution, bridge_owner, edge).is_some_and(
        |bridge_edge| {
            bridge_edge.start_height_mm == missing_edge.start_height_mm
                && bridge_edge.end_height_mm == missing_edge.end_height_mm
        },
    ) || heighted_region_endpoint_pair_for_owner_on_validation_edge(solution, bridge_owner, edge)
        .is_some_and(|bridge_edge| {
            bridge_edge.start_height_mm == missing_edge.start_height_mm
                && bridge_edge.end_height_mm == missing_edge.end_height_mm
        })
}

fn explicit_step_segment_bridge_owner(
    segment: NodeExplicitVerticalStepSegment,
    direct_owner: super::arrangement::NodeBandOwner,
) -> Option<super::arrangement::NodeBandOwner> {
    if segment.owner() == direct_owner {
        Some(segment.opposite_owner())
    } else if segment.opposite_owner() == direct_owner {
        Some(segment.owner())
    } else {
        None
    }
}

fn heighted_triangle_edge_for_owner_on_validation_edge(
    solution: &NodeTriangulationSolution,
    owner: super::arrangement::NodeBandOwner,
    edge: NodeValidationEdgeKey,
) -> Option<HeightedTriangleEdge> {
    for (region_index, region) in solution.regions.iter().enumerate() {
        if region.owner != owner {
            continue;
        }
        for triangle in &region.triangles {
            if !triangle_indices_valid(triangle, region.vertices.len()) {
                continue;
            }
            for triangle_edge in triangle_edges(triangle) {
                let (candidate, heighted_edge) =
                    heighted_triangle_edge_for_indices(region_index, region, triangle_edge);
                if candidate == edge {
                    return Some(heighted_edge);
                }
            }
        }
    }
    None
}

fn heighted_region_endpoint_pair_for_owner_on_validation_edge(
    solution: &NodeTriangulationSolution,
    owner: super::arrangement::NodeBandOwner,
    edge: NodeValidationEdgeKey,
) -> Option<HeightedTriangleEdge> {
    for (region_index, region) in solution.regions.iter().enumerate() {
        if region.owner != owner {
            continue;
        }
        let mut start_heights = BTreeSet::new();
        let mut end_heights = BTreeSet::new();
        for vertex in &region.vertices {
            let key = point_key_from_world(vertex.point_world);
            if key == edge.start {
                start_heights.insert(quantize_m(vertex.point_world.y));
            }
            if key == edge.end {
                end_heights.insert(quantize_m(vertex.point_world.y));
            }
        }
        if start_heights.len() == 1 && end_heights.len() == 1 {
            return Some(HeightedTriangleEdge {
                region_index,
                start_height_mm: *start_heights.iter().next()?,
                end_height_mm: *end_heights.iter().next()?,
            });
        }
    }
    None
}

fn edge_lies_on_explicit_vertical_step(
    segment: NodeExplicitVerticalStepSegment,
    edge: NodeValidationEdgeKey,
) -> bool {
    let start = NodeValidationPointKey::from_arrangement_key(segment.start());
    let end = NodeValidationPointKey::from_arrangement_key(segment.end());
    point_lies_on_validation_segment(edge.start, start, end)
        && point_lies_on_validation_segment(edge.end, start, end)
}

fn explicit_vertical_step_owners_match_regions(
    segment: NodeExplicitVerticalStepSegment,
    left_owner: super::arrangement::NodeBandOwner,
    right_owner: super::arrangement::NodeBandOwner,
) -> bool {
    (segment.owner() == left_owner && segment.opposite_owner() == right_owner)
        || (segment.owner() == right_owner && segment.opposite_owner() == left_owner)
}

fn point_lies_on_validation_segment(
    point: NodeValidationPointKey,
    start: NodeValidationPointKey,
    end: NodeValidationPointKey,
) -> bool {
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.x_key - start.x_key);
    let dz = i128::from(end.z_key - start.z_key);
    let px = i128::from(point.x_key - start.x_key);
    let pz = i128::from(point.z_key - start.z_key);
    let cross = px * dz - pz * dx;
    if cross != 0 && cross.abs() > validation_overlay_grid_collinearity_error_bound(dx, dz) {
        return false;
    }
    let inside_x = if start.x_key == end.x_key {
        point.x_key == start.x_key
    } else {
        point.x_key > start.x_key.min(end.x_key) && point.x_key < start.x_key.max(end.x_key)
    };
    let inside_z = if start.z_key == end.z_key {
        point.z_key == start.z_key
    } else {
        point.z_key > start.z_key.min(end.z_key) && point.z_key < start.z_key.max(end.z_key)
    };
    inside_x && inside_z
}

fn validation_overlay_grid_collinearity_error_bound(dx: i128, dz: i128) -> i128 {
    (dx.abs() + dz.abs()) * 2
}

fn push_triangle_edge_height_conflict(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
    edge: NodeValidationEdgeKey,
    existing: HeightedTriangleEdge,
    incoming: HeightedTriangleEdge,
) {
    let Some(existing_region) = solution.regions.get(existing.region_index) else {
        return;
    };
    let Some(incoming_region) = solution.regions.get(incoming.region_index) else {
        return;
    };
    let (point, existing_conflict_height_mm, incoming_conflict_height_mm) =
        if existing.start_height_mm != incoming.start_height_mm {
            (
                edge.start,
                existing.start_height_mm,
                incoming.start_height_mm,
            )
        } else {
            (edge.end, existing.end_height_mm, incoming.end_height_mm)
        };
    let (matching_explicit_step_segments, non_matching_explicit_step_segments) =
        explicit_step_segment_diagnostics_for_conflict(
            solution,
            edge,
            existing_region.owner,
            incoming_region.owner,
        );
    push_validation_diagnostic(
        solution,
        diagnostics,
        NodeGeometryBackend::Spade,
        NodeGeometryDiagnosticKind::CrossRegionHeightConflict {
            edge_start_x_key: edge.start.x_key,
            edge_start_z_key: edge.start.z_key,
            edge_end_x_key: edge.end.x_key,
            edge_end_z_key: edge.end.z_key,
            edge_start_x_mm: edge.start.x_mm(),
            edge_start_z_mm: edge.start.z_mm(),
            edge_end_x_mm: edge.end.x_mm(),
            edge_end_z_mm: edge.end.z_mm(),
            conflict_x_key: point.x_key,
            conflict_z_key: point.z_key,
            conflict_x_mm: point.x_mm(),
            conflict_z_mm: point.z_mm(),
            existing_region_index: existing.region_index,
            existing_owner: existing_region.owner.kind(),
            existing_owner_index: existing_region.owner.owner_index(),
            existing_start_height_mm: existing.start_height_mm,
            existing_end_height_mm: existing.end_height_mm,
            existing_conflict_height_mm,
            incoming_region_index: incoming.region_index,
            incoming_owner: incoming_region.owner.kind(),
            incoming_owner_index: incoming_region.owner.owner_index(),
            incoming_start_height_mm: incoming.start_height_mm,
            incoming_end_height_mm: incoming.end_height_mm,
            incoming_conflict_height_mm,
            matching_explicit_step_segments,
            non_matching_explicit_step_segments,
        },
    );
}

fn explicit_step_segment_diagnostics_for_conflict(
    solution: &NodeTriangulationSolution,
    edge: NodeValidationEdgeKey,
    existing_owner: super::arrangement::NodeBandOwner,
    incoming_owner: super::arrangement::NodeBandOwner,
) -> (
    Vec<NodeExplicitStepSegmentDiagnostic>,
    Vec<NodeExplicitStepSegmentDiagnostic>,
) {
    let mut matching = Vec::new();
    let mut non_matching = Vec::new();
    for (segment_index, segment) in solution
        .explicit_vertical_step_segments
        .iter()
        .copied()
        .enumerate()
    {
        let owners_match_regions =
            explicit_vertical_step_owners_match_regions(segment, existing_owner, incoming_owner);
        let edge_lies_on_segment = edge_lies_on_explicit_vertical_step(segment, edge);
        let segment_diagnostic = explicit_step_segment_diagnostic(
            segment_index,
            segment,
            owners_match_regions,
            edge_lies_on_segment,
        );
        if owners_match_regions && edge_lies_on_segment {
            matching.push(segment_diagnostic);
        } else {
            non_matching.push(segment_diagnostic);
        }
    }
    (matching, non_matching)
}

fn explicit_step_segment_diagnostic(
    segment_index: usize,
    segment: NodeExplicitVerticalStepSegment,
    owners_match_regions: bool,
    edge_lies_on_segment: bool,
) -> NodeExplicitStepSegmentDiagnostic {
    NodeExplicitStepSegmentDiagnostic {
        segment_index,
        start_x_key: segment.start().x_key(),
        start_z_key: segment.start().z_key(),
        end_x_key: segment.end().x_key(),
        end_z_key: segment.end().z_key(),
        start_x_mm: segment.start().x_mm(),
        start_z_mm: segment.start().z_mm(),
        end_x_mm: segment.end().x_mm(),
        end_z_mm: segment.end().z_mm(),
        owner: segment.owner().kind(),
        owner_index: segment.owner().owner_index(),
        opposite_owner: segment.opposite_owner().kind(),
        opposite_owner_index: segment.opposite_owner().owner_index(),
        owners_match_regions,
        edge_lies_on_segment,
    }
}

impl NodeValidationReport {
    pub(crate) fn from_triangulation_solution(
        solution: &NodeTriangulationSolution,
    ) -> Result<Self, NodeValidationError> {
        let mut diagnostics = Vec::new();
        let mut exposed_edges = BTreeMap::<NodeValidationEdgeKey, Vec<usize>>::new();
        let mut triangle_count = 0usize;
        let mut exposed_edge_count = 0usize;

        for (region_index, region) in solution.regions.iter().enumerate() {
            let region_exposed_edges =
                validate_region(solution, region_index, region, &mut diagnostics);
            triangle_count += region.triangles.len();
            exposed_edge_count += region_exposed_edges.len();
            for edge in region_exposed_edges {
                exposed_edges.entry(edge).or_default().push(region_index);
            }
        }

        for (edge, region_indices) in exposed_edges {
            if region_indices.len() > 2
                && !duplicate_exposed_edge_has_explicit_owner_context(solution, &region_indices)
                && !duplicate_exposed_edge_is_canonical_drift(solution, edge, &region_indices)
            {
                diagnostics.push(NodeGeometryDiagnostic {
                    node_id: solution.node_id,
                    piece_kind: solution.piece_kind,
                    stage: NodeGeometryStage::Validation,
                    backend: NodeGeometryBackend::Parry2d,
                    kind: NodeGeometryDiagnosticKind::DuplicateExposedEdge {
                        region_index: None,
                        start_x_mm: edge.start.x_mm(),
                        start_z_mm: edge.start.z_mm(),
                        end_x_mm: edge.end.x_mm(),
                        end_z_mm: edge.end.z_mm(),
                        count: region_indices.len(),
                    },
                });
            }
        }
        validate_cross_region_triangle_edge_heights(solution, &mut diagnostics);

        let report = Self {
            node_id: solution.node_id,
            piece_kind: solution.piece_kind,
            region_count: solution.regions.len(),
            triangle_count,
            exposed_edge_count,
            diagnostics,
        };
        if report.diagnostics.is_empty() {
            Ok(report)
        } else {
            Err(NodeValidationError { report })
        }
    }

    pub(crate) fn debug_dump(&self) -> String {
        let mut dump = String::new();
        let _ = write!(
            dump,
            "{{\"node_id\":{},\"piece_kind\":\"{:?}\",\"region_count\":{},\"triangle_count\":{},\"exposed_edge_count\":{},\"diagnostics\":[",
            self.node_id,
            self.piece_kind,
            self.region_count,
            self.triangle_count,
            self.exposed_edge_count
        );
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                let _ = write!(dump, ",");
            }
            let _ = write!(dump, "{}", diagnostic.debug_record());
        }
        let _ = write!(dump, "]}}");
        dump
    }

    pub(crate) fn has_blocking_diagnostics(&self) -> bool {
        self.diagnostics.iter().any(|diagnostic| {
            // Parry crossing checks are diagnostic only once Spade accepted the constraints and
            // the overlay coverage checks passed. Missing coverage and ownership failures still
            // block export.
            !matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    reason: NodeInvalidConstraintReason::Crossing,
                    ..
                }
            )
        })
    }

    pub(crate) fn from_rail_generation_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeRailGenerationError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_rail_generation_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_boolean_ownership_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeBooleanOwnershipError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_boolean_ownership_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_height_field_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeHeightFieldError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_height_field_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_triangulation_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeTriangulationError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_triangulation_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_arrangement_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeArrangementError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_arrangement_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_arrangement_diagnostics(arrangement: &NodeArrangement) -> Option<Self> {
        if arrangement.diagnostics().is_empty() {
            return None;
        }
        Some(Self {
            node_id: arrangement.node_id(),
            piece_kind: arrangement.piece_kind(),
            region_count: arrangement.regions().len(),
            triangle_count: arrangement.faces().len(),
            exposed_edge_count: arrangement.edges().len(),
            diagnostics: arrangement
                .diagnostics()
                .iter()
                .map(|diagnostic| {
                    NodeGeometryDiagnostic::from_arrangement_diagnostic(
                        arrangement.node_id(),
                        arrangement.piece_kind(),
                        diagnostic,
                    )
                })
                .collect(),
        })
    }

    pub(crate) fn from_owned_region_arrangement_diagnostics(
        arrangement: &NodeOwnedRegionArrangement,
    ) -> Option<Self> {
        if arrangement.diagnostics().is_empty() {
            return None;
        }
        Some(Self {
            node_id: arrangement.node_id(),
            piece_kind: arrangement.piece_kind(),
            region_count: arrangement.region_count(),
            triangle_count: 0,
            exposed_edge_count: arrangement.edges().len(),
            diagnostics: arrangement
                .diagnostics()
                .iter()
                .map(|diagnostic| {
                    NodeGeometryDiagnostic::from_owned_region_arrangement_diagnostic(
                        arrangement.node_id(),
                        arrangement.piece_kind(),
                        diagnostic,
                    )
                })
                .collect(),
        })
    }

    pub(crate) fn from_boundary_export_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        reason: &'static str,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::Validation,
            backend: NodeGeometryBackend::Parry2d,
            kind: NodeGeometryDiagnosticKind::BackendFailure { reason },
        })
    }

    fn single_diagnostic(diagnostic: NodeGeometryDiagnostic) -> Self {
        Self {
            node_id: diagnostic.node_id,
            piece_kind: diagnostic.piece_kind,
            region_count: 0,
            triangle_count: 0,
            exposed_edge_count: 0,
            diagnostics: vec![diagnostic],
        }
    }
}

impl NodeGeometryDiagnostic {
    fn from_rail_generation_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeRailGenerationError,
    ) -> Self {
        let kind = match error {
            NodeRailGenerationError::DegenerateConstraint { .. } => {
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index: 0,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::Degenerate,
                }
            }
            NodeRailGenerationError::DegenerateContour { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "degenerate_contour",
                }
            }
            NodeRailGenerationError::EmptyInput { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "empty_input",
                }
            }
            NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "noncanonical_generated_contact_endpoint",
                }
            }
            NodeRailGenerationError::TerminalCapGeneration { error } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: error.reason.diagnostic_reason(),
                }
            }
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::ContourGeneration,
            backend: NodeGeometryBackend::CavalierContours,
            kind,
        }
    }

    fn from_boolean_ownership_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeBooleanOwnershipError,
    ) -> Self {
        let kind = match error {
            NodeBooleanOwnershipError::UnownedAsphaltResidual {
                shape_count,
                area_m2,
            } => NodeGeometryDiagnosticKind::RejectedResidual {
                residual: NodeRejectedResidualKind::Asphalt,
                shape_count: *shape_count,
                area_m2: *area_m2,
            },
            NodeBooleanOwnershipError::UnownedBandResidual {
                kind,
                shape_count,
                area_m2,
            } => NodeGeometryDiagnosticKind::RejectedResidual {
                residual: NodeRejectedResidualKind::Band(*kind),
                shape_count: *shape_count,
                area_m2: *area_m2,
            },
            NodeBooleanOwnershipError::UnownedNonRoadResidual {
                shape_count,
                area_m2,
            } => NodeGeometryDiagnosticKind::RejectedResidual {
                residual: NodeRejectedResidualKind::NonRoad,
                shape_count: *shape_count,
                area_m2: *area_m2,
            },
            NodeBooleanOwnershipError::BooleanOperationFailed { stage } => {
                NodeGeometryDiagnosticKind::BackendFailure { reason: stage }
            }
            NodeBooleanOwnershipError::MissingBandOwner { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "missing_band_owner",
                }
            }
            NodeBooleanOwnershipError::NonCanonicalOwnedRegionVertex { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "noncanonical_owned_region_vertex",
                }
            }
            NodeBooleanOwnershipError::EmptyContourSet { .. }
            | NodeBooleanOwnershipError::EmptyFootprint { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "empty_boolean_input",
                }
            }
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::BooleanOwnership,
            backend: NodeGeometryBackend::IOverlay,
            kind,
        }
    }

    fn from_height_field_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeHeightFieldError,
    ) -> Self {
        let kind = match error {
            NodeHeightFieldError::InputOwnershipMismatch { .. } => {
                NodeGeometryDiagnosticKind::HeightFieldFailure {
                    reason: "input_ownership_mismatch",
                    mouth_order_index: None,
                    band_index: None,
                    kind: None,
                    source_kind: None,
                    height_field_id: None,
                    owner: None,
                    point_x_mm: None,
                    point_z_mm: None,
                    axis: None,
                    raw_parameter: None,
                }
            }
            NodeHeightFieldError::DuplicateSourceBand {
                mouth_order_index,
                band_index,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "duplicate_source_band",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: None,
                height_field_id: None,
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::MissingRegionBandIndex {
                mouth_order_index,
                kind,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "missing_region_band_index",
                mouth_order_index: Some(*mouth_order_index),
                band_index: None,
                kind: Some(*kind),
                source_kind: None,
                height_field_id: None,
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::MissingSourceBand {
                mouth_order_index,
                band_index,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "missing_source_band",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: None,
                height_field_id: None,
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::SourceBandKindMismatch {
                mouth_order_index,
                band_index,
                region_kind,
                source_kind,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "source_band_kind_mismatch",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: Some(*region_kind),
                source_kind: Some(*source_kind),
                height_field_id: None,
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::VertexOutsideHeightField {
                mouth_order_index,
                band_index,
                source_kind,
                height_field_id,
                owner,
                point_x_mm,
                point_z_mm,
                axis,
                raw_parameter,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "vertex_outside_height_field",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: Some(*source_kind),
                height_field_id: Some(*height_field_id),
                owner: *owner,
                point_x_mm: Some(*point_x_mm),
                point_z_mm: Some(*point_z_mm),
                axis: Some(*axis),
                raw_parameter: Some(*raw_parameter),
            },
            NodeHeightFieldError::TerminalCapGeneration { error } => {
                NodeGeometryDiagnosticKind::HeightFieldFailure {
                    reason: error.reason.diagnostic_reason(),
                    mouth_order_index: Some(error.mouth_order_index),
                    band_index: error.source_band_index,
                    kind: error.band_kind,
                    source_kind: error.band_kind,
                    height_field_id: None,
                    owner: None,
                    point_x_mm: None,
                    point_z_mm: None,
                    axis: None,
                    raw_parameter: None,
                }
            }
            NodeHeightFieldError::SourceHeightFieldConflict {
                mouth_order_index,
                band_index,
                source_kind,
                height_field_id,
                owner,
                existing_authority,
                incoming_authority,
                point_x_mm,
                point_z_mm,
                existing_height_mm,
                incoming_height_mm,
            } => NodeGeometryDiagnosticKind::SourceHeightFieldConflict {
                mouth_order_index: *mouth_order_index,
                band_index: *band_index,
                source_kind: *source_kind,
                height_field_id: *height_field_id,
                owner: *owner,
                existing_authority: *existing_authority,
                incoming_authority: *incoming_authority,
                x_mm: *point_x_mm,
                z_mm: *point_z_mm,
                existing_height_mm: *existing_height_mm,
                incoming_height_mm: *incoming_height_mm,
            },
            NodeHeightFieldError::SharedSourceHeightConflict {
                point_x_mm,
                point_z_mm,
                kind,
                owner,
                opposite_owner,
                height_field_id,
                incoming_owner,
                incoming_height_field_id,
                constraint_index,
                existing_authority,
                incoming_authority,
                existing_height_mm,
                incoming_height_mm,
            } => NodeGeometryDiagnosticKind::SharedSourceHeightConflict {
                x_mm: *point_x_mm,
                z_mm: *point_z_mm,
                kind: *kind,
                owner: *owner,
                opposite_owner: *opposite_owner,
                height_field_id: *height_field_id,
                incoming_owner: *incoming_owner,
                incoming_height_field_id: *incoming_height_field_id,
                constraint_index: *constraint_index,
                existing_authority: *existing_authority,
                incoming_authority: *incoming_authority,
                existing_height_mm: *existing_height_mm,
                incoming_height_mm: *incoming_height_mm,
            },
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::HeightEvaluation,
            backend: NodeGeometryBackend::HeightCarrier,
            kind,
        }
    }

    fn from_triangulation_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeTriangulationError,
    ) -> Self {
        let (backend, kind) = match error {
            NodeTriangulationError::InvalidConstraint { .. } => (
                NodeGeometryBackend::Spade,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index: 0,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::CdtRejected,
                },
            ),
            NodeTriangulationError::DuplicateVertexHeightConflict {
                x_mm,
                z_mm,
                existing_height_mm,
                incoming_height_mm,
                ..
            } => (
                NodeGeometryBackend::Spade,
                NodeGeometryDiagnosticKind::HeightConflict {
                    x_mm: *x_mm,
                    z_mm: *z_mm,
                    existing_height_mm: *existing_height_mm,
                    incoming_height_mm: *incoming_height_mm,
                },
            ),
            NodeTriangulationError::TriangleCoverageMismatch {
                region_index,
                missing_area_m2,
                extra_area_m2,
                ..
            } => (
                NodeGeometryBackend::IOverlay,
                NodeGeometryDiagnosticKind::TriangleCoverageMismatch {
                    region_index: *region_index,
                    missing_area_m2: *missing_area_m2,
                    extra_area_m2: *extra_area_m2,
                },
            ),
            NodeTriangulationError::BooleanOperationFailed { stage, .. } => (
                NodeGeometryBackend::IOverlay,
                NodeGeometryDiagnosticKind::BackendFailure { reason: stage },
            ),
            NodeTriangulationError::DegenerateRegionContour { region_index, .. } => (
                NodeGeometryBackend::Spade,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index: *region_index,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::Degenerate,
                },
            ),
            NodeTriangulationError::EmptyHeightSolution { .. }
            | NodeTriangulationError::EmptyRegionShape { .. }
            | NodeTriangulationError::CdtBuildFailed { .. }
            | NodeTriangulationError::EmptyTriangulation { .. } => (
                NodeGeometryBackend::Spade,
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "cdt_triangulation_failed",
                },
            ),
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::CdtTriangulation,
            backend,
            kind,
        }
    }

    fn from_arrangement_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeArrangementError,
    ) -> Self {
        let (backend, kind) = match error {
            NodeArrangementError::DuplicateVertexHeightConflict {
                key,
                existing_height_mm,
                incoming_height_mm,
            } => (
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::HeightConflict {
                    x_mm: key.x_mm(),
                    z_mm: key.z_mm(),
                    existing_height_mm: *existing_height_mm,
                    incoming_height_mm: *incoming_height_mm,
                },
            ),
            NodeArrangementError::EmptyOwnerSet { .. } => (
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "empty_arrangement_owner_set",
                },
            ),
            NodeArrangementError::DegenerateRegionContour { region_index, .. } => (
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index: *region_index,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::Degenerate,
                },
            ),
            NodeArrangementError::InputSolutionMismatch { .. }
            | NodeArrangementError::TriangulationRegionCountMismatch { .. }
            | NodeArrangementError::MissingHeightRegion { .. }
            | NodeArrangementError::RegionOwnerMismatch { .. }
            | NodeArrangementError::MissingTriangulatedVertex { .. } => (
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "arrangement_build_failed",
                },
            ),
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::Validation,
            backend,
            kind,
        }
    }

    fn from_arrangement_diagnostic(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        diagnostic: &NodeArrangementDiagnostic,
    ) -> Self {
        let kind = match diagnostic {
            NodeArrangementDiagnostic::MissingSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            } => NodeGeometryDiagnosticKind::SeamConstraintFailure {
                region_index: *region_index,
                owner: owner.kind(),
                owner_index: owner.owner_index(),
                opposite_owner: opposite_owner.kind(),
                opposite_owner_index: opposite_owner.owner_index(),
                start_x_mm: start.x_mm(),
                start_z_mm: start.z_mm(),
                end_x_mm: end.x_mm(),
                end_z_mm: end.z_mm(),
                reason: NodeSeamConstraintFailureReason::Missing,
            },
            NodeArrangementDiagnostic::AmbiguousSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            } => NodeGeometryDiagnosticKind::SeamConstraintFailure {
                region_index: *region_index,
                owner: owner.kind(),
                owner_index: owner.owner_index(),
                opposite_owner: opposite_owner.kind(),
                opposite_owner_index: opposite_owner.owner_index(),
                start_x_mm: start.x_mm(),
                start_z_mm: start.z_mm(),
                end_x_mm: end.x_mm(),
                end_z_mm: end.z_mm(),
                reason: NodeSeamConstraintFailureReason::Ambiguous,
            },
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::Validation,
            backend: NodeGeometryBackend::Parry2d,
            kind,
        }
    }

    fn from_owned_region_arrangement_diagnostic(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        diagnostic: &NodeOwnedRegionArrangementDiagnostic,
    ) -> Self {
        let (backend, kind) = match diagnostic {
            NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            } => (
                NodeGeometryBackend::IOverlay,
                NodeGeometryDiagnosticKind::SeamConstraintFailure {
                    region_index: *region_index,
                    owner: owner.kind(),
                    owner_index: owner.owner_index(),
                    opposite_owner: opposite_owner.kind(),
                    opposite_owner_index: opposite_owner.owner_index(),
                    start_x_mm: start.x_mm(),
                    start_z_mm: start.z_mm(),
                    end_x_mm: end.x_mm(),
                    end_z_mm: end.z_mm(),
                    reason: NodeSeamConstraintFailureReason::Missing,
                },
            ),
            NodeOwnedRegionArrangementDiagnostic::UnmaterializedRaisedStepAuthority {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
                source_constraint_indices,
            } => (
                NodeGeometryBackend::CanonicalKeys,
                NodeGeometryDiagnosticKind::UnmaterializedRaisedStepAuthority {
                    region_index: *region_index,
                    owner: owner.kind(),
                    owner_index: owner.owner_index(),
                    opposite_owner: opposite_owner.kind(),
                    opposite_owner_index: opposite_owner.owner_index(),
                    start_x_mm: start.x_mm(),
                    start_z_mm: start.z_mm(),
                    end_x_mm: end.x_mm(),
                    end_z_mm: end.z_mm(),
                    source_constraint_indices: source_constraint_indices.clone(),
                },
            ),
            NodeOwnedRegionArrangementDiagnostic::AmbiguousSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            } => (
                NodeGeometryBackend::IOverlay,
                NodeGeometryDiagnosticKind::SeamConstraintFailure {
                    region_index: *region_index,
                    owner: owner.kind(),
                    owner_index: owner.owner_index(),
                    opposite_owner: opposite_owner.kind(),
                    opposite_owner_index: opposite_owner.owner_index(),
                    start_x_mm: start.x_mm(),
                    start_z_mm: start.z_mm(),
                    end_x_mm: end.x_mm(),
                    end_z_mm: end.z_mm(),
                    reason: NodeSeamConstraintFailureReason::Ambiguous,
                },
            ),
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::BooleanOwnership,
            backend,
            kind,
        }
    }

    fn debug_record(&self) -> String {
        format!(
            "{{\"node_id\":{},\"piece_kind\":\"{:?}\",\"stage\":\"{}\",\"backend\":\"{}\",\"kind\":\"{}\",\"detail\":\"{:?}\"}}",
            self.node_id,
            self.piece_kind,
            self.stage.as_str(),
            self.backend.as_str(),
            self.kind.as_str(),
            self.kind
        )
    }
}

impl NodeGeometryStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::ContourGeneration => "contour_generation",
            Self::BooleanOwnership => "boolean_ownership",
            Self::HeightEvaluation => "height_evaluation",
            Self::Validation => "validation",
            Self::CdtTriangulation => "cdt_triangulation",
        }
    }
}

impl NodeGeometryBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::CavalierContours => "cavalier_contours",
            Self::IOverlay => "i_overlay",
            Self::HeightCarrier => "height_carrier",
            Self::CanonicalKeys => "canonical_keys",
            Self::Parry2d => "parry2d",
            Self::Spade => "spade",
        }
    }
}

impl NodeGeometryDiagnosticKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::RejectedResidual { .. } => "rejected_residual",
            Self::NonExplicitBoundaryVertex { .. } => "non_explicit_boundary_vertex",
            Self::HeightConflict { .. } | Self::CrossRegionHeightConflict { .. } => {
                "height_conflict"
            }
            Self::SourceHeightFieldConflict { .. } => "source_height_field_conflict",
            Self::SharedSourceHeightConflict { .. } => "shared_source_height_conflict",
            Self::HeightFieldFailure { .. } => "height_field_failure",
            Self::OpenBoundary { .. } => "open_boundary",
            Self::DuplicateExposedEdge { .. } => "duplicate_exposed_edge",
            Self::InvalidConstraint { .. } => "invalid_constraint",
            Self::TriangleCoverageMismatch { .. } => "triangle_coverage_mismatch",
            Self::TriangleOverlap { .. } => "triangle_overlap",
            Self::SeamConstraintFailure { .. } => "seam_constraint_failure",
            Self::UnmaterializedRaisedStepAuthority { .. } => {
                "unmaterialized_raised_step_authority"
            }
            Self::BackendFailure { .. } => "backend_failure",
        }
    }
}

fn duplicate_exposed_edge_has_explicit_owner_context(
    solution: &NodeTriangulationSolution,
    region_indices: &[usize],
) -> bool {
    let mut owners = BTreeSet::new();
    for region_index in region_indices {
        let Some(region) = solution.regions.get(*region_index) else {
            return false;
        };
        owners.insert(region.owner);
    }
    let owners = owners.into_iter().collect::<Vec<_>>();
    if owners.is_empty() {
        return false;
    }
    for (left_index, left) in owners.iter().copied().enumerate() {
        for right in owners.iter().copied().skip(left_index + 1) {
            if left.kind() == right.kind() || owners_form_explicit_vertical_step_pair(left, right) {
                continue;
            }
            return false;
        }
    }
    true
}

fn duplicate_exposed_edge_is_canonical_drift(
    solution: &NodeTriangulationSolution,
    edge: NodeValidationEdgeKey,
    region_indices: &[usize],
) -> bool {
    if validation_edge_length_m(edge) > VALIDATION_DUPLICATE_EXPOSED_EDGE_CANONICAL_DRIFT_M {
        return false;
    }

    let mut start_heights = BTreeSet::new();
    let mut end_heights = BTreeSet::new();
    for region_index in region_indices {
        let Some(region) = solution.regions.get(*region_index) else {
            return false;
        };
        let Some(start_height_mm) = region_height_mm_at_key(region, edge.start) else {
            return false;
        };
        let Some(end_height_mm) = region_height_mm_at_key(region, edge.end) else {
            return false;
        };
        start_heights.insert(start_height_mm);
        end_heights.insert(end_height_mm);
    }

    start_heights.len() == 1 && end_heights.len() == 1
}

fn validation_edge_length_m(edge: NodeValidationEdgeKey) -> f64 {
    let dx = (edge.end.x_key - edge.start.x_key) as f64 / VALIDATION_POINT_KEY_SCALE;
    let dz = (edge.end.z_key - edge.start.z_key) as f64 / VALIDATION_POINT_KEY_SCALE;
    dx.hypot(dz)
}

fn region_height_mm_at_key(
    region: &NodeTriangulatedRegion,
    point: NodeValidationPointKey,
) -> Option<i64> {
    region.vertices.iter().find_map(|vertex| {
        (point_key_from_world(vertex.point_world) == point)
            .then(|| quantize_m(vertex.point_world.y))
    })
}

fn validate_region(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) -> Vec<NodeValidationEdgeKey> {
    let boundary_segments =
        validate_boundary_constraints(solution, region_index, region, diagnostics);
    validate_constraint_crossings(solution, region_index, &boundary_segments, diagnostics);
    let exposed_edges = validate_triangles(
        solution,
        region_index,
        region,
        &boundary_segments,
        diagnostics,
    );
    validate_triangle_area_coverage(solution, region_index, region, diagnostics);
    exposed_edges
}

fn validate_boundary_constraints(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) -> Vec<BoundarySegment> {
    let mut seen_constraints = BTreeSet::new();
    let mut boundary_degree = BTreeMap::<NodeValidationPointKey, usize>::new();
    let mut boundary_segments = Vec::with_capacity(region.boundary_constraints.len());

    for (constraint_index, constraint) in region.boundary_constraints.iter().copied().enumerate() {
        if constraint[0] >= region.vertices.len() || constraint[1] >= region.vertices.len() {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: Some(constraint_index),
                    reason: NodeInvalidConstraintReason::OutOfRange,
                },
            );
            continue;
        }
        if constraint[0] == constraint[1] {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: Some(constraint_index),
                    reason: NodeInvalidConstraintReason::Degenerate,
                },
            );
            continue;
        }
        let normalized = normalized_constraint(constraint[0], constraint[1]);
        let key_edge = edge_key_for_indices(region, normalized);
        if key_edge.is_degenerate() {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::CanonicalKeys,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: Some(constraint_index),
                    reason: NodeInvalidConstraintReason::Degenerate,
                },
            );
            continue;
        }
        if !seen_constraints.insert(key_edge) {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::CanonicalKeys,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: Some(constraint_index),
                    reason: NodeInvalidConstraintReason::Duplicate,
                },
            );
            continue;
        }

        // Constraint identity is the canonical vertex pair, not the f32 Parry segment length.
        // Overlay-grid-distinct endpoint connectors can collapse after the f32 conversion.
        let segment = parry_segment_for_edge(region, normalized);
        *boundary_degree.entry(key_edge.start).or_default() += 1;
        *boundary_degree.entry(key_edge.end).or_default() += 1;
        boundary_segments.push(BoundarySegment {
            index: constraint_index,
            edge: normalized,
            key_edge,
            segment,
        });
    }

    for (_point_key, degree) in boundary_degree {
        if degree != 2 {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::CanonicalKeys,
                NodeGeometryDiagnosticKind::OpenBoundary {
                    region_index,
                    vertex_index: None,
                    degree,
                },
            );
        }
    }
    boundary_segments
}

fn validate_constraint_crossings(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    boundary_segments: &[BoundarySegment],
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    for first_index in 0..boundary_segments.len() {
        for second_index in first_index + 1..boundary_segments.len() {
            let first = boundary_segments[first_index];
            let second = boundary_segments[second_index];
            if shares_endpoint(first.edge, second.edge) {
                continue;
            }
            if key_edges_share_endpoint(first.key_edge, second.key_edge) {
                continue;
            }
            if canonical_key_segments_strictly_intersect(first.key_edge, second.key_edge) {
                let region = &solution.regions[region_index];
                crate::debug_log!(
                    "road",
                    "node_constraint_crossing node_id={} piece_kind={:?} region={} kind={:?} owner={:?} backend=canonical_keys first_constraint={} second_constraint={} first_key=({},{})->({},{}) second_key=({},{})->({},{}) first=({:.6},{:.6})->({:.6},{:.6}) second=({:.6},{:.6})->({:.6},{:.6})",
                    solution.node_id,
                    solution.piece_kind,
                    region_index,
                    region.kind,
                    region.owner,
                    first.index,
                    second.index,
                    first.key_edge.start.x_key,
                    first.key_edge.start.z_key,
                    first.key_edge.end.x_key,
                    first.key_edge.end.z_key,
                    second.key_edge.start.x_key,
                    second.key_edge.start.z_key,
                    second.key_edge.end.x_key,
                    second.key_edge.end.z_key,
                    first.segment.a.x,
                    first.segment.a.y,
                    first.segment.b.x,
                    first.segment.b.y,
                    second.segment.a.x,
                    second.segment.a.y,
                    second.segment.b.x,
                    second.segment.b.y
                );
                push_validation_diagnostic(
                    solution,
                    diagnostics,
                    NodeGeometryBackend::CanonicalKeys,
                    NodeGeometryDiagnosticKind::InvalidConstraint {
                        region_index,
                        constraint_index: Some(first.index.min(second.index)),
                        reason: NodeInvalidConstraintReason::Crossing,
                    },
                );
            }
        }
    }
}

fn key_edges_share_endpoint(a: NodeValidationEdgeKey, b: NodeValidationEdgeKey) -> bool {
    a.start == b.start || a.start == b.end || a.end == b.start || a.end == b.end
}

fn canonical_key_segments_strictly_intersect(
    first: NodeValidationEdgeKey,
    second: NodeValidationEdgeKey,
) -> bool {
    let [a, b] = first.endpoints();
    let [c, d] = second.endpoints();
    if key_edges_share_endpoint(first, second) {
        return false;
    }

    let ab_c = orientation(a, b, c);
    let ab_d = orientation(a, b, d);
    let cd_a = orientation(c, d, a);
    let cd_b = orientation(c, d, b);

    if ab_c == 0 && ab_d == 0 && cd_a == 0 && cd_b == 0 {
        return collinear_segments_overlap_with_positive_length(a, b, c, d);
    }

    if ab_c == 0 || ab_d == 0 || cd_a == 0 || cd_b == 0 {
        return false;
    }

    signs_differ(ab_c, ab_d) && signs_differ(cd_a, cd_b)
}

fn orientation(
    a: NodeValidationPointKey,
    b: NodeValidationPointKey,
    c: NodeValidationPointKey,
) -> i128 {
    let ab_x = i128::from(b.x_key) - i128::from(a.x_key);
    let ab_z = i128::from(b.z_key) - i128::from(a.z_key);
    let ac_x = i128::from(c.x_key) - i128::from(a.x_key);
    let ac_z = i128::from(c.z_key) - i128::from(a.z_key);
    ab_x * ac_z - ab_z * ac_x
}

fn signs_differ(a: i128, b: i128) -> bool {
    (a < 0 && b > 0) || (a > 0 && b < 0)
}

fn collinear_segments_overlap_with_positive_length(
    a: NodeValidationPointKey,
    b: NodeValidationPointKey,
    c: NodeValidationPointKey,
    d: NodeValidationPointKey,
) -> bool {
    if a.x_key != b.x_key || c.x_key != d.x_key {
        intervals_overlap_with_positive_length(a.x_key, b.x_key, c.x_key, d.x_key)
    } else {
        intervals_overlap_with_positive_length(a.z_key, b.z_key, c.z_key, d.z_key)
    }
}

fn intervals_overlap_with_positive_length(a0: i64, a1: i64, b0: i64, b1: i64) -> bool {
    let a_min = a0.min(a1);
    let a_max = a0.max(a1);
    let b_min = b0.min(b1);
    let b_max = b0.max(b1);
    a_min.max(b_min) < a_max.min(b_max)
}

fn validate_triangles(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    boundary_segments: &[BoundarySegment],
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) -> Vec<NodeValidationEdgeKey> {
    let boundary_edges = boundary_segments
        .iter()
        .map(|segment| segment.edge)
        .collect::<BTreeSet<_>>();
    let mut triangle_edge_counts = BTreeMap::<[usize; 2], usize>::new();
    for triangle in &region.triangles {
        if !triangle_indices_valid(triangle, region.vertices.len()) {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::OutOfRange,
                },
            );
            continue;
        }
        for edge in triangle_edges(triangle) {
            *triangle_edge_counts.entry(edge).or_default() += 1;
        }
    }

    let mut exposed_edges = Vec::new();
    for (edge, count) in triangle_edge_counts {
        if count > 2 {
            let edge_key = edge_key_for_indices(region, edge);
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::DuplicateExposedEdge {
                    region_index: Some(region_index),
                    start_x_mm: edge_key.start.x_mm(),
                    start_z_mm: edge_key.start.z_mm(),
                    end_x_mm: edge_key.end.x_mm(),
                    end_z_mm: edge_key.end.z_mm(),
                    count,
                },
            );
            continue;
        }
        if count != 1 {
            continue;
        }
        let edge_key = edge_key_for_indices(region, edge);
        exposed_edges.push(edge_key);
        if boundary_edges.contains(&edge)
            || edge_lies_on_boundary_constraint(region, edge, boundary_segments)
        {
            continue;
        }
        let start_distance_mm =
            min_distance_to_boundary_mm(region.vertices[edge[0]].point_world, boundary_segments);
        let end_distance_mm =
            min_distance_to_boundary_mm(region.vertices[edge[1]].point_world, boundary_segments);
        for (vertex_index, distance_mm) in
            [(edge[0], start_distance_mm), (edge[1], end_distance_mm)]
        {
            if distance_mm > quantize_m(f64::from(VALIDATION_MIN_SEGMENT_LENGTH_M)) {
                let key = point_key_from_world(region.vertices[vertex_index].point_world);
                push_validation_diagnostic(
                    solution,
                    diagnostics,
                    NodeGeometryBackend::Parry2d,
                    NodeGeometryDiagnosticKind::NonExplicitBoundaryVertex {
                        region_index,
                        x_mm: key.x_mm(),
                        z_mm: key.z_mm(),
                        min_boundary_distance_mm: distance_mm,
                    },
                );
            }
        }
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::Parry2d,
            NodeGeometryDiagnosticKind::OpenBoundary {
                region_index,
                vertex_index: None,
                degree: 1,
            },
        );
    }
    exposed_edges
}

fn validate_triangle_area_coverage(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    if region.triangles.is_empty() {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::Spade,
            NodeGeometryDiagnosticKind::BackendFailure {
                reason: "empty_triangle_set",
            },
        );
        return;
    }
    let triangle_contours = region
        .triangles
        .iter()
        .filter(|triangle| triangle_indices_valid(triangle, region.vertices.len()))
        .map(|triangle| triangle_contour(region, triangle))
        .collect::<Vec<_>>();
    let Some(triangle_shapes) = RoadSurfaceSystem::overlay_union_contours(&triangle_contours)
    else {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::IOverlay,
            NodeGeometryDiagnosticKind::BackendFailure {
                reason: "triangle_union_failed",
            },
        );
        return;
    };
    let union_area = triangle_shapes
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum::<f32>();
    let triangle_area_sum = region
        .triangles
        .iter()
        .filter(|triangle| triangle_indices_valid(triangle, region.vertices.len()))
        .map(|triangle| triangle_area_m2(region, triangle))
        .sum::<f32>();
    let overlap_area_m2 = (triangle_area_sum - union_area).max(0.0);
    let area_budget_m2 =
        RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&triangle_shapes);
    if overlap_area_m2 > area_budget_m2 {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::IOverlay,
            NodeGeometryDiagnosticKind::TriangleOverlap {
                region_index,
                overlap_area_m2,
            },
        );
    }

    let area_delta = union_area - region.area_m2;
    if area_delta.abs() > area_budget_m2 {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::IOverlay,
            NodeGeometryDiagnosticKind::TriangleCoverageMismatch {
                region_index,
                missing_area_m2: (-area_delta).max(0.0),
                extra_area_m2: area_delta.max(0.0),
            },
        );
    }
}

fn push_validation_diagnostic(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
    backend: NodeGeometryBackend,
    kind: NodeGeometryDiagnosticKind,
) {
    diagnostics.push(NodeGeometryDiagnostic {
        node_id: solution.node_id,
        piece_kind: solution.piece_kind,
        stage: NodeGeometryStage::Validation,
        backend,
        kind,
    });
}

fn parry_segment_for_edge(region: &NodeTriangulatedRegion, edge: [usize; 2]) -> Segment {
    Segment::new(
        parry_point_from_vertex(&region.vertices[edge[0]]),
        parry_point_from_vertex(&region.vertices[edge[1]]),
    )
}

fn parry_point_from_vertex(vertex: &NodeTriangulatedVertex) -> Vector {
    Vector::new(vertex.point_world.x as f32, vertex.point_world.z as f32)
}

fn min_distance_to_boundary_mm(
    point: super::backend::RoadVec3,
    boundary_segments: &[BoundarySegment],
) -> i64 {
    let point = Vector::new(point.x as f32, point.z as f32);
    boundary_segments
        .iter()
        .map(|segment| {
            segment
                .segment
                .distance_to_point(&Pose::identity(), point, false)
        })
        .min_by(|a, b| a.total_cmp(b))
        .map(|distance| quantize_m(f64::from(distance)))
        .unwrap_or(i64::MAX)
}

fn edge_lies_on_boundary_constraint(
    region: &NodeTriangulatedRegion,
    edge: [usize; 2],
    boundary_segments: &[BoundarySegment],
) -> bool {
    let edge_segment = parry_segment_for_edge(region, edge);
    [edge_segment.a, edge_segment.b]
        .into_iter()
        .all(|point| point_lies_on_boundary_constraint(point, boundary_segments))
}

fn point_lies_on_boundary_constraint(point: Vector, boundary_segments: &[BoundarySegment]) -> bool {
    boundary_segments.iter().any(|boundary| {
        boundary
            .segment
            .distance_to_point(&Pose::identity(), point, false)
            <= VALIDATION_MIN_SEGMENT_LENGTH_M
    })
}

fn triangle_edges(triangle: &NodeTriangulatedTriangle) -> [[usize; 2]; 3] {
    [
        normalized_constraint(triangle.vertices[0], triangle.vertices[1]),
        normalized_constraint(triangle.vertices[1], triangle.vertices[2]),
        normalized_constraint(triangle.vertices[2], triangle.vertices[0]),
    ]
}

fn triangle_indices_valid(triangle: &NodeTriangulatedTriangle, vertex_count: usize) -> bool {
    triangle.vertices.iter().all(|index| *index < vertex_count)
        && triangle.vertices[0] != triangle.vertices[1]
        && triangle.vertices[1] != triangle.vertices[2]
        && triangle.vertices[2] != triangle.vertices[0]
}

fn triangle_contour(
    region: &NodeTriangulatedRegion,
    triangle: &NodeTriangulatedTriangle,
) -> NodeOverlayContour {
    let mut contour = triangle
        .vertices
        .iter()
        .map(|index| {
            let point = region.vertices[*index].point_world;
            [point.x, point.z]
        })
        .collect::<Vec<_>>();
    if signed_overlay_area_m2(&contour) < 0.0 {
        contour.swap(1, 2);
    }
    contour
}

fn triangle_area_m2(region: &NodeTriangulatedRegion, triangle: &NodeTriangulatedTriangle) -> f32 {
    signed_overlay_area_m2(&triangle_contour(region, triangle)).abs()
}

fn signed_overlay_area_m2(contour: &NodeOverlayContour) -> f32 {
    if contour.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for index in 0..contour.len() {
        let start = contour[index];
        let end = contour[(index + 1) % contour.len()];
        area += start[0] * end[1] - end[0] * start[1];
    }
    (area * 0.5) as f32
}

fn edge_key_for_indices(
    region: &NodeTriangulatedRegion,
    edge: [usize; 2],
) -> NodeValidationEdgeKey {
    NodeValidationEdgeKey::new(
        point_key_from_world(region.vertices[edge[0]].point_world),
        point_key_from_world(region.vertices[edge[1]].point_world),
    )
}

fn point_key_from_world(point: super::backend::RoadVec3) -> NodeValidationPointKey {
    NodeValidationPointKey {
        x_key: quantize_point(point.x),
        z_key: quantize_point(point.z),
    }
}

fn normalized_constraint(a: usize, b: usize) -> [usize; 2] {
    if a < b { [a, b] } else { [b, a] }
}

fn shares_endpoint(a: [usize; 2], b: [usize; 2]) -> bool {
    a[0] == b[0] || a[0] == b[1] || a[1] == b[0] || a[1] == b[1]
}

fn quantize_m(value: f64) -> i64 {
    (value * VALIDATION_KEY_SCALE).round() as i64
}

fn quantize_point(value: f64) -> i64 {
    (value * VALIDATION_POINT_KEY_SCALE).round() as i64
}

fn validation_point_key_to_mm(value: i64) -> i64 {
    ((value as f64 / VALIDATION_POINT_KEY_SCALE) * VALIDATION_KEY_SCALE).round() as i64
}

impl NodeValidationPointKey {
    fn from_arrangement_key(key: NodeArrangementKey) -> Self {
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }

    fn x_mm(self) -> i64 {
        validation_point_key_to_mm(self.x_key)
    }

    fn z_mm(self) -> i64 {
        validation_point_key_to_mm(self.z_key)
    }
}

impl NodeValidationEdgeKey {
    fn new(a: NodeValidationPointKey, b: NodeValidationPointKey) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }

    fn endpoints(self) -> [NodeValidationPointKey; 2] {
        [self.start, self.end]
    }

    fn is_degenerate(self) -> bool {
        self.start == self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::arrangement::{
        NodeArrangement, NodeArrangementDiagnostic, NodeArrangementKey, NodeBandHeightFieldId,
        NodeBandOwner, NodeExplicitVerticalStepSegment,
    };
    use crate::simulation::network::surface::backend::{RoadVec2, RoadVec3};
    use crate::simulation::network::surface::height::NodeHeightSolution;
    use crate::simulation::network::surface::input::NodeArrangementInput;
    use crate::simulation::network::surface::ownership::{
        NodeBooleanOwnership, NodeOwnedRegionArrangementDiagnostic, NodeOwnedRegionArrangementKey,
    };
    use crate::simulation::network::surface::rails::{
        NodeGeneratedContourClaimPriority, NodeGeneratedContourPurpose, NodeRailContourSet,
    };
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
        let rails =
            NodeRailContourSet::from_input(&input).expect("test input should produce rails");
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
        NodeTriangulatedRegion {
            kind,
            owner: NodeBandOwner::new(kind, owner_index),
            height_field_id,
            vertices: vertices
                .into_iter()
                .map(|point_world| NodeTriangulatedVertex {
                    point_world,
                    height_field_id,
                })
                .collect(),
            boundary_constraints: vec![[0, 1], [1, 2], [0, 2]],
            triangles: vec![NodeTriangulatedTriangle {
                vertices: [0, 1, 2],
            }],
            area_m2: 0.5,
        }
    }

    fn key_point(x: f64, z: f64) -> NodeValidationPointKey {
        NodeValidationPointKey {
            x_key: quantize_point(x),
            z_key: quantize_point(z),
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
        let mouth_asphalt_field =
            NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
        let joined_asphalt_field =
            NodeBandHeightFieldId::new(1, 0, RoadSurfaceBandKind::Carriageway);
        let curb_field = NodeBandHeightFieldId::new(0, 1, RoadSurfaceBandKind::CurbOrShoulder);
        let start = NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0));
        let end = NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0));
        let asphalt_handoff = NodeExplicitVerticalStepSegment::new(
            start,
            end,
            mouth_asphalt_owner,
            joined_asphalt_owner,
        )
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

        NodeValidationReport::from_triangulation_solution(&solution).expect(
            "same-height asphalt owner handoff should carry the explicit curb step authority",
        );
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
}
