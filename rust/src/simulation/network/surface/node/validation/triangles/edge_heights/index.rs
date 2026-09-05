// SPDX-License-Identifier: GPL-2.0-only

//! Heighted triangle-edge indexing and owner coverage checks.

use super::*;

impl ValidationTriangleEdgeIndex {
    pub(super) fn from_solution(solution: &NodeTriangulationSolution) -> Self {
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

    pub(super) fn owner_covers_edge_with_matching_heights(
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

fn heighted_triangle_edge_for_indices(
    region_index: usize,
    region: &NodeTriangulatedRegion,
    edge: [usize; 2],
) -> (NodeValidationEdgeKey, HeightedTriangleEdge) {
    let start = region.vertices[edge[0]].point_world;
    let end = region.vertices[edge[1]].point_world;
    let start_source_provenance = region.vertices[edge[0]].grade_authority.source_provenance;
    let end_source_provenance = region.vertices[edge[1]].grade_authority.source_provenance;
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
                start_source_provenance,
                end_source_provenance,
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
                start_source_provenance: end_source_provenance,
                end_source_provenance: start_source_provenance,
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
