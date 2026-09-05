//! Rail-source authority validation helpers.

use super::*;

pub(in crate::simulation::network::surface::node::ownership) fn canonical_points_by_mm_key_by_owner(
    points_by_owner: &BTreeMap<NodeBandOwner, Vec<NodeOwnershipPointKey>>,
) -> BTreeMap<NodeBandOwner, BTreeMap<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>> {
    let mut by_owner = BTreeMap::new();
    for (owner, points) in points_by_owner {
        let mut by_mm_key =
            BTreeMap::<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>::new();
        for point in points {
            by_mm_key
                .entry(ownership_mm_key(*point))
                .or_default()
                .insert(*point);
        }
        by_owner.insert(*owner, by_mm_key);
    }
    by_owner
}

#[cfg(test)]
pub(in crate::simulation::network::surface::node::ownership) fn validate_owned_region_vertices_against_carrier_closure(
    regions: &[NodeBooleanOwnedRegion],
    rails: &NodeRailContourSet,
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<NodeCarrierProvenanceClosure, NodeBooleanOwnershipError> {
    NodeCarrierProvenanceClosure::from_owned_regions(regions, rails, rail_points)
}

pub(in crate::simulation::network::surface::node::ownership) fn constraint_authority_owners(
    constraint: &NodeRailConstraint,
) -> impl Iterator<Item = NodeBandOwner> {
    let owners = match (constraint.owner, constraint.opposite_owner) {
        (None, None) => [None, None],
        (Some(owner), None) | (None, Some(owner)) => [Some(owner), None],
        (Some(left), Some(right)) if left == right => [Some(left), None],
        (Some(left), Some(right)) if left < right => [Some(left), Some(right)],
        (Some(left), Some(right)) => [Some(right), Some(left)],
    };
    owners.into_iter().flatten()
}
