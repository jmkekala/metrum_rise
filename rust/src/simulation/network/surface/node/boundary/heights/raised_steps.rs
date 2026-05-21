//! Raised-step boundary height authorization.

use super::*;

#[cfg(test)]
pub(super) fn raised_step_footprint_height_candidate(
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

pub(super) fn raised_step_footprint_height_mm(
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

fn raised_step_footprint_authorized_rank_pairs(
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

fn explicit_vertical_step_endpoint_group_authorizes_footprint_height_pair(
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
    owner_has_explicit_vertical_step_side_at_key(
        key,
        lower_owner,
        true,
        explicit_vertical_step_segments,
    ) && owner_has_explicit_vertical_step_side_at_key(
        key,
        raised_owner,
        false,
        explicit_vertical_step_segments,
    )
}

fn owner_has_explicit_vertical_step_side_at_key(
    key: arrangement::NodeArrangementKey,
    owner: arrangement::NodeBandOwner,
    lower_side: bool,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> bool {
    explicit_vertical_step_segments
        .iter()
        .copied()
        .any(|segment| {
            arrangement_key_lies_exactly_on_segment(key, segment.start(), segment.end())
                && owner_matches_explicit_vertical_step_side(owner, lower_side, segment)
        })
}

fn owner_matches_explicit_vertical_step_side(
    owner: arrangement::NodeBandOwner,
    lower_side: bool,
    segment: arrangement::NodeExplicitVerticalStepSegment,
) -> bool {
    let segment_owner = segment.owner();
    let opposite_owner = segment.opposite_owner();
    let Some(owner_rank) = raised_step_band_rank(segment_owner.kind()) else {
        return false;
    };
    let Some(opposite_rank) = raised_step_band_rank(opposite_owner.kind()) else {
        return false;
    };
    match owner_rank.cmp(&opposite_rank) {
        std::cmp::Ordering::Less => {
            (lower_side && owner == segment_owner) || (!lower_side && owner == opposite_owner)
        }
        std::cmp::Ordering::Greater => {
            (lower_side && owner == opposite_owner) || (!lower_side && owner == segment_owner)
        }
        std::cmp::Ordering::Equal => false,
    }
}

#[cfg(test)]
fn ordered_raised_step_footprint_candidates(
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

fn explicit_vertical_step_authorizes_footprint_height_pair(
    key: arrangement::NodeArrangementKey,
    lower: NodeFootprintBoundaryDirectVertex,
    raised: NodeFootprintBoundaryDirectVertex,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> bool {
    let lower_owner = arrangement::NodeBandOwner::new(lower.owner_kind, lower.owner_index);
    let raised_owner = arrangement::NodeBandOwner::new(raised.owner_kind, raised.owner_index);
    explicit_vertical_step_segments.iter().any(|segment| {
        arrangement_key_lies_exactly_on_segment(key, segment.start(), segment.end())
            && vertical_step_segment_authorizes_owner_pair(*segment, lower_owner, raised_owner)
    })
}

fn vertical_step_segment_authorizes_owner_pair(
    segment: arrangement::NodeExplicitVerticalStepSegment,
    lower_owner: arrangement::NodeBandOwner,
    raised_owner: arrangement::NodeBandOwner,
) -> bool {
    ((segment.owner() == lower_owner && segment.opposite_owner() == raised_owner)
        || (segment.owner() == raised_owner && segment.opposite_owner() == lower_owner))
        && raised_step_kinds_can_contact(lower_owner.kind(), raised_owner.kind())
        && raised_step_band_rank(lower_owner.kind())
            .zip(raised_step_band_rank(raised_owner.kind()))
            .is_some_and(|(lower_rank, raised_rank)| lower_rank < raised_rank)
}

fn terminal_source_edge_endpoints_authorize_footprint_height_pair(
    key: arrangement::NodeArrangementKey,
    lower: NodeFootprintBoundaryHeightCandidate,
    raised: NodeFootprintBoundaryHeightCandidate,
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> bool {
    if !raised_step_kinds_can_contact(lower.source.owner_kind, raised.source.owner_kind) {
        return false;
    }
    let Some(lower_rank) = raised_step_band_rank(lower.source.owner_kind) else {
        return false;
    };
    let Some(raised_rank) = raised_step_band_rank(raised.source.owner_kind) else {
        return false;
    };
    if lower_rank >= raised_rank {
        return false;
    }
    source_edges.iter().any(|lower_edge| {
        terminal_source_edge_endpoint_proves_candidate_at_key(lower_edge, key, lower)
            && source_edges.iter().any(|raised_edge| {
                terminal_source_edge_endpoint_proves_candidate_at_key(raised_edge, key, raised)
            })
    })
}

fn terminal_source_edge_endpoint_proves_candidate_at_key(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    key: arrangement::NodeArrangementKey,
    candidate: NodeFootprintBoundaryHeightCandidate,
) -> bool {
    if source_edge.kind != RoadSurfaceVisualNodePieceKind::Terminal
        || source_edge.owner_kind != candidate.source.owner_kind
        || source_edge.owner_index != candidate.source.owner_index
    {
        return false;
    }
    terminal_source_edge_endpoint_matches_candidate(
        source_edge.start_key,
        source_edge.start_point_key.y_mm,
        source_edge.start_source,
        key,
        candidate,
    ) || terminal_source_edge_endpoint_matches_candidate(
        source_edge.end_key,
        source_edge.end_point_key.y_mm,
        source_edge.end_source,
        key,
        candidate,
    )
}

fn terminal_source_edge_endpoint_matches_candidate(
    endpoint_key: arrangement::NodeArrangementKey,
    endpoint_height_mm: i64,
    endpoint_source: NodeFootprintBoundaryDirectSource,
    key: arrangement::NodeArrangementKey,
    candidate: NodeFootprintBoundaryHeightCandidate,
) -> bool {
    if endpoint_height_mm != candidate.height_mm {
        return false;
    }
    endpoint_key == key
        && candidate.source.source == NodeFootprintBoundaryVertexSource::Direct(endpoint_source)
}
