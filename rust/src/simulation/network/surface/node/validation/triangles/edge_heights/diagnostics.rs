// SPDX-License-Identifier: GPL-2.0-only

//! Cross-region edge-height diagnostics.

use super::steps::{
    edge_lies_on_explicit_vertical_step, explicit_vertical_step_owners_match_regions,
};
use super::*;

pub(super) fn push_triangle_edge_height_conflict(
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
