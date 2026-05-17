//! Cross-region triangle edge height validation.

use super::super::super::arrangement::{
    NodeBandOwner, NodeExplicitVerticalStepSegment, owners_form_explicit_vertical_step_pair,
};
use super::super::super::keys::SurfaceSegmentParameter;
use super::super::super::segments;
use super::super::super::triangulation::{NodeTriangulatedRegion, NodeTriangulationSolution};
use super::super::report::{
    NodeExplicitStepSegmentDiagnostic, NodeGeometryBackend, NodeGeometryDiagnostic,
    NodeGeometryDiagnosticKind, push_validation_diagnostic,
};
use super::super::{
    NodeValidationEdgeKey, NodeValidationPointKey, point_key_from_world, quantize_m,
};
use super::{edge_indices_valid, triangle_edges, triangle_indices_valid};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightedTriangleEdge {
    region_index: usize,
    start_height_mm: i64,
    end_height_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeightedOwnedCoverageEdge {
    edge: NodeValidationEdgeKey,
    heighted_edge: HeightedTriangleEdge,
}

#[derive(Default)]
struct ValidationTriangleEdgeIndex {
    by_edge: BTreeMap<NodeValidationEdgeKey, Vec<HeightedTriangleEdge>>,
    by_owner_coverage: BTreeMap<NodeBandOwner, Vec<HeightedOwnedCoverageEdge>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightedEdgeCoverageInterval {
    start: SurfaceSegmentParameter,
    end: SurfaceSegmentParameter,
}

impl ValidationTriangleEdgeIndex {
    fn from_solution(solution: &NodeTriangulationSolution) -> Self {
        let mut index = Self::default();
        for (region_index, region) in solution.regions.iter().enumerate() {
            for triangle in &region.triangles {
                if !triangle_indices_valid(triangle, region.vertices.len()) {
                    continue;
                }
                for edge in triangle_edges(triangle) {
                    let (edge_key, heighted_edge) =
                        heighted_triangle_edge_for_indices(region_index, region, edge);
                    index
                        .by_edge
                        .entry(edge_key)
                        .or_default()
                        .push(heighted_edge);
                    index
                        .by_owner_coverage
                        .entry(region.owner)
                        .or_default()
                        .push(HeightedOwnedCoverageEdge {
                            edge: edge_key,
                            heighted_edge,
                        });
                }
            }
            for edge in &region.boundary_constraints {
                if !edge_indices_valid(*edge, region.vertices.len()) {
                    continue;
                }
                let (edge_key, heighted_edge) =
                    heighted_triangle_edge_for_indices(region_index, region, *edge);
                index
                    .by_owner_coverage
                    .entry(region.owner)
                    .or_default()
                    .push(HeightedOwnedCoverageEdge {
                        edge: edge_key,
                        heighted_edge,
                    });
            }
        }
        index
    }

    fn owner_covers_edge_with_matching_heights(
        &self,
        owner: NodeBandOwner,
        edge: NodeValidationEdgeKey,
        target: HeightedTriangleEdge,
    ) -> bool {
        let Some(candidates) = self.by_owner_coverage.get(&owner) else {
            return false;
        };
        let mut intervals = candidates
            .iter()
            .filter_map(|candidate| matching_coverage_interval(edge, target, *candidate))
            .collect::<Vec<_>>();
        if intervals.is_empty() {
            return false;
        }
        intervals.sort_unstable();

        let mut covered = SurfaceSegmentParameter::zero();
        let end = SurfaceSegmentParameter::one();
        for interval in intervals {
            if interval.start > covered {
                return false;
            }
            covered = covered.max(interval.end);
            if covered >= end {
                return true;
            }
        }
        false
    }
}

pub(super) fn validate_cross_region_triangle_edge_heights(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    let edge_index = ValidationTriangleEdgeIndex::from_solution(solution);

    for (edge_key, heighted_edges) in &edge_index.by_edge {
        let mut heighted_edges = heighted_edges.clone();
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
                        solution,
                        &edge_index,
                        *edge_key,
                        left,
                        right,
                    )
                {
                    continue;
                }
                push_triangle_edge_height_conflict(solution, diagnostics, *edge_key, left, right);
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

fn matching_coverage_interval(
    edge: NodeValidationEdgeKey,
    target: HeightedTriangleEdge,
    candidate: HeightedOwnedCoverageEdge,
) -> Option<HeightedEdgeCoverageInterval> {
    let edge_start = edge.start.surface_key();
    let edge_end = edge.end.surface_key();
    let candidate_start = candidate.edge.start.surface_key();
    let candidate_end = candidate.edge.end.surface_key();

    let candidate_start_parameter =
        segments::exact_line_parameter(candidate_start, edge_start, edge_end)?;
    let candidate_end_parameter =
        segments::exact_line_parameter(candidate_end, edge_start, edge_end)?;
    let (candidate_interval_start, candidate_interval_end) =
        if candidate_start_parameter <= candidate_end_parameter {
            (candidate_start_parameter, candidate_end_parameter)
        } else {
            (candidate_end_parameter, candidate_start_parameter)
        };
    let start = candidate_interval_start.max(SurfaceSegmentParameter::zero());
    let end = candidate_interval_end.min(SurfaceSegmentParameter::one());
    if start >= end {
        return None;
    }

    let start_height_mm =
        heighted_candidate_edge_height_at_target_parameter(edge, candidate, start)?;
    let end_height_mm = heighted_candidate_edge_height_at_target_parameter(edge, candidate, end)?;
    let expected_start_height_mm =
        segments::interpolate_height_i64(target.start_height_mm, target.end_height_mm, start);
    let expected_end_height_mm =
        segments::interpolate_height_i64(target.start_height_mm, target.end_height_mm, end);
    (start_height_mm == expected_start_height_mm && end_height_mm == expected_end_height_mm)
        .then_some(HeightedEdgeCoverageInterval { start, end })
}

fn heighted_candidate_edge_height_at_target_parameter(
    edge: NodeValidationEdgeKey,
    candidate: HeightedOwnedCoverageEdge,
    parameter: SurfaceSegmentParameter,
) -> Option<i64> {
    let target_start = edge.start.surface_key();
    let target_end = edge.end.surface_key();
    let point = segments::interpolate_key(target_start, target_end, parameter);
    let candidate_start = candidate.edge.start.surface_key();
    let candidate_end = candidate.edge.end.surface_key();
    if !segments::key_lies_exactly_on_segment(point, candidate_start, candidate_end) {
        return None;
    }
    let candidate_parameter =
        segments::exact_line_parameter(point, candidate_start, candidate_end)?;
    Some(segments::interpolate_height_i64(
        candidate.heighted_edge.start_height_mm,
        candidate.heighted_edge.end_height_mm,
        candidate_parameter,
    ))
}

fn cross_region_edges_form_explicit_vertical_step(
    solution: &NodeTriangulationSolution,
    edge_index: &ValidationTriangleEdgeIndex,
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
        edge_index,
        edge,
        left_region.owner,
        left,
        right_region.owner,
        right,
    )
}

fn cross_region_edges_form_same_height_owner_handoff_explicit_vertical_step(
    solution: &NodeTriangulationSolution,
    edge_index: &ValidationTriangleEdgeIndex,
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
                edge_index,
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
                edge_index,
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
    edge_index: &ValidationTriangleEdgeIndex,
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
    edge_index.owner_covers_edge_with_matching_heights(bridge_owner, edge, missing_edge)
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
    point
        .surface_key()
        .lies_exactly_on_segment(start.surface_key(), end.surface_key())
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
