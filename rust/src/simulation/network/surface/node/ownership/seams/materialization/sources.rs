// SPDX-License-Identifier: GPL-2.0-only

//! Source seam queries used after owned-edge materialization.

use super::*;
use crate::simulation::network::surface::keys::SurfaceXzKey;
use crate::simulation::network::surface::node::arrangement::{
    PreparedSeamConstraintCoverages, SeamConstraintCoverageScratch,
    prepared_seam_constraints_covering_surface_key_edge_as_fragments_into,
};

pub(in crate::simulation::network::surface::node::ownership) fn owned_source_constraints_for_edge<
    'a,
>(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    constraints: &PreparedSeamConstraintCoverages<'a>,
    scratch: &mut SeamConstraintCoverageScratch,
    matches: &mut Vec<&'a NodeRegionSeamConstraint>,
) {
    prepared_seam_constraints_covering_surface_key_edge_as_fragments_into(
        SurfaceXzKey::from_raw_tuple(start),
        SurfaceXzKey::from_raw_tuple(end),
        constraints,
        scratch,
        matches,
    );
    matches.dedup_by_key(|constraint| constraint.constraint_index);
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
    rail_constraint_index: &OwnedEdgeRailConstraintIndex<'_>,
    candidate_indices: &[usize],
    intervals: &mut Vec<(i128, i128)>,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    if piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN {
        return Vec::new();
    }
    let mut source_constraint_indices = candidate_indices
        .iter()
        .copied()
        .filter(|&index| rail_constraint_index.constraint_covers_owned_edge(index, start, end))
        .filter_map(|index| {
            Some((
                rail_constraint_index.constraint(index)?,
                rail_constraint_index.constraint_points(index)?,
            ))
        })
        .filter(|(constraint, _)| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
        .filter(|(constraint, _)| !constraint_is_point_contact(constraint))
        .filter(|(constraint, _)| {
            rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        })
        .filter(|(_, points)| {
            edge_lies_on_constraint_polyline_on_overlay_grid(start, end, points, intervals)
        })
        .map(|(constraint, _)| constraint.constraint_index)
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

        let mut matches = Vec::new();
        let prepared = PreparedSeamConstraintCoverages::new(&constraints);
        owned_source_constraints_for_edge(
            ownership_key_from_road_point(RoadVec2::new(0.0, 0.0)),
            ownership_key_from_road_point(RoadVec2::new(3.0, 0.0)),
            &prepared,
            &mut SeamConstraintCoverageScratch::default(),
            &mut matches,
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

        let mut matches = Vec::new();
        let prepared = PreparedSeamConstraintCoverages::new(&constraints);
        owned_source_constraints_for_edge(
            ownership_key_from_road_point(RoadVec2::new(0.0, 0.0)),
            ownership_key_from_road_point(RoadVec2::new(3.0, 0.0)),
            &prepared,
            &mut SeamConstraintCoverageScratch::default(),
            &mut matches,
        );

        assert!(matches.is_empty());
    }
}
