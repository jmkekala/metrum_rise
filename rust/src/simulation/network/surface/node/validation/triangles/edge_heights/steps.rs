// SPDX-License-Identifier: GPL-2.0-only

//! Explicit vertical-step authorization for cross-region height edges.

use super::*;
use crate::simulation::network::surface::RoadSurfaceVisualNodePieceKind;

pub(super) fn cross_region_edges_form_explicit_vertical_step(
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
    if solution.piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN {
        return false;
    }
    let Some(bridge_owner) = explicit_step_segment_bridge_owner(step_segment, direct_owner) else {
        return false;
    };
    if bridge_owner.kind() != missing_owner.kind() || bridge_owner == missing_owner {
        return false;
    }
    let Some(missing_height_field) = solution
        .regions
        .get(missing_edge.region_index)
        .map(|region| region.height_field_id)
    else {
        return false;
    };
    if !solution.regions.iter().any(|region| {
        region.owner == bridge_owner
            && region.height_field_id.kind() == missing_height_field.kind()
            && region.height_field_id.band_index() == missing_height_field.band_index()
    }) {
        return false;
    }
    let has_same_kind_handoff = solution
        .explicit_vertical_step_segments
        .iter()
        .copied()
        .any(|segment| {
            explicit_vertical_step_owners_match_regions(segment, bridge_owner, missing_owner)
                && edge_lies_on_explicit_vertical_step(segment, edge)
        });
    let has_paired_step_handoff = solution
        .explicit_vertical_step_segments
        .iter()
        .copied()
        .filter(|segment| edge_lies_on_explicit_vertical_step(*segment, edge))
        .filter_map(|segment| explicit_step_segment_bridge_owner(segment, missing_owner))
        .any(|paired_owner| paired_owner.kind() == direct_owner.kind());
    if !has_same_kind_handoff && !has_paired_step_handoff {
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

pub(super) fn edge_lies_on_explicit_vertical_step(
    segment: NodeExplicitVerticalStepSegment,
    edge: NodeValidationEdgeKey,
) -> bool {
    let start = NodeValidationPointKey::from_arrangement_key(segment.start());
    let end = NodeValidationPointKey::from_arrangement_key(segment.end());
    point_lies_on_validation_segment_or_dust(edge.start, start, end)
        && point_lies_on_validation_segment_or_dust(edge.end, start, end)
}

pub(super) fn explicit_vertical_step_owners_match_regions(
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

fn point_lies_on_validation_segment_or_dust(
    point: NodeValidationPointKey,
    start: NodeValidationPointKey,
    end: NodeValidationPointKey,
) -> bool {
    if point_lies_on_validation_segment(point, start, end) {
        return true;
    }
    let dx = i128::from(end.x_key - start.x_key);
    let dz = i128::from(end.z_key - start.z_key);
    let denominator = dx * dx + dz * dz;
    if denominator == 0 {
        return false;
    }
    let px = i128::from(point.x_key - start.x_key);
    let pz = i128::from(point.z_key - start.z_key);
    let numerator = px * dx + pz * dz;
    let length_key_units = (denominator as f64).sqrt();
    let dust_key_units = validation_explicit_step_dust_key_units() as f64;
    let endpoint_padding = dust_key_units * length_key_units;
    let numerator_f64 = numerator as f64;
    if numerator_f64 < -endpoint_padding || numerator_f64 > denominator as f64 + endpoint_padding {
        return false;
    }
    let cross = dx * pz - dz * px;
    cross.unsigned_abs() as f64 <= dust_key_units * length_key_units
}

fn validation_explicit_step_dust_key_units() -> i64 {
    (f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * SURFACE_XZ_KEY_SCALE)
        .round()
        .max(1.0) as i64
}
