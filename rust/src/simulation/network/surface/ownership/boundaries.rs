//! Final owned-boundary extraction and arrangement export for node ownership.

use super::super::arrangement::{NodeBandOwner, NodeSeamSource, seam_constraints_are_ambiguous};
use super::super::rails::NodeRailConstraint;
use super::super::{NodeOverlayShapes, RoadSurfaceVisualNodePieceKind};
use super::rings::{noded_owned_region_edge_points, owned_region_global_points};
use super::seams::{
    junctionn_unmaterialized_raised_step_authority_indices_for_edge,
    owned_boundary_requires_explicit_seam, owned_source_constraints_for_edge,
    source_constraints_materialize_raised_step_authority,
};
use super::topology_keys::{
    OwnedRegionEdgeKey, canonical_source_indices, ownership_key_from_overlay_point,
};
use super::{
    NodeBooleanOwnedRegion, NodeOwnedRegionArrangement, NodeOwnedRegionArrangementDiagnostic,
    NodeOwnedRegionArrangementEdge, NodeOwnedRegionArrangementKey,
};
use std::collections::BTreeMap;

impl NodeOwnedRegionArrangement {
    pub(crate) fn from_owned_regions(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        regions: &[NodeBooleanOwnedRegion],
        footprint_shapes: &NodeOverlayShapes,
        rail_constraints: &[NodeRailConstraint],
    ) -> Self {
        let boundary_refs = owned_region_boundary_refs(regions, footprint_shapes);
        let mut edges = Vec::new();
        let mut diagnostics = Vec::new();

        for (edge_key, refs) in boundary_refs.edges {
            let refs = canonical_owned_region_edge_refs(&refs);
            for edge_ref in &refs {
                let Some(region) = regions.get(edge_ref.region_index) else {
                    continue;
                };
                let edge_neighbor = owned_region_edge_neighbor_for_ref(&refs, *edge_ref);
                let source_constraints = owned_source_constraints_for_edge(
                    edge_key.start,
                    edge_key.end,
                    &region.seam_constraints,
                );
                let start = NodeOwnedRegionArrangementKey::from_ownership_key(edge_key.start);
                let end = NodeOwnedRegionArrangementKey::from_ownership_key(edge_key.end);
                if let OwnedRegionEdgeNeighbor::Ambiguous {
                    opposite_owners, ..
                } = &edge_neighbor
                {
                    diagnostics.push(
                        NodeOwnedRegionArrangementDiagnostic::AmbiguousOwnedBoundaryEdge {
                            region_index: edge_ref.region_index,
                            owner: edge_ref.owner,
                            opposite_owners: opposite_owners.clone(),
                            start,
                            end,
                        },
                    );
                }
                let opposite_owner = match edge_neighbor {
                    OwnedRegionEdgeNeighbor::Unique { opposite_owner }
                    | OwnedRegionEdgeNeighbor::EquivalentSameKind { opposite_owner, .. } => {
                        Some(opposite_owner)
                    }
                    OwnedRegionEdgeNeighbor::Exposed
                    | OwnedRegionEdgeNeighbor::Ambiguous { .. } => None,
                };
                if let Some(opposite_owner) = opposite_owner {
                    if owned_boundary_requires_explicit_seam(edge_ref.owner, opposite_owner) {
                        let source_constraint_indices =
                            junctionn_unmaterialized_raised_step_authority_indices_for_edge(
                                piece_kind,
                                edge_key.start,
                                edge_key.end,
                                rail_constraints,
                                edge_ref.owner,
                                opposite_owner,
                            );
                        if !source_constraint_indices.is_empty()
                            && !source_constraints_materialize_raised_step_authority(
                                &source_constraints,
                                &source_constraint_indices,
                                edge_ref.owner,
                                opposite_owner,
                            )
                        {
                            diagnostics.push(
                                NodeOwnedRegionArrangementDiagnostic::UnmaterializedRaisedStepAuthority {
                                    region_index: edge_ref.region_index,
                                    owner: edge_ref.owner,
                                    opposite_owner,
                                    start,
                                    end,
                                    source_constraint_indices,
                                },
                            );
                        } else if source_constraints.is_empty() {
                            diagnostics.push(
                                NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
                                    region_index: edge_ref.region_index,
                                    owner: edge_ref.owner,
                                    opposite_owner,
                                    start,
                                    end,
                                },
                            );
                        } else if seam_constraints_are_ambiguous(&source_constraints) {
                            diagnostics.push(
                                NodeOwnedRegionArrangementDiagnostic::AmbiguousSeamConstraint {
                                    region_index: edge_ref.region_index,
                                    owner: edge_ref.owner,
                                    opposite_owner,
                                    start,
                                    end,
                                },
                            );
                        }
                    }
                }
                let seam_source = source_constraints
                    .first()
                    .map(|constraint| constraint.seam_source.clone())
                    .unwrap_or_else(|| NodeSeamSource::for_owner(edge_ref.owner));
                let source_constraint_indices = canonical_source_indices(
                    source_constraints
                        .iter()
                        .map(|constraint| constraint.constraint_index),
                );
                edges.push(NodeOwnedRegionArrangementEdge {
                    region_index: edge_ref.region_index,
                    owner: edge_ref.owner,
                    opposite_owner,
                    start: NodeOwnedRegionArrangementKey::from_ownership_key(edge_key.start),
                    end: NodeOwnedRegionArrangementKey::from_ownership_key(edge_key.end),
                    seam_source,
                    source_constraint_indices,
                });
            }
        }

