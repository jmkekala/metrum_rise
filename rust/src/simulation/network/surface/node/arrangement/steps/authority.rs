//! Explicit vertical-step owner-pair authority helpers.

use super::*;

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
