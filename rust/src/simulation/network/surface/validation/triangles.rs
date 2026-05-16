//! Triangle validation and cross-region height checks.

use super::super::arrangement::{
    NodeBandOwner, NodeExplicitVerticalStepSegment, owners_form_explicit_vertical_step_pair,
};
use super::super::triangulation::{
    NodeTriangulatedRegion, NodeTriangulatedTriangle, NodeTriangulationSolution,
};
use super::super::{NodeOverlayContour, RoadSurfaceSystem};
use super::boundaries::{
    duplicate_exposed_edge_has_explicit_owner_context, duplicate_exposed_edge_is_canonical_drift,
    edge_lies_on_boundary_constraint, min_distance_to_boundary_mm, validate_region,
};
use super::report::{
    NodeExplicitStepSegmentDiagnostic, NodeGeometryBackend, NodeGeometryDiagnostic,
    NodeGeometryDiagnosticKind, NodeGeometryStage, NodeInvalidConstraintReason,
    NodeValidationError, NodeValidationReport, push_validation_diagnostic,
};
use super::{
    BoundarySegment, NodeValidationEdgeKey, NodeValidationPointKey,
    VALIDATION_MIN_SEGMENT_LENGTH_M, edge_key_for_indices, normalized_constraint,
    point_key_from_world, quantize_m,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightedTriangleEdge {
    region_index: usize,
    start_height_mm: i64,
    end_height_mm: i64,
}

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn validate_node_triangulation_solution(
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
    left_owner: NodeBandOwner,
    left: HeightedTriangleEdge,
    right_owner: NodeBandOwner,
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
    missing_owner: NodeBandOwner,
    missing_edge: HeightedTriangleEdge,
    direct_owner: NodeBandOwner,
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
    direct_owner: NodeBandOwner,
) -> Option<NodeBandOwner> {
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
    owner: NodeBandOwner,
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
    owner: NodeBandOwner,
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
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
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
    point
        .surface_key()
        .lies_on_open_segment(start.surface_key(), end.surface_key())
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
    existing_owner: NodeBandOwner,
    incoming_owner: NodeBandOwner,
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
}

pub(super) fn validate_triangles(
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

pub(super) fn validate_triangle_area_coverage(
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
    if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
        contour.swap(1, 2);
    }
    contour
}

fn triangle_area_m2(region: &NodeTriangulatedRegion, triangle: &NodeTriangulatedTriangle) -> f32 {
    let points = triangle
        .vertices
        .map(|index| region.vertices[index].point_world);
    (RoadSurfaceSystem::road_triangle_double_area_xz_m2(points) * 0.5) as f32
}
