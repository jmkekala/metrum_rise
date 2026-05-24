//! Explicit vertical-step footprint authorization.

use super::*;

pub(super) fn explicit_vertical_step_authorizes_footprint_height_pair(
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

pub(super) fn explicit_same_kind_vertical_step_authorizes_footprint_height_pair(
    key: arrangement::NodeArrangementKey,
    lower: NodeFootprintBoundaryDirectVertex,
    raised: NodeFootprintBoundaryDirectVertex,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> bool {
    if lower.owner_kind != raised.owner_kind
        || lower.owner_index == raised.owner_index
        || raised_step_band_rank(lower.owner_kind).is_none()
    {
        return false;
    }
    let lower_owner = arrangement::NodeBandOwner::new(lower.owner_kind, lower.owner_index);
    let raised_owner = arrangement::NodeBandOwner::new(raised.owner_kind, raised.owner_index);
    explicit_vertical_step_segments.iter().any(|segment| {
        arrangement_key_lies_exactly_on_segment(key, segment.start(), segment.end())
            && ((segment.owner() == lower_owner && segment.opposite_owner() == raised_owner)
                || (segment.owner() == raised_owner && segment.opposite_owner() == lower_owner))
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
