//! Raised-step owner-rank pairing helpers.

use super::*;

pub(super) fn raised_step_footprint_authorized_height_mm(
    key: arrangement::NodeArrangementKey,
    candidates: &[NodeFootprintBoundaryHeightCandidate],
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> Option<i64> {
    let mut raised_heights = Vec::<i64>::new();
    for left_index in 0..candidates.len() {
        for right in candidates.iter().copied().skip(left_index + 1) {
            let left = candidates[left_index];
            if left.height_mm == right.height_mm {
                continue;
            }
            let Some((lower, raised)) = ordered_raised_step_footprint_candidates(left, right)
            else {
                continue;
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
            let explicit_step_endpoint_group_authorized =
                explicit_vertical_step_endpoint_group_authorizes_footprint_height_pair(
                    key,
                    lower.source,
                    raised.source,
                    explicit_vertical_step_segments,
                );
            if !explicit_step_authorized
                && !terminal_source_endpoint_authorized
                && !explicit_step_endpoint_group_authorized
            {
                continue;
            }
            if !raised_heights.contains(&raised.height_mm) {
                raised_heights.push(raised.height_mm);
            }
        }
    }
    raised_heights.sort_unstable();
    raised_heights.dedup();
    let [raised_height_mm] = raised_heights.as_slice() else {
        return None;
    };
    Some(*raised_height_mm)
}

pub(super) fn explicit_vertical_step_endpoint_group_authorizes_footprint_height_pair(
    key: arrangement::NodeArrangementKey,
    lower: NodeFootprintBoundaryDirectVertex,
    raised: NodeFootprintBoundaryDirectVertex,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> bool {
    let Some(lower_rank) = raised_step_band_rank(lower.owner_kind) else {
        return false;
    };
    let Some(raised_rank) = raised_step_band_rank(raised.owner_kind) else {
        return false;
    };
    if lower_rank >= raised_rank {
        return false;
    }
    let lower_owner = arrangement::NodeBandOwner::new(lower.owner_kind, lower.owner_index);
    let raised_owner = arrangement::NodeBandOwner::new(raised.owner_kind, raised.owner_index);
    arrangement::explicit_vertical_step_segments_authorize_height_side_at_key(
        key,
        lower_owner,
        true,
        explicit_vertical_step_segments,
    ) && arrangement::explicit_vertical_step_segments_authorize_height_side_at_key(
        key,
        raised_owner,
        false,
        explicit_vertical_step_segments,
    )
}

pub(super) fn same_kind_explicit_vertical_step_footprint_height_mm(
    key: arrangement::NodeArrangementKey,
    candidates: &[NodeFootprintBoundaryHeightCandidate],
    lower_height_mm: i64,
    raised_height_mm: i64,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> Option<i64> {
    let lower_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.height_mm == lower_height_mm)
        .collect::<Vec<_>>();
    let raised_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.height_mm == raised_height_mm)
        .collect::<Vec<_>>();
    if lower_candidates.is_empty() || raised_candidates.is_empty() {
        return None;
    }

    let mut authorized_pairs = Vec::<(
        NodeFootprintBoundaryDirectVertex,
        NodeFootprintBoundaryDirectVertex,
    )>::new();
    for lower in &lower_candidates {
        for raised in &raised_candidates {
            if !explicit_same_kind_vertical_step_authorizes_footprint_height_pair(
                key,
                lower.source,
                raised.source,
                explicit_vertical_step_segments,
            ) {
                continue;
            }
            authorized_pairs.push((lower.source, raised.source));
        }
    }
    if authorized_pairs.is_empty() {
        return None;
    }

    let all_lower_authorized = lower_candidates.iter().all(|candidate| {
        authorized_pairs
            .iter()
            .any(|(lower, _)| *lower == candidate.source)
    });
    let all_raised_authorized = raised_candidates.iter().all(|candidate| {
        authorized_pairs
            .iter()
            .any(|(_, raised)| *raised == candidate.source)
    });
    (all_lower_authorized && all_raised_authorized).then_some(raised_height_mm)
}

pub(super) fn ordered_raised_step_footprint_candidates(
    left: NodeFootprintBoundaryHeightCandidate,
    right: NodeFootprintBoundaryHeightCandidate,
) -> Option<(
    NodeFootprintBoundaryHeightCandidate,
    NodeFootprintBoundaryHeightCandidate,
)> {
    let left_rank = raised_step_band_rank(left.source.owner_kind)?;
    let right_rank = raised_step_band_rank(right.source.owner_kind)?;
    match left_rank.cmp(&right_rank) {
        std::cmp::Ordering::Less => Some((left, right)),
        std::cmp::Ordering::Greater => Some((right, left)),
        std::cmp::Ordering::Equal => None,
    }
}
