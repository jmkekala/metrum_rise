// SPDX-License-Identifier: GPL-2.0-only

//! Seam materialization on final noded owned boundaries.

mod candidates;
mod emission;
mod matching;
mod sources;

#[cfg(test)]
use super::super::super::NodeOverlayShapes;
use super::super::super::arrangement::{
    NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource, PreparedSeamConstraintCoverages,
    SeamConstraintCoverageScratch,
};
use super::super::super::backend::RoadVec2;
use super::super::super::rails::{NodeRailConstraint, NodeRailConstraintKind};
use super::super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::super::NodeBooleanOwnedRegion;
#[cfg(test)]
use super::super::boundaries::owned_region_boundary_refs;
use super::super::boundaries::{
    OwnedRegionBoundaryRefs, OwnedRegionEdgeNeighbor, owned_region_edge_neighbor_for_ref,
};
use super::super::contact_semantics::{
    band_boundary_constrains_shared_height, owners_form_raised_step_contact,
    raised_step_contact_constrains_shared_height,
    raised_step_contact_requires_exact_constraint_span,
};
use super::super::reuse::NodeOwnershipBuildReuseContext;
use super::super::topology_keys::{
    NodeOwnershipPointKey, ownership_key_from_road_point, road_point_from_key,
};
use super::predicates::{
    canonicalize_seam_constraints, constraint_is_material_transition, constraint_is_point_contact,
    edge_lies_on_constraint_polyline_on_overlay_grid,
    edge_lies_on_single_constraint_segment_with_points,
};
pub(in crate::simulation::network::surface::node::ownership) use candidates::OwnedEdgeRailConstraintIndex;
pub(in crate::simulation::network::surface::node::ownership) use candidates::materialized_endpoint_pair_constraint_indices_for_owned_edge;
use candidates::{OwnedEdgeSeamCandidate, materialized_seam_candidates_for_owned_edge};
use emission::push_candidate_region_seam_constraint;

use matching::{
    material_contact_kind_for_owned_edge, owned_edge_lies_on_prepared_rail_constraint,
    rail_constraint_band_contour_authorizes_owned_edge,
    rail_constraint_can_materialize_for_owned_edge, rail_constraint_owner_pair_matches_edge,
};
pub(in crate::simulation::network::surface::node::ownership) use sources::{
    junctionn_unmaterialized_raised_step_authority_indices_for_edge,
    owned_boundary_requires_explicit_seam, owned_source_constraints_for_edge,
    source_constraints_materialize_raised_step_authority,
};

#[cfg(test)]
pub(in crate::simulation::network::surface::node::ownership) fn materialize_noded_region_seam_constraints(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_constraints: &[NodeRailConstraint],
    piece_kind: RoadSurfaceVisualNodePieceKind,
) {
    let boundary_refs = owned_region_boundary_refs(regions, footprint_shapes);
    let mut reuse = NodeOwnershipBuildReuseContext::new(None, rail_constraints);
    let rail_constraint_index = OwnedEdgeRailConstraintIndex::new(rail_constraints);
    materialize_noded_region_seam_constraints_from_boundary_refs_with_reuse(
        regions,
        &boundary_refs,
        &rail_constraint_index,
        piece_kind,
        &mut reuse,
    );
}

pub(in crate::simulation::network::surface::node::ownership) fn materialize_noded_region_seam_constraints_from_boundary_refs_with_reuse(
    regions: &mut [NodeBooleanOwnedRegion],
    boundary_refs: &OwnedRegionBoundaryRefs,
    rail_constraint_index: &OwnedEdgeRailConstraintIndex<'_>,
    piece_kind: RoadSurfaceVisualNodePieceKind,
    reuse: &mut NodeOwnershipBuildReuseContext<'_>,
) {
    let mut additions = vec![Vec::new(); regions.len()];
    let mut rail_candidate_indices = Vec::new();
    let mut rail_coverage_intervals = Vec::new();
    let mut source_constraint_scratch = SeamConstraintCoverageScratch::default();
    let mut source_constraints = Vec::new();
    let prepared_source_constraints = regions
        .iter()
        .map(|region| PreparedSeamConstraintCoverages::new(&region.seam_constraints))
        .collect::<Vec<_>>();
    for (edge_key, refs) in &boundary_refs.edges {
        for edge_ref in refs {
            let opposite_owner = match owned_region_edge_neighbor_for_ref(refs, *edge_ref) {
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
            owned_source_constraints_for_edge(
                edge_key.start,
                edge_key.end,
                &prepared_source_constraints[edge_ref.region_index],
                &mut source_constraint_scratch,
                &mut source_constraints,
            );
            rail_constraint_index.candidate_indices_for_owned_edge(
                edge_key.start,
                edge_key.end,
                edge_ref.owner,
                opposite_owner,
                &mut rail_candidate_indices,
            );
            let edge_additions = reuse.materialized_owned_edge_seams(
                edge_key.start,
                edge_key.end,
                edge_ref.owner,
                opposite_owner,
                piece_kind,
                &source_constraints,
                rail_candidate_indices
                    .iter()
                    .filter_map(|&index| rail_constraint_index.constraint(index)),
                || {
                    let mut edge_additions = Vec::new();
                    for candidate in materialized_seam_candidates_for_owned_edge(
                        edge_key.start,
                        edge_key.end,
                        rail_constraint_index,
                        edge_ref.owner,
                        opposite_owner,
                        piece_kind,
                        &rail_candidate_indices,
                        &mut rail_coverage_intervals,
                    ) {
                        push_candidate_region_seam_constraint(
                            candidate,
                            &mut edge_additions,
                            region.owner,
                            opposite_owner,
                            start_xz,
                            end_xz,
                        );
                    }
                    if edge_ref.owner.kind() == opposite_owner.kind()
                        && edge_ref.owner != opposite_owner
                        && let Some(source) = source_constraints.first()
                    {
                        edge_additions.push(NodeRegionSeamConstraint {
                            constraint_index: source.constraint_index,
                            seam_source: source.seam_source,
                            owner: Some(edge_ref.owner),
                            opposite_owner: Some(opposite_owner),
                            constrains_shared_height: false,
                            is_material_transition: true,
                            start_xz,
                            end_xz,
                        });
                    }
                    edge_additions
                },
            );
            additions[edge_ref.region_index].extend(edge_additions);
        }
    }
    drop(prepared_source_constraints);
    for (region, mut seam_additions) in regions.iter_mut().zip(additions) {
        region.seam_constraints.append(&mut seam_additions);
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }
}
