//! Explicit vertical-step authorization for cross-region height edges.

use super::*;

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

pub(super) fn edge_lies_on_explicit_vertical_step(
    segment: NodeExplicitVerticalStepSegment,
    edge: NodeValidationEdgeKey,
) -> bool {
    let start = NodeValidationPointKey::from_arrangement_key(segment.start());
    let end = NodeValidationPointKey::from_arrangement_key(segment.end());
    point_lies_on_validation_segment(edge.start, start, end)
        && point_lies_on_validation_segment(edge.end, start, end)
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
