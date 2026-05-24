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
    let mut candidates = Vec::new();
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
        candidates.extend(
            matching_constraints
                .into_iter()
                .filter(|constraint| {
                    !has_exact_owner_pair_source
                        || super::rail_constraint_owner_pair_matches_edge(
                            constraint,
                            owner,
                            opposite_owner,
                        )
                })
                .map(OwnedEdgeSeamCandidate::RailConstraint),
        );
    }

    if let Some(kind) = super::material_contact_kind_for_owned_edge(owner, opposite_owner) {
        let endpoint_pair_sources = materialized_endpoint_pair_constraint_indices_for_owned_edge(
            start,
            end,
            rail_constraints,
            owner,
            opposite_owner,
            piece_kind,
        );
        if !endpoint_pair_sources.is_empty() {
            candidates.extend(endpoint_pair_sources.into_iter().map(|constraint_index| {
                OwnedEdgeSeamCandidate::EndpointPair {
                    constraint_index,
                    kind,
                }
            }));
        }
    }

    if !candidates.is_empty() {
        return candidates;
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

pub(in crate::simulation::network::surface::node::ownership) fn materialized_endpoint_pair_constraint_indices_for_owned_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Vec<usize> {
    let Some(kind) = super::material_contact_kind_for_owned_edge(owner, opposite_owner) else {
        return Vec::new();
    };
    let start_constraint_indices = source_authorized_point_contact_constraint_indices_at_key(
        start,
        rail_constraints,
        owner,
        opposite_owner,
        kind,
        piece_kind,
    );
    if start_constraint_indices.is_empty() {
        return Vec::new();
    };
    let end_constraint_indices = source_authorized_point_contact_constraint_indices_at_key(
        end,
        rail_constraints,
        owner,
        opposite_owner,
        kind,
        piece_kind,
    );
    if end_constraint_indices.is_empty() {
        return Vec::new();
    };
    canonical_source_indices(
        start_constraint_indices
            .into_iter()
            .chain(end_constraint_indices),
    )
}

fn source_authorized_point_contact_constraint_indices_at_key(
    key: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    kind: NodeRailConstraintKind,
    _piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Vec<usize> {
    if let Some(constraint_index) = exact_owner_pair_point_contact_constraint_index_at_key(
        key,
        rail_constraints,
        owner,
        opposite_owner,
        kind,
    ) {
        return vec![constraint_index];
    }
    Vec::new()
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

#[cfg(test)]
mod tests {
    use super::super::super::super::topology_keys::road_point_from_key;
    use super::*;
    use crate::simulation::network::surface::RoadSurfaceBandKind;

    #[test]
    fn junctionn_endpoint_pair_rejects_same_kind_source_vertex_handoff() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 7);
        let source_opposite = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
        let final_opposite = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 6);
        let start = (7_101_408, 5_000_000);
        let end = (7_491_119, 5_675_001);
        let constraints = vec![
            NodeRailConstraint {
                constraint_index: 39,
                kind: NodeRailConstraintKind::RaisedStepContact,
                source_mouth_order_index: 0,
                source_band_index: Some(2),
                source_boundary_index: Some(1),
                owner: Some(owner),
                opposite_owner: Some(source_opposite),
                points_xz: vec![
                    road_point_from_key((6_321_985, 3_650_000)),
                    road_point_from_key(start),
                ],
            },
            NodeRailConstraint {
                constraint_index: 255,
                kind: NodeRailConstraintKind::RaisedStepContact,
                source_mouth_order_index: 0,
                source_band_index: Some(2),
                source_boundary_index: None,
                owner: Some(owner),
                opposite_owner: Some(final_opposite),
                points_xz: vec![road_point_from_key(end), road_point_from_key(end)],
            },
        ];

        let candidates = materialized_seam_candidates_for_owned_edge(
            start,
            end,
            &constraints,
            owner,
            final_opposite,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        );

        assert!(
            candidates.is_empty(),
            "same-kind owner substitution must not authorize a seam without exact source ownership"
        );
    }

    #[test]
    fn generated_contact_does_not_authorize_different_same_kind_owner_pair() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 10);
        let source_opposite = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 11);
        let final_opposite = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
        let start = (0, 3_650_556);
        let end = (31_912_125, 4_207_584);
        let constraints = vec![
            NodeRailConstraint {
                constraint_index: 1350,
                kind: NodeRailConstraintKind::RaisedStepContact,
                source_mouth_order_index: 1,
                source_band_index: None,
                source_boundary_index: None,
                owner: Some(owner),
                opposite_owner: Some(source_opposite),
                points_xz: vec![
                    road_point_from_key((23_918_169, 4_068_049)),
                    road_point_from_key(end),
                ],
            },
            NodeRailConstraint {
                constraint_index: 1351,
                kind: NodeRailConstraintKind::RaisedStepContact,
                source_mouth_order_index: 0,
                source_band_index: None,
                source_boundary_index: None,
                owner: Some(owner),
                opposite_owner: Some(final_opposite),
                points_xz: vec![road_point_from_key(start), road_point_from_key(start)],
            },
        ];

        let candidates = materialized_endpoint_pair_constraint_indices_for_owned_edge(
            start,
            end,
            &constraints,
            owner,
            final_opposite,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        );

        assert!(
            candidates.is_empty(),
            "generated contacts must not be reinterpreted as a different same-kind owner pair"
        );
    }
}
