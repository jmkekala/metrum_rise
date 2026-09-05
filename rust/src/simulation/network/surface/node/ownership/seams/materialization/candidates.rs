// SPDX-License-Identifier: GPL-2.0-only

//! Candidate selection for owned-edge seam materialization.

use super::super::super::super::super::keys::SURFACE_XZ_KEY_SCALE;
use super::super::super::super::arrangement::NodeBandOwner;
use super::super::super::super::rails::{NodeRailConstraint, NodeRailConstraintKind};
use super::super::super::super::{
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, RoadSurfaceVisualNodePieceKind,
};
use super::super::super::topology_keys::{
    NodeOwnershipPointKey, canonical_source_indices, ownership_key_from_road_point,
};
use super::super::predicates::{
    constraint_applies_to_owner, constraint_is_material_transition, constraint_is_point_contact,
};
use std::collections::HashMap;

#[derive(Clone, Copy)]
struct OwnedEdgeRailConstraintBounds {
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

pub(in crate::simulation::network::surface::node::ownership) struct OwnedEdgeRailConstraintIndex<'a>
{
    constraints: &'a [NodeRailConstraint],
    point_keys: Vec<Vec<NodeOwnershipPointKey>>,
    bounds: Vec<Option<OwnedEdgeRailConstraintBounds>>,
    constraint_bits_by_owner: HashMap<NodeBandOwner, Vec<u64>>,
    ownerless_constraint_bits: Vec<u64>,
    point_contacts: HashMap<
        (
            NodeOwnershipPointKey,
            NodeRailConstraintKind,
            NodeBandOwner,
            NodeBandOwner,
        ),
        usize,
    >,
}

impl<'a> OwnedEdgeRailConstraintIndex<'a> {
    pub(in crate::simulation::network::surface::node::ownership) fn new(
        constraints: &'a [NodeRailConstraint],
    ) -> Self {
        let mut point_keys_by_constraint = Vec::with_capacity(constraints.len());
        let mut bounds = Vec::with_capacity(constraints.len());
        let mut point_contacts = HashMap::new();
        let constraint_word_count = constraints.len().div_ceil(u64::BITS as usize);
        let mut constraint_bits_by_owner = HashMap::<NodeBandOwner, Vec<u64>>::new();
        let mut ownerless_constraint_bits = vec![0; constraint_word_count];
        for (constraint_position, constraint) in constraints.iter().enumerate() {
            let point_keys = constraint
                .points_xz
                .iter()
                .copied()
                .map(ownership_key_from_road_point)
                .collect::<Vec<_>>();
            let constraint_bounds = point_keys.first().copied().map(|first| {
                point_keys.iter().copied().skip(1).fold(
                    OwnedEdgeRailConstraintBounds {
                        min_x: first.0,
                        min_z: first.1,
                        max_x: first.0,
                        max_z: first.1,
                    },
                    |mut bounds, point| {
                        bounds.min_x = bounds.min_x.min(point.0);
                        bounds.min_z = bounds.min_z.min(point.1);
                        bounds.max_x = bounds.max_x.max(point.0);
                        bounds.max_z = bounds.max_z.max(point.1);
                        bounds
                    },
                )
            });
            point_keys_by_constraint.push(point_keys);
            bounds.push(constraint_bounds);
            let mut indexed_owner = false;
            for owner in [constraint.owner, constraint.opposite_owner]
                .into_iter()
                .flatten()
            {
                indexed_owner = true;
                let bits = constraint_bits_by_owner
                    .entry(owner)
                    .or_insert_with(|| vec![0; constraint_word_count]);
                bits[constraint_position / u64::BITS as usize] |=
                    1_u64 << (constraint_position % u64::BITS as usize);
            }
            if !indexed_owner {
                ownerless_constraint_bits[constraint_position / u64::BITS as usize] |=
                    1_u64 << (constraint_position % u64::BITS as usize);
            }
            let (Some(owner), Some(opposite_owner), Some(first)) = (
                constraint.owner,
                constraint.opposite_owner,
                constraint.points_xz.first().copied(),
            ) else {
                continue;
            };
            if !constraint_is_point_contact(constraint) {
                continue;
            }
            let (owner, opposite_owner) = canonical_owner_pair(owner, opposite_owner);
            point_contacts
                .entry((
                    ownership_key_from_road_point(first),
                    constraint.kind,
                    owner,
                    opposite_owner,
                ))
                .and_modify(|selected: &mut usize| {
                    *selected = (*selected).min(constraint.constraint_index);
                })
                .or_insert(constraint.constraint_index);
        }
        Self {
            constraints,
            point_keys: point_keys_by_constraint,
            bounds,
            constraint_bits_by_owner,
            ownerless_constraint_bits,
            point_contacts,
        }
    }

