//! Candidate selection for owned-edge seam materialization.

use super::super::super::super::RoadSurfaceVisualNodePieceKind;
use super::super::super::super::arrangement::NodeBandOwner;
use super::super::super::super::rails::{NodeRailConstraint, NodeRailConstraintKind};
use super::super::super::topology_keys::{
    NodeOwnershipPointKey, canonical_source_indices, ownership_key_from_road_point,
};
use super::super::predicates::{
    constraint_applies_to_owner, constraint_is_material_transition, constraint_is_point_contact,
};

pub(super) enum OwnedEdgeSeamCandidate<'a> {
    RailConstraint(&'a NodeRailConstraint),
    EndpointPair {
        constraint_index: usize,
        kind: NodeRailConstraintKind,
    },
    SourceConstraint {
        constraint_index: usize,
        kind: NodeRailConstraintKind,
    },
}

pub(super) fn materialized_seam_candidates_for_owned_edge<'a>(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &'a [NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Vec<OwnedEdgeSeamCandidate<'a>> {
    let matching_constraints = matching_rail_constraints_for_owned_edge(
        start,
        end,
        rail_constraints,
        owner,
        opposite_owner,
        piece_kind,
    );
    if !matching_constraints.is_empty() {
        let has_exact_owner_pair_source = matching_constraints.iter().any(|constraint| {
            super::rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        });
        return matching_constraints
            .into_iter()
            .filter(|constraint| {
                !has_exact_owner_pair_source
                    || super::rail_constraint_owner_pair_matches_edge(
                        constraint,
                        owner,
                        opposite_owner,
                    )
            })
            .map(OwnedEdgeSeamCandidate::RailConstraint)
            .collect();
    }

    if let Some(kind) = super::material_contact_kind_for_owned_edge(owner, opposite_owner) {
        let endpoint_pair_sources = materialized_endpoint_pair_constraint_indices_for_owned_edge(
            start,
            end,
            rail_constraints,
            owner,
            opposite_owner,
        );
        if !endpoint_pair_sources.is_empty() {
            return endpoint_pair_sources
                .into_iter()
                .map(|constraint_index| OwnedEdgeSeamCandidate::EndpointPair {
                    constraint_index,
                    kind,
                })
                .collect();
        }
    }

    materialized_source_constraint_for_owned_step_edge(
        start,
        end,
        rail_constraints,
        owner,
        opposite_owner,
        piece_kind,
    )
    .map(|(constraint_index, kind)| {
        vec![OwnedEdgeSeamCandidate::SourceConstraint {
            constraint_index,
            kind,
        }]
    })
    .unwrap_or_default()
}

fn matching_rail_constraints_for_owned_edge<'a>(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &'a [NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Vec<&'a NodeRailConstraint> {
    rail_constraints
        .iter()
        .filter(|constraint| {
            super::rail_constraint_can_materialize_for_owned_edge(constraint, owner, opposite_owner)
        })
        .filter(|constraint| {
            super::owned_edge_lies_on_rail_constraint(
                start,
                end,
                constraint,
                owner,
                opposite_owner,
                piece_kind,
            )
        })
        .collect()
}

fn materialized_endpoint_pair_constraint_indices_for_owned_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    let Some(kind) = super::material_contact_kind_for_owned_edge(owner, opposite_owner) else {
        return Vec::new();
    };
    let Some(start_constraint_index) = exact_owner_pair_point_contact_constraint_index_at_key(
        start,
        rail_constraints,
        owner,
        opposite_owner,
        kind,
    ) else {
        return Vec::new();
    };
    let Some(end_constraint_index) = exact_owner_pair_point_contact_constraint_index_at_key(
        end,
        rail_constraints,
        owner,
        opposite_owner,
        kind,
    ) else {
        return Vec::new();
    };
    canonical_source_indices([start_constraint_index, end_constraint_index])
}

fn materialized_source_constraint_for_owned_step_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Option<(usize, NodeRailConstraintKind)> {
    let kind = super::material_contact_kind_for_owned_edge(owner, opposite_owner)?;
    rail_constraints
        .iter()
        .filter(|constraint| {
            constraint_applies_to_owner(constraint, owner)
                || constraint_applies_to_owner(constraint, opposite_owner)
        })
        .filter(|constraint| {
            super::owned_edge_lies_on_rail_constraint(
                start,
                end,
                constraint,
                owner,
                opposite_owner,
                piece_kind,
            )
        })
        .min_by_key(|constraint| {
            (
                constraint_is_material_transition(constraint),
                constraint.constraint_index,
            )
        })
        .map(|constraint| (constraint.constraint_index, kind))
}

fn exact_owner_pair_point_contact_constraint_index_at_key(
    key: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    kind: NodeRailConstraintKind,
) -> Option<usize> {
    rail_constraints
        .iter()
        .filter(|constraint| constraint.kind == kind)
        .filter(|constraint| {
            super::rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        })
        .filter(|constraint| constraint_is_point_contact(constraint))
        .filter(|constraint| {
            constraint
                .points_xz
                .first()
                .copied()
                .map(ownership_key_from_road_point)
                == Some(key)
        })
        .map(|constraint| constraint.constraint_index)
        .min()
}
