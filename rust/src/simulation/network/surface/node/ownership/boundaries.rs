// SPDX-License-Identifier: GPL-2.0-only

//! Final owned-boundary extraction and arrangement export for node ownership.

use super::super::arrangement::{
    NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource, PreparedSeamConstraintCoverages,
    SeamConstraintCoverageScratch, seam_constraints_are_ambiguous,
};
use super::super::rails::NodeGeneratedContourClaimPriority;
#[cfg(test)]
use super::super::rails::NodeRailConstraint;
use super::super::{NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::rings::NodeOwnershipPointIndex;
use super::seams::{
    OwnedEdgeRailConstraintIndex, junctionn_unmaterialized_raised_step_authority_indices_for_edge,
    materialized_endpoint_pair_constraint_indices_for_owned_edge,
    owned_boundary_requires_explicit_seam, owned_source_constraints_for_edge,
    source_constraints_materialize_raised_step_authority,
};
use super::topology_keys::{
    OwnedRegionEdgeKey, canonical_source_indices, ownership_key_from_overlay_point,
    ownership_key_from_road_point, point_key_lies_exactly_on_segment, point_key_lies_on_segment,
    segment_parameter_key,
};
use super::{
    NodeBooleanOwnedRegion, NodeOwnedRegionArrangement, NodeOwnedRegionArrangementDiagnostic,
    NodeOwnedRegionArrangementEdge, NodeOwnedRegionArrangementKey,
};
use std::collections::BTreeMap;

impl NodeOwnedRegionArrangement {
    #[cfg(test)]
    pub(crate) fn from_owned_regions(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        regions: &[NodeBooleanOwnedRegion],
        footprint_shapes: &NodeOverlayShapes,
        rail_constraints: &[NodeRailConstraint],
    ) -> Self {
        let boundary_refs = owned_region_boundary_refs(regions, footprint_shapes);
        let rail_constraint_index = OwnedEdgeRailConstraintIndex::new(rail_constraints);
        Self::from_owned_regions_with_boundary_refs(
            node_id,
            piece_kind,
            regions,
            &boundary_refs,
            &rail_constraint_index,
        )
    }

    pub(super) fn from_owned_regions_with_boundary_refs(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        regions: &[NodeBooleanOwnedRegion],
        boundary_refs: &OwnedRegionBoundaryRefs,
        rail_constraint_index: &OwnedEdgeRailConstraintIndex<'_>,
    ) -> Self {
        let mut edges = Vec::new();
        let mut diagnostics = Vec::new();
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
                if edge_ref.region_index >= regions.len() {
                    continue;
                }
                let edge_neighbor = owned_region_edge_neighbor_for_ref(refs, *edge_ref);
                owned_source_constraints_for_edge(
                    edge_key.start,
                    edge_key.end,
                    &prepared_source_constraints[edge_ref.region_index],
                    &mut source_constraint_scratch,
                    &mut source_constraints,
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
                let endpoint_pair_source_constraint_indices = if source_constraints.is_empty() {
                    if let Some(opposite_owner) = opposite_owner {
                        canonical_source_indices(
                            endpoint_pair_constraint_indices_from_region_seams(
                                edge_key.start,
                                edge_key.end,
                                regions,
                                edge_ref.owner,
                                opposite_owner,
                            )
                            .into_iter()
                            .chain(
                                materialized_endpoint_pair_constraint_indices_for_owned_edge(
                                    edge_key.start,
                                    edge_key.end,
                                    rail_constraint_index,
                                    edge_ref.owner,
                                    opposite_owner,
                                    piece_kind,
                                ),
                            ),
                        )
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                let side_join_asphalt_boundary_indices = if source_constraints.is_empty()
                    && endpoint_pair_source_constraint_indices.is_empty()
                {
                    opposite_owner
                        .map(|opposite_owner| {
                            source_authorized_junction_side_join_asphalt_boundary_indices(
                                piece_kind,
                                edge_key.start,
                                edge_key.end,
                                regions,
                                &refs,
                                *edge_ref,
                                opposite_owner,
                            )
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                if let Some(opposite_owner) = opposite_owner {
                    if owned_boundary_requires_explicit_seam(edge_ref.owner, opposite_owner) {
                        rail_constraint_index.candidate_indices_for_owned_edge(
                            edge_key.start,
                            edge_key.end,
                            edge_ref.owner,
                            opposite_owner,
                            &mut rail_candidate_indices,
                        );
                        let source_constraint_indices =
                            junctionn_unmaterialized_raised_step_authority_indices_for_edge(
                                piece_kind,
                                edge_key.start,
                                edge_key.end,
                                rail_constraint_index,
                                &rail_candidate_indices,
                                &mut rail_coverage_intervals,
                                edge_ref.owner,
                                opposite_owner,
                            );
                        if !source_constraint_indices.is_empty()
                            && endpoint_pair_source_constraint_indices.is_empty()
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
                        } else if source_constraints.is_empty()
                            && endpoint_pair_source_constraint_indices.is_empty()
                            && side_join_asphalt_boundary_indices.is_empty()
                        {
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
                    .map(|constraint| constraint.seam_source)
                    .or_else(|| {
                        (!endpoint_pair_source_constraint_indices.is_empty()).then(|| {
                            NodeSeamSource::RaisedStepContact {
                                owner_index: edge_ref.owner.owner_index(),
                            }
                        })
                    })
                    .or_else(|| {
                        (!side_join_asphalt_boundary_indices.is_empty())
                            .then(|| NodeSeamSource::for_owner(edge_ref.owner))
                    })
                    .unwrap_or_else(|| NodeSeamSource::for_owner(edge_ref.owner));
                let source_constraint_indices = canonical_source_indices(
                    source_constraints
                        .iter()
                        .map(|constraint| constraint.constraint_index)
                        .chain(endpoint_pair_source_constraint_indices)
                        .chain(side_join_asphalt_boundary_indices),
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

fn endpoint_pair_constraint_indices_from_region_seams(
    start: (i64, i64),
    end: (i64, i64),
    regions: &[NodeBooleanOwnedRegion],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    let start_indices = source_authorized_region_seam_endpoint_constraint_indices(
        start,
        regions,
        owner,
        opposite_owner,
    );
    if start_indices.is_empty() {
        return Vec::new();
    }
    let end_indices = source_authorized_region_seam_endpoint_constraint_indices(
        end,
        regions,
        owner,
        opposite_owner,
    );
    if end_indices.is_empty() {
        return Vec::new();
    }
    canonical_source_indices(start_indices.into_iter().chain(end_indices))
}

fn source_authorized_junction_side_join_asphalt_boundary_indices(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    start: (i64, i64),
    end: (i64, i64),
    regions: &[NodeBooleanOwnedRegion],
    refs: &[OwnedRegionEdgeRef],
    edge_ref: OwnedRegionEdgeRef,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    if piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN
        || !owners_form_carriageway_sidewalk_boundary(edge_ref.owner, opposite_owner)
    {
        return Vec::new();
    }

    let Some(region) = regions.get(edge_ref.region_index) else {
        return Vec::new();
    };
    let Some(opposite_region) = refs
        .iter()
        .filter(|candidate| candidate.region_index != edge_ref.region_index)
        .filter(|candidate| candidate.owner == opposite_owner)
        .filter_map(|candidate| regions.get(candidate.region_index))
        .next()
    else {
        return Vec::new();
    };
    let (carriageway_region, sidewalk_region) =
        if region.owner.kind() == RoadSurfaceBandKind::Carriageway {
            (region, opposite_region)
        } else {
            (opposite_region, region)
        };
    if carriageway_region.claim_priority != NodeGeneratedContourClaimPriority::SideJoin
        || sidewalk_region.owner.kind() != RoadSurfaceBandKind::Sidewalk
        || !matches!(
            sidewalk_region.claim_priority,
            NodeGeneratedContourClaimPriority::MouthBand
                | NodeGeneratedContourClaimPriority::SideJoin
        )
    {
        return Vec::new();
    }

    let start_indices =
        endpoint_source_indices_from_region_seams(start, &sidewalk_region.seam_constraints);
    if start_indices.is_empty() {
        return Vec::new();
    }
    let end_indices =
        endpoint_source_indices_from_region_seams(end, &sidewalk_region.seam_constraints);
    if end_indices.is_empty() {
        return Vec::new();
    }
    canonical_source_indices(start_indices.into_iter().chain(end_indices))
}

fn owners_form_carriageway_sidewalk_boundary(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(
        (owner.kind(), opposite_owner.kind()),
        (
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::Sidewalk
        ) | (
            RoadSurfaceBandKind::Sidewalk,
            RoadSurfaceBandKind::Carriageway
        )
    )
}

fn endpoint_source_indices_from_region_seams(
    key: (i64, i64),
    constraints: &[NodeRegionSeamConstraint],
) -> Vec<usize> {
    constraints
        .iter()
        .filter(|constraint| region_seam_has_exact_endpoint_key(constraint, key))
        .map(|constraint| constraint.constraint_index)
        .collect()
}

fn source_authorized_region_seam_endpoint_constraint_indices(
    key: (i64, i64),
    regions: &[NodeBooleanOwnedRegion],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    regions
        .iter()
        .flat_map(|region| region.seam_constraints.iter())
        .filter(|constraint| constraint.is_material_transition)
        .filter(|constraint| region_seam_has_exact_endpoint_key(constraint, key))
        .filter(|constraint| {
            region_seam_authorizes_same_kind_handoff(constraint, owner, opposite_owner)
        })
        .map(|constraint| constraint.constraint_index)
        .collect()
}

fn region_seam_has_exact_endpoint_key(
    constraint: &NodeRegionSeamConstraint,
    key: (i64, i64),
) -> bool {
    ownership_key_from_road_point(constraint.start_xz) == key
        || ownership_key_from_road_point(constraint.end_xz) == key
}

fn region_seam_authorizes_same_kind_handoff(
    constraint: &NodeRegionSeamConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    let (Some(source_owner), Some(source_opposite_owner)) =
        (constraint.owner, constraint.opposite_owner)
    else {
        return false;
    };
    (source_owner == owner && source_opposite_owner.kind() == opposite_owner.kind())
        || (source_opposite_owner == owner && source_owner.kind() == opposite_owner.kind())
        || (source_owner == opposite_owner && source_opposite_owner.kind() == owner.kind())
        || (source_opposite_owner == opposite_owner && source_owner.kind() == owner.kind())
}

pub(super) struct OwnedRegionBoundaryRefs {
    pub(super) edges: BTreeMap<OwnedRegionEdgeKey, Vec<OwnedRegionEdgeRef>>,
}

impl OwnedRegionBoundaryRefs {
    pub(super) fn from_owned_regions(
        regions: &[NodeBooleanOwnedRegion],
        footprint_shapes: &NodeOverlayShapes,
    ) -> Self {
        let region_point_index = NodeOwnershipPointIndex::new(&owned_region_point_keys(regions));
        let footprint_point_index =
            NodeOwnershipPointIndex::new(&final_footprint_boundary_point_keys(footprint_shapes));
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
                    let points = noded_owned_region_boundary_edge_points(
                        start,
                        end,
                        &region_point_index,
                        &footprint_point_index,
                    );
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

        for refs in edges.values_mut() {
            refs.sort_unstable();
            refs.dedup();
        }
        Self { edges }
    }
}

#[cfg(test)]
pub(super) fn owned_region_boundary_refs(
    regions: &[NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) -> OwnedRegionBoundaryRefs {
    OwnedRegionBoundaryRefs::from_owned_regions(regions, footprint_shapes)
}

fn owned_region_point_keys(regions: &[NodeBooleanOwnedRegion]) -> Vec<(i64, i64)> {
    regions
        .iter()
        .flat_map(|region| region.shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(ownership_key_from_overlay_point)
        .collect()
}

fn final_footprint_boundary_point_keys(footprint_shapes: &NodeOverlayShapes) -> Vec<(i64, i64)> {
    let mut points = Vec::new();
    for contour in footprint_shapes
        .iter()
        .flat_map(|shape| shape.iter())
        .filter(|contour| contour.len() >= 2)
    {
        for index in 0..contour.len() {
            let start = ownership_key_from_overlay_point(contour[index]);
            let end = ownership_key_from_overlay_point(contour[(index + 1) % contour.len()]);
            if start != end {
                // The former edge scan only emitted endpoints which themselves lay on the owned
                // edge. Any such endpoint also proves that its source footprint edge overlaps, so
                // retaining the endpoint set preserves the same tolerant split-point semantics.
                points.extend([start, end]);
            }
        }
    }
    points
}

fn noded_owned_region_boundary_edge_points(
    start: (i64, i64),
    end: (i64, i64),
    region_point_index: &NodeOwnershipPointIndex,
    footprint_point_index: &NodeOwnershipPointIndex,
) -> Vec<(i64, i64)> {
    let mut split_points = region_point_index
        .candidates_between(start, end)
        .iter()
        .copied()
        .filter(|point| *point != start && *point != end)
        .filter(|point| point_key_lies_exactly_on_segment(*point, start, end))
        .collect::<Vec<_>>();
    split_points.extend(supported_final_footprint_boundary_points_for_edge(
        start,
        end,
        footprint_point_index,
    ));
    split_points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    split_points.dedup();

    let mut points = Vec::with_capacity(split_points.len() + 2);
    points.push(start);
    points.extend(split_points);
    points.push(end);
    points
}

fn supported_final_footprint_boundary_points_for_edge(
    start: (i64, i64),
    end: (i64, i64),
    footprint_point_index: &NodeOwnershipPointIndex,
) -> Vec<(i64, i64)> {
    let mut points = footprint_point_index
        .tolerant_candidates_between(start, end)
        .iter()
        .copied()
        .filter(|point| *point != start && *point != end)
        .filter(|point| point_key_lies_on_segment(*point, start, end))
        .collect::<Vec<_>>();
    points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    points.dedup();
    points
}

pub(super) fn owned_region_edge_neighbor_for_ref(
    refs: &[OwnedRegionEdgeRef],
    edge_ref: OwnedRegionEdgeRef,
) -> OwnedRegionEdgeNeighbor {
    let mut neighbor_owners = refs
        .iter()
        .map(|edge_ref| edge_ref.owner)
        .filter(|owner| *owner != edge_ref.owner);
    let Some(first_owner) = neighbor_owners.next() else {
        return OwnedRegionEdgeNeighbor::Exposed;
    };
    if neighbor_owners.all(|owner| owner == first_owner) {
        return OwnedRegionEdgeNeighbor::Unique {
            opposite_owner: first_owner,
        };
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_boundary_noding_keeps_exact_region_and_tolerant_footprint_semantics() {
        let region_point_index = NodeOwnershipPointIndex::new(&[(40, 0), (60, 1), (500, 0)]);
        let footprint_point_index = NodeOwnershipPointIndex::new(&[(20, 2), (80, 3), (500, 0)]);

        assert_eq!(
            noded_owned_region_boundary_edge_points(
                (0, 0),
                (100, 0),
                &region_point_index,
                &footprint_point_index,
            ),
            vec![(0, 0), (20, 2), (40, 0), (100, 0)]
        );
    }
}
