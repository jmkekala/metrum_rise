//! Raised-step footprint height candidate selection.

use super::*;

#[cfg(test)]
pub(in crate::simulation::network::surface::node::boundary::heights) fn raised_step_footprint_height_candidate(
    key: arrangement::NodeArrangementKey,
    candidates: &[NodeFootprintBoundaryHeightCandidate],
    heights: &[i64],
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> Option<NodeFootprintBoundaryHeightCandidate> {
    // Raised-step corners can put a lower material edge and its raised neighbor at one footprint
    // key. Accept that only when a materialized vertical-step segment or exact terminal source-edge
    // endpoints prove the ordered owner pair at that canonical key; unrelated cross-material
    // conflicts still reject.
    let [_, _] = heights else {
        return None;
    };

    let mut raised_candidates = Vec::new();
    let mut checked_pairs = 0usize;
    for (left_index, left) in candidates.iter().copied().enumerate() {
        for right in candidates.iter().copied().skip(left_index + 1) {
            if left.height_mm == right.height_mm {
                continue;
            }
            checked_pairs += 1;
            let Some((lower, raised)) = ordered_raised_step_footprint_candidates(left, right)
            else {
                return None;
            };
            let explicit_step_authorized = explicit_vertical_step_authorizes_footprint_height_pair(
                key,
                lower.source,
                raised.source,
                explicit_vertical_step_segments,
            );
            let terminal_source_endpoint_authorized =
                terminal_source_edge_endpoints_authorize_footprint_height_pair(
                    key,
                    lower,
                    raised,
                    source_edges,
                );
            if !explicit_step_authorized && !terminal_source_endpoint_authorized {
                continue;
            }
            if !raised_candidates
                .iter()
                .any(|candidate: &NodeFootprintBoundaryHeightCandidate| *candidate == raised)
            {
                raised_candidates.push(raised);
            }
        }
    }
    if checked_pairs == 0 || raised_candidates.is_empty() {
        return None;
    }
    let mut source = None;
    for candidate in raised_candidates {
        let point_key = ArrangementBoundaryPointKey {
            x_key: key.x_key(),
            z_key: key.z_key(),
            y_mm: candidate.height_mm,
        };
        if merge_node_footprint_boundary_point_source(point_key, &mut source, candidate.source)
            .is_err()
        {
            return None;
        }
    }
    source.map(|source| NodeFootprintBoundaryHeightCandidate {
        height_mm: heights[1],
        source,
    })
}

pub(in crate::simulation::network::surface::node::boundary::heights) fn raised_step_footprint_height_mm(
    key: arrangement::NodeArrangementKey,
    candidates: &[NodeFootprintBoundaryHeightCandidate],
    heights: &[i64],
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> Option<i64> {
    let [lower_height_mm, raised_height_mm] = heights else {
        return None;
    };

    raised_step_footprint_authorized_rank_pairs(
        key,
        candidates,
        *lower_height_mm,
        *raised_height_mm,
        explicit_vertical_step_segments,
        source_edges,
    )
    .is_some()
    .then_some(*raised_height_mm)
}
