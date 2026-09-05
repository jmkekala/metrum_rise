// SPDX-License-Identifier: GPL-2.0-only

//! Region seam constraint emission from selected owned-edge candidates.

use super::*;

pub(super) fn push_candidate_region_seam_constraint(
    candidate: OwnedEdgeSeamCandidate<'_>,
    seams: &mut Vec<NodeRegionSeamConstraint>,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    match candidate {
        OwnedEdgeSeamCandidate::RailConstraint(constraint) => {
            push_materialized_region_seam_constraint(
                seams,
                constraint,
                owner,
                opposite_owner,
                start_xz,
                end_xz,
            );
        }
        OwnedEdgeSeamCandidate::EndpointPair {
            constraint_index,
            kind,
        }
        | OwnedEdgeSeamCandidate::SourceConstraint {
            constraint_index,
            kind,
        } => {
            push_materialized_endpoint_pair_region_seam_constraint(
                seams,
                constraint_index,
                kind,
                owner,
                opposite_owner,
                start_xz,
                end_xz,
            );
        }
    }
}

fn push_materialized_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    let (constraint_owner, constraint_opposite_owner) =
        materialized_constraint_owner_pair(constraint, owner, opposite_owner);
    let materialized_kind =
        materialized_constraint_kind_for_owned_edge(constraint, owner, opposite_owner);
    seams.push(NodeRegionSeamConstraint {
        constraint_index: constraint.constraint_index,
        seam_source: seam_source_from_materialized_constraint_kind(
            materialized_kind,
            owner,
            opposite_owner,
        ),
        owner: constraint_owner,
        opposite_owner: constraint_opposite_owner,
        constrains_shared_height: materialized_constraint_constrains_shared_height(
            constraint,
            owner,
            opposite_owner,
            start_xz,
            end_xz,
        ),
        is_material_transition: materialized_constraint_kind_is_material_transition(
            materialized_kind,
        ),
        start_xz,
        end_xz,
    });
}

fn push_materialized_endpoint_pair_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint_index: usize,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    seams.push(NodeRegionSeamConstraint {
        constraint_index,
        seam_source: seam_source_from_materialized_constraint_kind(kind, owner, opposite_owner),
        owner: Some(owner),
        opposite_owner: Some(opposite_owner),
        constrains_shared_height: false,
        is_material_transition: materialized_constraint_kind_is_material_transition(kind),
        start_xz,
        end_xz,
    });
}

fn materialized_constraint_owner_pair(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> (Option<NodeBandOwner>, Option<NodeBandOwner>) {
    if rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner) {
        (constraint.owner, constraint.opposite_owner)
    } else {
        (Some(owner), Some(opposite_owner))
    }
}

fn materialized_constraint_kind_for_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> NodeRailConstraintKind {
    if rail_constraint_band_contour_authorizes_owned_edge(constraint, owner, opposite_owner) {
        return material_contact_kind_for_owned_edge(owner, opposite_owner)
            .expect("band contour authorization requires a material contact kind");
    }
    if rail_constraint_can_materialize_for_owned_edge(constraint, owner, opposite_owner)
        && let Some(kind) = material_contact_kind_for_owned_edge(owner, opposite_owner)
    {
        return kind;
    }
    constraint.kind
}

fn seam_source_from_materialized_constraint_kind(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    _opposite_owner: NodeBandOwner,
) -> NodeSeamSource {
    match kind {
        NodeRailConstraintKind::RaisedStepContact => NodeSeamSource::RaisedStepContact {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::AsphaltBoundary { .. } => NodeSeamSource::AsphaltBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::FootprintSeam { .. }
        | NodeRailConstraintKind::FullRoadbedContour => NodeSeamSource::FootprintBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::BandBoundary { .. } => NodeSeamSource::for_owner(owner),
    }
}

fn materialized_constraint_kind_constrains_shared_height(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    match kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::AsphaltBoundary { .. } => true,
        NodeRailConstraintKind::RaisedStepContact => {
            raised_step_contact_constrains_shared_height(owner, opposite_owner)
        }
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => band_boundary_constrains_shared_height(left_kind, right_kind),
        _ => false,
    }
}

fn materialized_constraint_constrains_shared_height(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) -> bool {
    if ownership_key_from_road_point(start_xz) == ownership_key_from_road_point(end_xz) {
        return false;
    }
    let kind = materialized_constraint_kind_for_owned_edge(constraint, owner, opposite_owner);
    if kind == NodeRailConstraintKind::RaisedStepContact
        && !rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
    {
        return false;
    }
    materialized_constraint_kind_constrains_shared_height(kind, owner, opposite_owner)
}

fn materialized_constraint_kind_is_material_transition(kind: NodeRailConstraintKind) -> bool {
    match kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::RaisedStepContact
        | NodeRailConstraintKind::BandBoundary { .. } => true,
        NodeRailConstraintKind::AsphaltBoundary { adjacent_kind } => {
            adjacent_kind != RoadSurfaceBandKind::Carriageway
        }
        _ => false,
    }
}