    pub(in crate::simulation::network::surface::node::ownership) fn candidate_indices_for_owned_edge(
        &self,
        start: NodeOwnershipPointKey,
        end: NodeOwnershipPointKey,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        candidate_indices: &mut Vec<usize>,
    ) {
        let min_x = start.0.min(end.0);
        let min_z = start.1.min(end.1);
        let max_x = start.0.max(end.0);
        let max_z = start.1.max(end.1);
        let margin =
            (f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * SURFACE_XZ_KEY_SCALE).round() as i64;
        let query_min_x = min_x.saturating_sub(margin);
        let query_min_z = min_z.saturating_sub(margin);
        let query_max_x = max_x.saturating_add(margin);
        let query_max_z = max_z.saturating_add(margin);
        candidate_indices.clear();
        let owner_bits = self.constraint_bits_by_owner.get(&owner);
        let opposite_bits = self.constraint_bits_by_owner.get(&opposite_owner);
        for word_index in 0..self.ownerless_constraint_bits.len() {
            let mut remaining = self.ownerless_constraint_bits[word_index]
                | owner_bits.map_or(0, |bits| bits[word_index])
                | opposite_bits.map_or(0, |bits| bits[word_index]);
            while remaining != 0 {
                let bit = remaining.trailing_zeros() as usize;
                let index = word_index * u64::BITS as usize + bit;
                if self
                    .bounds
                    .get(index)
                    .and_then(Option::as_ref)
                    .is_some_and(|bounds| {
                        bounds.min_x <= query_max_x
                            && query_min_x <= bounds.max_x
                            && bounds.min_z <= query_max_z
                            && query_min_z <= bounds.max_z
                    })
                {
                    candidate_indices.push(index);
                }
                remaining &= remaining - 1;
            }
        }
    }

    pub(super) fn constraint(&self, index: usize) -> Option<&'a NodeRailConstraint> {
        self.constraints.get(index)
    }

    pub(super) fn constraint_points(&self, index: usize) -> Option<&[NodeOwnershipPointKey]> {
        self.point_keys.get(index).map(Vec::as_slice)
    }

    pub(super) fn constraint_covers_owned_edge(
        &self,
        index: usize,
        start: NodeOwnershipPointKey,
        end: NodeOwnershipPointKey,
    ) -> bool {
        let min_x = start.0.min(end.0);
        let min_z = start.1.min(end.1);
        let max_x = start.0.max(end.0);
        let max_z = start.1.max(end.1);
        let margin =
            (f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * SURFACE_XZ_KEY_SCALE).round() as i64;
        self.bounds
            .get(index)
            .and_then(Option::as_ref)
            .is_some_and(|bounds| {
                bounds.min_x.saturating_sub(margin) <= min_x
                    && max_x <= bounds.max_x.saturating_add(margin)
                    && bounds.min_z.saturating_sub(margin) <= min_z
                    && max_z <= bounds.max_z.saturating_add(margin)
            })
    }

    fn exact_owner_pair_point_contact_constraint_index_at_key(
        &self,
        key: NodeOwnershipPointKey,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        kind: NodeRailConstraintKind,
    ) -> Option<usize> {
        let (owner, opposite_owner) = canonical_owner_pair(owner, opposite_owner);
        self.point_contacts
            .get(&(key, kind, owner, opposite_owner))
            .copied()
    }
}

fn canonical_owner_pair(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> (NodeBandOwner, NodeBandOwner) {
    if owner <= opposite_owner {
        (owner, opposite_owner)
    } else {
        (opposite_owner, owner)
    }
}

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
    rail_constraint_index: &'a OwnedEdgeRailConstraintIndex<'a>,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
    candidate_indices: &[usize],
    intervals: &mut Vec<(i128, i128)>,
) -> Vec<OwnedEdgeSeamCandidate<'a>> {
    let mut candidates = Vec::new();
    append_matching_rail_constraints_for_owned_edge(
        start,
        end,
        rail_constraint_index,
        owner,
        opposite_owner,
        piece_kind,
        candidate_indices,
        intervals,
        &mut candidates,
    );
    let has_exact_owner_pair_source = candidates.iter().any(|candidate| {
        matches!(candidate, OwnedEdgeSeamCandidate::RailConstraint(constraint)
            if super::rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner))
    });
    if has_exact_owner_pair_source {
        candidates.retain(|candidate| {
            matches!(candidate, OwnedEdgeSeamCandidate::RailConstraint(constraint)
            if super::rail_constraint_owner_pair_matches_edge(
                constraint,
                owner,
                opposite_owner,
            ))
        });
    }

    if let Some(kind) = super::material_contact_kind_for_owned_edge(owner, opposite_owner) {
        let endpoint_pair_sources = materialized_endpoint_pair_constraint_indices_for_owned_edge(
            start,
            end,
            rail_constraint_index,
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
        rail_constraint_index,
        owner,
        opposite_owner,
        piece_kind,
        candidate_indices,
        intervals,
    )
    .map(|(constraint_index, kind)| {
        vec![OwnedEdgeSeamCandidate::SourceConstraint {
            constraint_index,
            kind,
        }]
    })
    .unwrap_or_default()
}