        Self {
            node_id,
            piece_kind,
            region_count: regions.len(),
            edges,
            diagnostics,
        }
    }
}

pub(super) struct OwnedRegionBoundaryRefs {
    pub(super) edges: BTreeMap<OwnedRegionEdgeKey, Vec<OwnedRegionEdgeRef>>,
}

pub(super) fn owned_region_boundary_refs(
    regions: &[NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) -> OwnedRegionBoundaryRefs {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    let mut edges = BTreeMap::<OwnedRegionEdgeKey, Vec<OwnedRegionEdgeRef>>::new();
    for (region_index, region) in regions.iter().enumerate() {
        for contour in &region.shape {
            if contour.len() < 2 {
                continue;
            }
            for edge_index in 0..contour.len() {
                let start = ownership_key_from_overlay_point(contour[edge_index]);
                let end =
                    ownership_key_from_overlay_point(contour[(edge_index + 1) % contour.len()]);
                if start == end {
                    continue;
                }
                let points = noded_owned_region_edge_points(start, end, &global_points);
                for segment in points.windows(2) {
                    if segment[0] == segment[1] {
                        continue;
                    }
                    let edge_ref = OwnedRegionEdgeRef {
                        region_index,
                        owner: region.owner,
                    };
                    edges
                        .entry(OwnedRegionEdgeKey::new(segment[0], segment[1]))
                        .or_default()
                        .push(edge_ref);
                }
            }
        }
    }

    OwnedRegionBoundaryRefs { edges }
}

pub(super) fn canonical_owned_region_edge_refs(
    refs: &[OwnedRegionEdgeRef],
) -> Vec<OwnedRegionEdgeRef> {
    let mut refs = refs.to_vec();
    refs.sort_unstable();
    refs.dedup();
    refs
}

pub(super) fn owned_region_edge_neighbor_for_ref(
    refs: &[OwnedRegionEdgeRef],
    edge_ref: OwnedRegionEdgeRef,
) -> OwnedRegionEdgeNeighbor {
    let mut owners = refs
        .iter()
        .map(|edge_ref| edge_ref.owner)
        .filter(|owner| *owner != edge_ref.owner)
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    let material_owners = owners
        .iter()
        .copied()
        .filter(|owner| owner.kind() != edge_ref.owner.kind())
        .collect::<Vec<_>>();
    let candidate_owners = if material_owners.is_empty() {
        &owners
    } else {
        &material_owners
    };
    match candidate_owners.len() {
        0 => OwnedRegionEdgeNeighbor::Exposed,
        1 => OwnedRegionEdgeNeighbor::Unique {
            opposite_owner: candidate_owners[0],
        },
        _ if candidate_owners
            .iter()
            .all(|owner| owner.kind() == candidate_owners[0].kind()) =>
        {
            OwnedRegionEdgeNeighbor::EquivalentSameKind {
                opposite_owner: candidate_owners[0],
                equivalent_owners: candidate_owners.clone(),
            }
        }
        _ => OwnedRegionEdgeNeighbor::Ambiguous {
            opposite_owners: candidate_owners.clone(),
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct OwnedRegionEdgeRef {
    pub(super) region_index: usize,
    pub(super) owner: NodeBandOwner,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OwnedRegionEdgeNeighbor {
    Exposed,
    Unique {
        opposite_owner: NodeBandOwner,
    },
    EquivalentSameKind {
        opposite_owner: NodeBandOwner,
        equivalent_owners: Vec<NodeBandOwner>,
    },
    Ambiguous {
        opposite_owners: Vec<NodeBandOwner>,
    },
}
