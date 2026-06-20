//! Canonical arrangement boundary-edge ownership and source matching.

use super::super::RoadSurfaceBandKind;
use super::super::keys::SurfaceXzSegmentKey;
use super::build::{
    canonical_sources, merge_sorted_unique,
    source_authorities_form_side_join_asphalt_sidewalk_split,
};
use super::regions::PendingArrangementRegion;
use super::seams::{
    NodeRegionSeamConstraint, NodeSeamSource, seam_constraint_can_source_edge_owner_pair,
    seam_constraint_covers_edge, seam_constraints_are_ambiguous,
};
use super::{
    NodeArrangement, NodeArrangementDiagnostic, NodeArrangementEdge, NodeArrangementEdgeId,
    NodeArrangementKey, NodeArrangementVertex, NodeArrangementVertexId, NodeBandHeightFieldId,
    NodeBandOwner,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct NodeArrangementEdgeOwner {
    pub(super) owner: NodeBandOwner,
    pub(super) height_field_id: NodeBandHeightFieldId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct NodeArrangementEdgeKey {
    pub(super) start: NodeArrangementKey,
    pub(super) end: NodeArrangementKey,
}

#[derive(Clone, Copy)]
pub(super) struct PendingArrangementEdge {
    pub(super) key: NodeArrangementEdgeKey,
    pub(super) start: NodeArrangementVertexId,
    pub(super) end: NodeArrangementVertexId,
}

pub(super) fn collect_pending_region_edge_support(
    pending_regions: &[PendingArrangementRegion],
    vertices: &[NodeArrangementVertex],
) -> (
    BTreeMap<NodeArrangementEdgeKey, Vec<NodeArrangementEdgeOwner>>,
    BTreeMap<NodeArrangementEdgeKey, usize>,
) {
    let mut edge_owners = BTreeMap::<NodeArrangementEdgeKey, Vec<NodeArrangementEdgeOwner>>::new();
    let mut edge_use_counts = BTreeMap::<NodeArrangementEdgeKey, usize>::new();

    for pending in pending_regions {
        let pending_edge_owner = NodeArrangementEdgeOwner {
            owner: pending.owner,
            height_field_id: pending.height_field_id,
        };
        for edge in pending.loop_edges(vertices) {
            *edge_use_counts.entry(edge.key).or_default() += 1;
            edge_owners
                .entry(edge.key)
                .and_modify(|owners| merge_sorted_unique(owners, vec![pending_edge_owner]))
                .or_insert_with(|| vec![pending_edge_owner]);
        }
    }

    (edge_owners, edge_use_counts)
}

pub(super) fn loop_edges(
    loop_vertices: &[NodeArrangementVertexId],
    vertices: &[NodeArrangementVertex],
) -> Vec<PendingArrangementEdge> {
    if loop_vertices.len() < 2 {
        return Vec::new();
    }
    (0..loop_vertices.len())
        .filter_map(|index| {
            let start = loop_vertices[index];
            let end = loop_vertices[(index + 1) % loop_vertices.len()];
            let start_key = vertices.get(start.0)?.key;
            let end_key = vertices.get(end.0)?.key;
            (start != end && start_key != end_key).then_some(PendingArrangementEdge {
                key: NodeArrangementEdgeKey::new(start_key, end_key),
                start,
                end,
            })
        })
        .collect()
}

impl NodeArrangementEdgeKey {
    fn new(a: NodeArrangementKey, b: NodeArrangementKey) -> Self {
        let segment = SurfaceXzSegmentKey::new(a.surface_key(), b.surface_key());
        Self {
            start: NodeArrangementKey::from_surface_key(segment.start()),
            end: NodeArrangementKey::from_surface_key(segment.end()),
        }
    }
}

impl NodeArrangement {
    pub(crate) fn push_edge(
        &mut self,
        start: NodeArrangementVertexId,
        end: NodeArrangementVertexId,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        opposite_owner: Option<NodeBandOwner>,
        opposite_height_field_id: Option<NodeBandHeightFieldId>,
        exposed_boundary: bool,
        constrains_shared_height: bool,
        is_material_transition: bool,
        seam_source: NodeSeamSource,
        source_constraint_indices: Vec<usize>,
    ) -> NodeArrangementEdgeId {
        let id = NodeArrangementEdgeId(self.edges.len());
        self.edges.push(NodeArrangementEdge {
            id,
            start,
            end,
            owner,
            height_field_id,
            opposite_owner,
            opposite_height_field_id,
            exposed_boundary,
            constrains_shared_height,
            is_material_transition,
            seam_source,
            source_constraint_indices,
        });
        id
    }

    pub(super) fn push_boundary_edges_for_pending_region(
        &mut self,
        pending: &PendingArrangementRegion,
        all_pending_regions: &[PendingArrangementRegion],
        edge_owners: &BTreeMap<NodeArrangementEdgeKey, Vec<NodeArrangementEdgeOwner>>,
        edge_use_counts: &BTreeMap<NodeArrangementEdgeKey, usize>,
    ) -> Vec<NodeArrangementEdgeId> {
        let mut boundary_edges = Vec::with_capacity(pending.edge_count());
        for edge in pending.loop_edges(&self.vertices) {
            let opposite = edge_owners.get(&edge.key).and_then(|owners| {
                owners
                    .iter()
                    .copied()
                    .find(|owner| owner.owner != pending.owner)
            });
            let opposite_owner = opposite.map(|owner| owner.owner);
            let opposite_height_field_id = opposite.map(|owner| owner.height_field_id);
            let source_constraints =
                source_constraints_for_edge(edge, &pending.seam_constraints, &self.vertices);
            let selected_source_constraint = selected_edge_source_constraint(&source_constraints);
            let endpoint_pair_source_constraint_indices = if source_constraints.is_empty() {
                opposite_owner
                    .map(|opposite_owner| {
                        endpoint_pair_constraint_indices_from_pending_region_seams(
                            edge,
                            all_pending_regions,
                            &self.vertices,
                            pending.owner,
                            opposite_owner,
                        )
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let has_source_authorized_side_join_boundary = source_constraints.is_empty()
                && endpoint_pair_source_constraint_indices.is_empty()
                && opposite_owner.is_some_and(|opposite_owner| {
                    edge_has_source_authorized_side_join_asphalt_sidewalk_boundary(
                        edge,
                        &self.vertices,
                        pending.owner,
                        opposite_owner,
                    )
                });
            if let Some(opposite_owner) = opposite_owner {
                if owners_require_explicit_boundary_seam(pending.owner, opposite_owner) {
                    if source_constraints.is_empty()
                        && endpoint_pair_source_constraint_indices.is_empty()
                        && !has_source_authorized_side_join_boundary
                    {
                        self.diagnostics
                            .push(NodeArrangementDiagnostic::MissingSeamConstraint {
                                region_index: pending.region_index,
                                owner: pending.owner,
                                opposite_owner,
                                start: edge.key.start,
                                end: edge.key.end,
                            });
                    } else if seam_constraints_are_ambiguous(&source_constraints) {
                        self.diagnostics
                            .push(NodeArrangementDiagnostic::AmbiguousSeamConstraint {
                                region_index: pending.region_index,
                                owner: pending.owner,
                                opposite_owner,
                                start: edge.key.start,
                                end: edge.key.end,
                            });
                    }
                }
            }
            let seam_source = selected_source_constraint
                .map(|constraint| constraint.seam_source)
                .or_else(|| {
                    (!endpoint_pair_source_constraint_indices.is_empty()).then(|| {
                        NodeSeamSource::RaisedStepContact {
                            owner_index: pending.owner.owner_index(),
                        }
                    })
                })
                .or_else(|| {
                    has_source_authorized_side_join_boundary
                        .then(|| NodeSeamSource::for_owner(pending.owner))
                })
                .unwrap_or_else(|| NodeSeamSource::for_owner(pending.owner));
            let constrains_shared_height = selected_source_constraint
                .is_some_and(|constraint| constraint.constrains_shared_height);
            let is_material_transition = selected_source_constraint
                .is_some_and(|constraint| constraint.is_material_transition)
                || !endpoint_pair_source_constraint_indices.is_empty()
                || has_source_authorized_side_join_boundary;
            let source_constraint_indices = canonical_sources(
                source_constraints
                    .iter()
                    .map(|constraint| constraint.constraint_index)
                    .chain(endpoint_pair_source_constraint_indices),
            );
            boundary_edges.push(self.push_edge(
                edge.start,
                edge.end,
                pending.owner,
                pending.height_field_id,
                opposite_owner,
                opposite_height_field_id,
                edge_use_counts.get(&edge.key).copied() == Some(1),
                constrains_shared_height,
                is_material_transition,
                seam_source,
                source_constraint_indices,
            ));
        }
        boundary_edges
    }

    pub(super) fn edge_has_applicable_material_source_constraint(
        &self,
        edge: &NodeArrangementEdge,
    ) -> bool {
        let Some(opposite_owner) = edge.opposite_owner else {
            return false;
        };
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return false;
        };
        self.regions.iter().any(|region| {
            region.seam_constraints.iter().any(|constraint| {
                constraint.is_material_transition
                    && edge
                        .source_constraint_indices
                        .contains(&constraint.constraint_index)
                    && seam_constraint_covers_edge(constraint, start, end)
                    && seam_constraint_can_source_edge_owner_pair(
                        constraint,
                        edge.owner,
                        Some(opposite_owner),
                    )
            })
        })
    }
}

fn source_constraints_for_edge<'a>(
    edge: PendingArrangementEdge,
    constraints: &'a [NodeRegionSeamConstraint],
    vertices: &[NodeArrangementVertex],
) -> Vec<&'a NodeRegionSeamConstraint> {
    let Some(start) = vertices.get(edge.start.0).map(|vertex| vertex.key) else {
        return Vec::new();
    };
    let Some(end) = vertices.get(edge.end.0).map(|vertex| vertex.key) else {
        return Vec::new();
    };
    let mut matches = constraints
        .iter()
        .filter(|constraint| seam_constraint_covers_edge(constraint, start, end))
        .collect::<Vec<_>>();
    matches.sort_by_key(|constraint| (constraint.priority_key(), constraint.constraint_index));
    matches.dedup_by_key(|constraint| constraint.constraint_index);
    matches
}

fn edge_has_source_authorized_side_join_asphalt_sidewalk_boundary(
    edge: PendingArrangementEdge,
    vertices: &[NodeArrangementVertex],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    if !owners_form_carriageway_sidewalk_boundary(owner, opposite_owner) {
        return false;
    }
    edge_endpoint_has_source_authorized_side_join_asphalt_sidewalk_split(
        edge.key.start,
        vertices,
        owner,
        opposite_owner,
    ) && edge_endpoint_has_source_authorized_side_join_asphalt_sidewalk_split(
        edge.key.end,
        vertices,
        owner,
        opposite_owner,
    )
}

fn edge_endpoint_has_source_authorized_side_join_asphalt_sidewalk_split(
    key: NodeArrangementKey,
    vertices: &[NodeArrangementVertex],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    let Some(owner_authority) = vertex_grade_authority_for_owner_at_key(vertices, key, owner)
    else {
        return false;
    };
    let Some(opposite_authority) =
        vertex_grade_authority_for_owner_at_key(vertices, key, opposite_owner)
    else {
        return false;
    };
    source_authorities_form_side_join_asphalt_sidewalk_split(owner_authority, opposite_authority)
}

fn vertex_grade_authority_for_owner_at_key(
    vertices: &[NodeArrangementVertex],
    key: NodeArrangementKey,
    owner: NodeBandOwner,
) -> Option<super::super::height::NodeGradeVertexAuthority> {
    vertices
        .iter()
        .find(|vertex| vertex.key() == key && vertex.owners().contains(&owner))
        .map(NodeArrangementVertex::grade_authority)
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

fn endpoint_pair_constraint_indices_from_pending_region_seams(
    edge: PendingArrangementEdge,
    regions: &[PendingArrangementRegion],
    vertices: &[NodeArrangementVertex],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    let Some(start) = vertices.get(edge.start.0).map(|vertex| vertex.key) else {
        return Vec::new();
    };
    let Some(end) = vertices.get(edge.end.0).map(|vertex| vertex.key) else {
        return Vec::new();
    };
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
    canonical_sources(start_indices.into_iter().chain(end_indices))
}

fn source_authorized_region_seam_endpoint_constraint_indices(
    key: NodeArrangementKey,
    regions: &[PendingArrangementRegion],
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
    key: NodeArrangementKey,
) -> bool {
    NodeArrangementKey::from_point(constraint.start_xz) == key
        || NodeArrangementKey::from_point(constraint.end_xz) == key
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

fn selected_edge_source_constraint<'a>(
    constraints: &'a [&'a NodeRegionSeamConstraint],
) -> Option<&'a NodeRegionSeamConstraint> {
    constraints.first().copied()
}

fn owners_require_explicit_boundary_seam(a: NodeBandOwner, b: NodeBandOwner) -> bool {
    a.kind() != b.kind()
}