fn append_matching_rail_constraints_for_owned_edge<'a>(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraint_index: &'a OwnedEdgeRailConstraintIndex<'a>,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
    candidate_indices: &[usize],
    intervals: &mut Vec<(i128, i128)>,
    matches: &mut Vec<OwnedEdgeSeamCandidate<'a>>,
) {
    for &index in candidate_indices {
        if !rail_constraint_index.constraint_covers_owned_edge(index, start, end) {
            continue;
        }
        let (Some(constraint), Some(points)) = (
            rail_constraint_index.constraint(index),
            rail_constraint_index.constraint_points(index),
        ) else {
            continue;
        };
        if !super::rail_constraint_can_materialize_for_owned_edge(constraint, owner, opposite_owner)
            || !super::owned_edge_lies_on_prepared_rail_constraint(
                start,
                end,
                constraint,
                points,
                intervals,
                owner,
                opposite_owner,
                piece_kind,
            )
        {
            continue;
        }
        matches.push(OwnedEdgeSeamCandidate::RailConstraint(constraint));
    }
}

pub(in crate::simulation::network::surface::node::ownership) fn materialized_endpoint_pair_constraint_indices_for_owned_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraint_index: &OwnedEdgeRailConstraintIndex<'_>,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Vec<usize> {
    let Some(kind) = super::material_contact_kind_for_owned_edge(owner, opposite_owner) else {
        return Vec::new();
    };
    let Some(start_constraint_index) = source_authorized_point_contact_constraint_index_at_key(
        start,
        rail_constraint_index,
        owner,
        opposite_owner,
        kind,
        piece_kind,
    ) else {
        return Vec::new();
    };
    let Some(end_constraint_index) = source_authorized_point_contact_constraint_index_at_key(
        end,
        rail_constraint_index,
        owner,
        opposite_owner,
        kind,
        piece_kind,
    ) else {
        return Vec::new();
    };
    canonical_source_indices([start_constraint_index, end_constraint_index])
}

fn source_authorized_point_contact_constraint_index_at_key(
    key: NodeOwnershipPointKey,
    rail_constraint_index: &OwnedEdgeRailConstraintIndex<'_>,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    kind: NodeRailConstraintKind,
    _piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Option<usize> {
    rail_constraint_index.exact_owner_pair_point_contact_constraint_index_at_key(
        key,
        owner,
        opposite_owner,
        kind,
    )
}

fn materialized_source_constraint_for_owned_step_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraint_index: &OwnedEdgeRailConstraintIndex<'_>,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
    candidate_indices: &[usize],
    intervals: &mut Vec<(i128, i128)>,
) -> Option<(usize, NodeRailConstraintKind)> {
    let kind = super::material_contact_kind_for_owned_edge(owner, opposite_owner)?;
    candidate_indices
        .iter()
        .copied()
        .filter(|&index| rail_constraint_index.constraint_covers_owned_edge(index, start, end))
        .filter_map(|index| {
            Some((
                rail_constraint_index.constraint(index)?,
                rail_constraint_index.constraint_points(index)?,
            ))
        })
        .filter(|(constraint, _)| {
            constraint_applies_to_owner(constraint, owner)
                || constraint_applies_to_owner(constraint, opposite_owner)
        })
        .filter(|(constraint, points)| {
            super::owned_edge_lies_on_prepared_rail_constraint(
                start,
                end,
                constraint,
                points,
                intervals,
                owner,
                opposite_owner,
                piece_kind,
            )
        })
        .min_by_key(|(constraint, _)| {
            (
                constraint_is_material_transition(constraint),
                constraint.constraint_index,
            )
        })
        .map(|(constraint, _)| (constraint.constraint_index, kind))
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

        let constraint_index = OwnedEdgeRailConstraintIndex::new(&constraints);
        let mut candidate_indices = Vec::new();
        let mut intervals = Vec::new();
        constraint_index.candidate_indices_for_owned_edge(
            start,
            end,
            owner,
            final_opposite,
            &mut candidate_indices,
        );
        let candidates = materialized_seam_candidates_for_owned_edge(
            start,
            end,
            &constraint_index,
            owner,
            final_opposite,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &candidate_indices,
            &mut intervals,
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

        let constraint_index = OwnedEdgeRailConstraintIndex::new(&constraints);
        let candidates = materialized_endpoint_pair_constraint_indices_for_owned_edge(
            start,
            end,
            &constraint_index,
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
