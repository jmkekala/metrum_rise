//! Seam materialization on final noded owned boundaries.

mod candidates;

use super::super::super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource};
use super::super::super::backend::RoadVec2;
use super::super::super::rails::{NodeRailConstraint, NodeRailConstraintKind};
use super::super::super::{NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::super::NodeBooleanOwnedRegion;
use super::super::boundaries::{
    OwnedRegionEdgeNeighbor, canonical_owned_region_edge_refs, owned_region_boundary_refs,
    owned_region_edge_neighbor_for_ref,
};
use super::super::contact_semantics::{
    owners_form_raised_step_contact, raised_step_contact_constrains_shared_height,
    raised_step_contact_requires_exact_constraint_span,
};
use super::super::topology_keys::{
    NodeOwnershipPointKey, ownership_key_from_road_point, point_key_lies_on_segment,
    road_point_from_key,
};
use super::predicates::{
    canonicalize_seam_constraints, constraint_is_material_transition, constraint_is_point_contact,
    edge_lies_on_constraint_polyline_on_overlay_grid, edge_lies_on_single_constraint_segment,
};
use candidates::{OwnedEdgeSeamCandidate, materialized_seam_candidates_for_owned_edge};

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
        constrains_shared_height: materialized_constraint_kind_constrains_shared_height(
            kind,
            owner,
            opposite_owner,
        ),
        is_material_transition: materialized_constraint_kind_is_material_transition(kind),
        start_xz,
        end_xz,
    });
}

pub(in crate::simulation::network::surface::node::ownership) fn materialize_noded_region_seam_constraints(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_constraints: &[NodeRailConstraint],
    piece_kind: RoadSurfaceVisualNodePieceKind,
) {
    let boundary_refs = owned_region_boundary_refs(regions, footprint_shapes);
    let mut additions = vec![Vec::new(); regions.len()];
    for (edge_key, refs) in boundary_refs.edges {
        let refs = canonical_owned_region_edge_refs(&refs);
        for edge_ref in &refs {
            let opposite_owner = match owned_region_edge_neighbor_for_ref(&refs, *edge_ref) {
                OwnedRegionEdgeNeighbor::Unique { opposite_owner }
                | OwnedRegionEdgeNeighbor::EquivalentSameKind { opposite_owner, .. } => {
                    opposite_owner
                }
                OwnedRegionEdgeNeighbor::Ambiguous { .. } => {
                    continue;
                }
                OwnedRegionEdgeNeighbor::Exposed => {
                    continue;
                }
            };
            let Some(region) = regions.get(edge_ref.region_index) else {
                continue;
            };
            let start_xz = road_point_from_key(edge_key.start);
            let end_xz = road_point_from_key(edge_key.end);
            for candidate in materialized_seam_candidates_for_owned_edge(
                edge_key.start,
                edge_key.end,
                rail_constraints,
                edge_ref.owner,
                opposite_owner,
                piece_kind,
            ) {
                push_candidate_region_seam_constraint(
                    candidate,
                    &mut additions[edge_ref.region_index],
                    region.owner,
                    opposite_owner,
                    start_xz,
                    end_xz,
                );
            }
        }
    }
    for (region, mut seam_additions) in regions.iter_mut().zip(additions) {
        region.seam_constraints.append(&mut seam_additions);
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }
}

fn push_candidate_region_seam_constraint(
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

fn rail_constraint_owner_pair_matches_edge(
    constraint: &NodeRailConstraint,
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

fn rail_constraint_can_materialize_for_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        || rail_constraint_owner_kinds_authorize_owned_edge(constraint, owner, opposite_owner)
        || rail_constraint_band_contour_authorizes_owned_edge(constraint, owner, opposite_owner)
        || rail_constraint_role_matches_owned_edge(constraint, owner, opposite_owner)
}

fn rail_constraint_band_contour_authorizes_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    let NodeRailConstraintKind::BandContour { kind } = constraint.kind else {
        return false;
    };
    if material_contact_kind_for_owned_edge(owner, opposite_owner).is_none() {
        return false;
    }
    if kind != owner.kind() && kind != opposite_owner.kind() {
        return false;
    }
    constraint.owner.is_none_or(|constraint_owner| {
        constraint_owner == owner || constraint_owner == opposite_owner
    })
}

fn rail_constraint_owner_kinds_authorize_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    if !constraint_is_material_transition(constraint) {
        return false;
    }
    let Some((constraint_owner, constraint_opposite_owner)) =
        constraint.owner.zip(constraint.opposite_owner)
    else {
        return false;
    };
    if ![constraint_owner, constraint_opposite_owner]
        .into_iter()
        .any(|constraint_owner| constraint_owner == owner || constraint_owner == opposite_owner)
    {
        return false;
    }
    owner_sets_match_by_kind(
        owner,
        opposite_owner,
        constraint_owner,
        constraint_opposite_owner,
    )
}

fn owner_sets_match_by_kind(
    left_owner: NodeBandOwner,
    left_opposite_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    right_opposite_owner: NodeBandOwner,
) -> bool {
    (left_owner.kind() == right_owner.kind()
        && left_opposite_owner.kind() == right_opposite_owner.kind())
        || (left_owner.kind() == right_opposite_owner.kind()
            && left_opposite_owner.kind() == right_owner.kind())
}

fn rail_constraint_role_matches_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    if constraint.owner.zip(constraint.opposite_owner).is_some() {
        return false;
    }
    let Some(role_owner) = constraint.owner.or(constraint.opposite_owner) else {
        return false;
    };
    if role_owner != owner && role_owner != opposite_owner {
        return false;
    }
    match constraint.kind {
        NodeRailConstraintKind::RaisedStepContact => {
            owners_form_raised_step_contact(owner, opposite_owner)
        }
        _ => false,
    }
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
    constraint.kind
}

fn material_contact_kind_for_owned_edge(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<NodeRailConstraintKind> {
    owners_form_raised_step_contact(owner, opposite_owner)
        .then_some(NodeRailConstraintKind::RaisedStepContact)
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
        _ => false,
    }
}

fn materialized_constraint_constrains_shared_height(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
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

fn owned_edge_lies_on_rail_constraint(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> bool {
    if start == end || constraint.points_xz.len() < 2 {
        return false;
    }
    if edge_lies_on_single_constraint_segment(start, end, constraint) {
        return true;
    }
    if matches!(constraint.kind, NodeRailConstraintKind::BandContour { .. }) {
        return false;
    }
    let exact_owner_pair =
        rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner);
    if materialized_edge_requires_exact_constraint_span(constraint, owner, opposite_owner) {
        if exact_owner_pair && piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN {
            return edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint);
        }
        if !exact_owner_pair
            || (constraint.source_boundary_index.is_some()
                && piece_kind != RoadSurfaceVisualNodePieceKind::Terminal)
        {
            return false;
        }
    }
    if constraint.kind == NodeRailConstraintKind::RaisedStepContact
        && exact_owner_pair
        && piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN
    {
        return edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint);
    }
    matches!(
        piece_kind,
        RoadSurfaceVisualNodePieceKind::Bend | RoadSurfaceVisualNodePieceKind::Terminal
    ) && edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint)
}

fn materialized_edge_requires_exact_constraint_span(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(constraint.kind, NodeRailConstraintKind::RaisedStepContact)
        && raised_step_contact_requires_exact_constraint_span(owner, opposite_owner)
}

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
