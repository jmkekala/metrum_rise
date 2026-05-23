//! Raised-step owner-rank pairing helpers.

use super::*;

pub(super) fn raised_step_footprint_authorized_rank_pairs(
    key: arrangement::NodeArrangementKey,
    candidates: &[NodeFootprintBoundaryHeightCandidate],
    lower_height_mm: i64,
    raised_height_mm: i64,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> Option<Vec<(u8, u8)>> {
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

    for lower in &lower_candidates {
        let lower_rank = raised_step_band_rank(lower.source.owner_kind)?;
        for raised in &raised_candidates {
            let raised_rank = raised_step_band_rank(raised.source.owner_kind)?;
            if lower_rank >= raised_rank {
                return None;
            }
        }
    }

    let mut authorized_pairs = Vec::<(u8, u8)>::new();
    for lower in &lower_candidates {
        for raised in &raised_candidates {
            let explicit_step_authorized = explicit_vertical_step_authorizes_footprint_height_pair(
                key,
                lower.source,
                raised.source,
                explicit_vertical_step_segments,
            );
            let terminal_source_endpoint_authorized =
                terminal_source_edge_endpoints_authorize_footprint_height_pair(
                    key,
                    *lower,
                    *raised,
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
            let lower_rank = raised_step_band_rank(lower.source.owner_kind)?;
            let raised_rank = raised_step_band_rank(raised.source.owner_kind)?;
            let pair = (lower_rank, raised_rank);
            if !authorized_pairs.contains(&pair) {
                authorized_pairs.push(pair);
            }
        }
    }
    if authorized_pairs.is_empty() {
        return None;
    }

    for lower in &lower_candidates {
        let lower_rank = raised_step_band_rank(lower.source.owner_kind)?;
        if !authorized_pairs
            .iter()
            .any(|(authorized_lower_rank, _)| lower_rank <= *authorized_lower_rank)
        {
            return None;
        }
    }
    for raised in &raised_candidates {
        let raised_rank = raised_step_band_rank(raised.source.owner_kind)?;
        if !authorized_pairs
            .iter()
            .any(|(_, authorized_raised_rank)| raised_rank >= *authorized_raised_rank)
        {
            return None;
        }
    }

    Some(authorized_pairs)
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

#[cfg(test)]
pub(super) fn ordered_raised_step_footprint_candidates(
    left: NodeFootprintBoundaryHeightCandidate,
    right: NodeFootprintBoundaryHeightCandidate,
) -> Option<(
    NodeFootprintBoundaryHeightCandidate,
    NodeFootprintBoundaryHeightCandidate,
)> {
    if !raised_step_kinds_can_contact(left.source.owner_kind, right.source.owner_kind) {
        return None;
    }
    let left_rank = raised_step_band_rank(left.source.owner_kind)?;
    let right_rank = raised_step_band_rank(right.source.owner_kind)?;
    match left_rank.cmp(&right_rank) {
        std::cmp::Ordering::Less => Some((left, right)),
        std::cmp::Ordering::Greater => Some((right, left)),
        std::cmp::Ordering::Equal => None,
    }
}
