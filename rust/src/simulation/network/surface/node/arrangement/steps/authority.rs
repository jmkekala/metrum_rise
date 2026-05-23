//! Explicit vertical-step owner-pair authority helpers.

use super::*;

pub(in crate::simulation::network::surface::node::arrangement) fn owner_sets_have_explicit_vertical_step_endpoint_authority(
    key: NodeArrangementKey,
    left_owners: &[NodeBandOwner],
    right_owners: &[NodeBandOwner],
    segments: &[NodeExplicitVerticalStepSegment],
) -> bool {
    left_owners.iter().copied().any(|left_owner| {
        right_owners.iter().copied().any(|right_owner| {
            let Some(left_rank) = raised_step_band_rank(left_owner.kind()) else {
                return false;
            };
            let Some(right_rank) = raised_step_band_rank(right_owner.kind()) else {
                return false;
            };
            match left_rank.cmp(&right_rank) {
                std::cmp::Ordering::Less => {
                    explicit_vertical_step_segments_authorize_height_side_at_key(
                        key, left_owner, true, segments,
                    ) && explicit_vertical_step_segments_authorize_height_side_at_key(
                        key,
                        right_owner,
                        false,
                        segments,
                    )
                }
                std::cmp::Ordering::Greater => {
                    explicit_vertical_step_segments_authorize_height_side_at_key(
                        key, left_owner, false, segments,
                    ) && explicit_vertical_step_segments_authorize_height_side_at_key(
                        key,
                        right_owner,
                        true,
                        segments,
                    )
                }
                std::cmp::Ordering::Equal => false,
            }
        })
    })
}

pub(crate) fn owners_form_explicit_vertical_step_pair(a: NodeBandOwner, b: NodeBandOwner) -> bool {
    if a == b {
        return false;
    }
    if a.kind() == b.kind() {
        return raised_step_band_rank(a.kind()).is_some();
    }
    ordered_raised_step_kinds(a.kind(), b.kind()).is_some()
}

pub(in crate::simulation::network::surface::node::arrangement) fn owner_sets_match_step(
    left_owners: &[NodeBandOwner],
    right_owners: &[NodeBandOwner],
    step_owner: NodeBandOwner,
    step_opposite_owner: NodeBandOwner,
) -> bool {
    (left_owners.contains(&step_owner) && right_owners.contains(&step_opposite_owner))
        || (left_owners.contains(&step_opposite_owner) && right_owners.contains(&step_owner))
}
