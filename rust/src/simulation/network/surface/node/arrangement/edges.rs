//! Canonical arrangement boundary-edge ownership and source matching.

use super::super::keys::SurfaceXzSegmentKey;
use super::build::{canonical_sources, merge_sorted_unique};
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
            if let Some(opposite_owner) = opposite_owner {
                if owners_require_explicit_boundary_seam(pending.owner, opposite_owner) {
                    if source_constraints.is_empty() {
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
            let seam_source = source_constraints
                .first()
                .map(|constraint| constraint.seam_source)
                .unwrap_or_else(|| NodeSeamSource::for_owner(pending.owner));
            let constrains_shared_height = source_constraints
                .first()
                .is_some_and(|constraint| constraint.constrains_shared_height);
            let is_material_transition = source_constraints
                .first()
                .is_some_and(|constraint| constraint.is_material_transition);
            let source_constraint_indices = canonical_sources(
                source_constraints
                    .iter()
                    .map(|constraint| constraint.constraint_index),
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

    pub(super) fn edge_touches_key(
        &self,
        edge: &NodeArrangementEdge,
        key: NodeArrangementKey,
    ) -> bool {
        let Some(start) = self.vertices.get(edge.start.0) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0) else {
            return false;
        };
        start.key == key || end.key == key
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

fn owners_require_explicit_boundary_seam(a: NodeBandOwner, b: NodeBandOwner) -> bool {
    a.kind() != b.kind()
}
