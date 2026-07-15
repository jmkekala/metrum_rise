//! Source seam queries used after owned-edge materialization.

use super::*;
use crate::simulation::network::surface::keys::SurfaceXzKey;
use crate::simulation::network::surface::node::arrangement::seam_constraints_covering_surface_key_edge_as_fragments;

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
    matches.extend(seam_constraints_covering_surface_key_edge_as_fragments(
        SurfaceXzKey::from_raw_tuple(start),
        SurfaceXzKey::from_raw_tuple(end),
        constraints,
    ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::RoadSurfaceBandKind;
    use crate::simulation::network::surface::backend::RoadVec2;

    fn source_fragment(
        constraint_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start_xz: RoadVec2,
        end_xz: RoadVec2,
    ) -> NodeRegionSeamConstraint {
        NodeRegionSeamConstraint {
            constraint_index,
            seam_source: NodeSeamSource::RaisedStepContact {
                owner_index: owner.owner_index(),
            },
            owner: Some(owner),
            opposite_owner: Some(opposite_owner),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz,
            end_xz,
        }
    }

    #[test]
    fn owned_source_constraints_accept_same_source_fragments_covering_edge() {
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
        let constraints = vec![
            source_fragment(
                91,
                curb,
                sidewalk,
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(1.0, 0.0),
            ),
            source_fragment(
                91,
                curb,
                sidewalk,
                RoadVec2::new(1.0, 0.0),
                RoadVec2::new(3.0, 0.0),
            ),
        ];

        let matches = owned_source_constraints_for_edge(
            ownership_key_from_road_point(RoadVec2::new(0.0, 0.0)),
            ownership_key_from_road_point(RoadVec2::new(3.0, 0.0)),
            &constraints,
        );

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].constraint_index, 91);
    }

    #[test]
    fn owned_source_constraints_reject_gapped_source_fragments() {
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
        let constraints = vec![
            source_fragment(
                92,
                curb,
                sidewalk,
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(1.0, 0.0),
            ),
            source_fragment(
                92,
                curb,
                sidewalk,
                RoadVec2::new(2.0, 0.0),
                RoadVec2::new(3.0, 0.0),
            ),
        ];

        let matches = owned_source_constraints_for_edge(
            ownership_key_from_road_point(RoadVec2::new(0.0, 0.0)),
            ownership_key_from_road_point(RoadVec2::new(3.0, 0.0)),
            &constraints,
        );

        assert!(matches.is_empty());
    }
}
