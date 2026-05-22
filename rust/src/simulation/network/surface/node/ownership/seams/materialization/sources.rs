//! Source seam queries used after owned-edge materialization.

use super::*;

pub(in crate::simulation::network::surface::node::ownership) fn owned_source_constraints_for_edge<
    'a,
>(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    constraints: &'a [NodeRegionSeamConstraint],
) -> Vec<&'a NodeRegionSeamConstraint> {
    let mut matches = constraints
        .iter()
        .filter(|constraint| {
            let constraint_start = ownership_key_from_road_point(constraint.start_xz);
            let constraint_end = ownership_key_from_road_point(constraint.end_xz);
            point_key_lies_on_segment(start, constraint_start, constraint_end)
                && point_key_lies_on_segment(end, constraint_start, constraint_end)
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|constraint| (constraint.priority_key(), constraint.constraint_index));
    matches.dedup_by_key(|constraint| constraint.constraint_index);
    matches
}

pub(in crate::simulation::network::surface::node::ownership) fn owned_boundary_requires_explicit_seam(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    owner.kind() != opposite_owner.kind()
}

pub(in crate::simulation::network::surface::node::ownership) fn junctionn_unmaterialized_raised_step_authority_indices_for_edge(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    if piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN {
        return Vec::new();
    }
    let mut source_constraint_indices = rail_constraints
        .iter()
        .filter(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
        .filter(|constraint| !constraint_is_point_contact(constraint))
        .filter(|constraint| {
            rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        })
        .filter(|constraint| {
            edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint)
        })
        .map(|constraint| constraint.constraint_index)
        .collect::<Vec<_>>();
    source_constraint_indices.sort_unstable();
    source_constraint_indices.dedup();
    source_constraint_indices
}

pub(in crate::simulation::network::surface::node::ownership) fn source_constraints_materialize_raised_step_authority(
    source_constraints: &[&NodeRegionSeamConstraint],
    source_constraint_indices: &[usize],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    source_constraints.iter().any(|constraint| {
        source_constraint_indices.contains(&constraint.constraint_index)
            && constraint.is_material_transition
            && seam_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
            && matches!(
                constraint.seam_source,
                NodeSeamSource::RaisedStepContact { .. }
            )
    })
}

fn seam_constraint_owner_pair_matches_edge(
    constraint: &NodeRegionSeamConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(
        (constraint.owner, constraint.opposite_owner),
        (Some(left), Some(right))
            if (left == owner && right == opposite_owner)
                || (left == opposite_owner && right == owner)
    )
}
