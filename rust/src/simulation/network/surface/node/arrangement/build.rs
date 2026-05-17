//! Construction pipeline for canonical node arrangements.

use super::super::backend::RoadVec2;
use super::super::grade::{NodeGradeCarrierDecision, NodeGradeVertexAuthority};
use super::super::height::NodeHeightSolution;
use super::super::keys::SurfaceHeightMmKey;
use super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::edges::collect_pending_region_edge_support;
use super::model::{NodeArrangementHeightKey, NodeArrangementVertexContextKey};
use super::seams::{
    NodeSeamSource, owners_for_material_seam_constraint, seam_constraint_touches_key,
};
use super::{
    NodeArrangement, NodeArrangementError, NodeArrangementKey, NodeArrangementVertex,
    NodeArrangementVertexId, NodeBandHeightFieldId, NodeBandOwner,
};
use std::collections::{BTreeMap, BTreeSet};

impl NodeArrangement {
    #[cfg(test)]
    pub(crate) fn insert_vertex(
        &mut self,
        point_xz: RoadVec2,
        height_m: f64,
        owners: impl IntoIterator<Item = NodeBandOwner>,
        height_field_id: NodeBandHeightFieldId,
        seam_sources: impl IntoIterator<Item = NodeSeamSource>,
    ) -> Result<NodeArrangementVertexId, NodeArrangementError> {
        let key = NodeArrangementKey::from_point(point_xz);
        let owners = canonical_non_empty_owners(key, owners)?;
        let owner = owners[0];
        let grade_authority = NodeGradeVertexAuthority::new(
            point_xz,
            height_m,
            owner,
            height_field_id,
            NodeGradeCarrierDecision::SourceCarrier { authority: None },
        );
        self.insert_vertex_with_grade_authority(
            point_xz,
            height_m,
            owners,
            height_field_id,
            seam_sources,
            grade_authority,
        )
    }

    pub(super) fn insert_vertex_with_grade_authority(
        &mut self,
        point_xz: RoadVec2,
        height_m: f64,
        owners: impl IntoIterator<Item = NodeBandOwner>,
        height_field_id: NodeBandHeightFieldId,
        seam_sources: impl IntoIterator<Item = NodeSeamSource>,
        grade_authority: NodeGradeVertexAuthority,
    ) -> Result<NodeArrangementVertexId, NodeArrangementError> {
        let key = NodeArrangementKey::from_point(point_xz);
        let height_key = NodeArrangementHeightKey(quantize_height_m(height_m));
        let owners = canonical_non_empty_owners(key, owners)?;
        let seam_sources = canonical_sources(seam_sources);
        let context_key = NodeArrangementVertexContextKey {
            position: key,
            owners: owners.clone(),
            height_field_id,
        };

        if let Some(conflict) =
            self.height_owner_conflict_at_key(key, height_key, &owners, height_field_id)
        {
            return Err(NodeArrangementError::DuplicateVertexHeightConflict {
                key,
                existing_height_mm: conflict.0,
                incoming_height_mm: height_key.0,
            });
        }

        if let Some(existing_id) = self.vertex_by_context_key.get(&context_key).copied() {
            let existing = &mut self.vertices[existing_id.0];
            if existing.height_key != height_key {
                return Err(NodeArrangementError::DuplicateVertexHeightConflict {
                    key,
                    existing_height_mm: existing.height_key.0,
                    incoming_height_mm: height_key.0,
                });
            }
            existing.grade_authority =
                merged_node_grade_authority(existing.grade_authority, grade_authority);
            merge_sorted_unique(&mut existing.owners, owners);
            merge_sorted_unique(&mut existing.seam_sources, seam_sources);
            return Ok(existing_id);
        }

        Ok(self.push_vertex(
            key,
            context_key,
            point_xz,
            height_m,
            height_key,
            owners,
            height_field_id,
            seam_sources,
            grade_authority,
        ))
    }

    fn height_owner_conflict_at_key(
        &self,
        key: NodeArrangementKey,
        height_key: NodeArrangementHeightKey,
        owners: &[NodeBandOwner],
        height_field_id: NodeBandHeightFieldId,
    ) -> Option<NodeArrangementHeightKey> {
        self.vertices
            .iter()
            .find(|vertex| {
                vertex.key == key
                    && vertex.height_key != height_key
                    && (vertex.height_field_id == height_field_id
                        || owners_overlap(&vertex.owners, owners))
            })
            .map(|vertex| vertex.height_key)
    }

    fn push_vertex(
        &mut self,
        key: NodeArrangementKey,
        context_key: NodeArrangementVertexContextKey,
        point_xz: RoadVec2,
        height_m: f64,
        height_key: NodeArrangementHeightKey,
        owners: Vec<NodeBandOwner>,
        height_field_id: NodeBandHeightFieldId,
        seam_sources: Vec<NodeSeamSource>,
        grade_authority: NodeGradeVertexAuthority,
    ) -> NodeArrangementVertexId {
        let id = NodeArrangementVertexId(self.vertices.len());
        self.vertices.push(NodeArrangementVertex {
            id,
            key,
            point_xz,
            height_m,
            height_key,
            owners,
            height_field_id,
            seam_sources,
            grade_authority,
        });
        self.vertex_by_context_key.insert(context_key, id);
        id
    }

    pub(crate) fn from_height_solution(
        heights: &NodeHeightSolution,
    ) -> Result<Self, NodeArrangementError> {
        let mut arrangement = Self::new(heights.node_id, heights.piece_kind);
        let mut pending_regions = Vec::with_capacity(heights.regions.len());

        for (region_index, height_region) in heights.regions.iter().enumerate() {
            let pending = arrangement.pending_region(region_index, height_region)?;
            pending_regions.push(pending);
        }

        let (edge_owners, edge_use_counts) =
            collect_pending_region_edge_support(&pending_regions, &arrangement.vertices);

        for pending in pending_regions {
            let boundary_edges = arrangement.push_boundary_edges_for_pending_region(
                &pending,
                &edge_owners,
                &edge_use_counts,
            );
            arrangement.push_region(
                pending.owner,
                pending.height_field_id,
                pending.outer_loop,
                pending.holes,
                boundary_edges,
                pending.area_m2,
                pending.seam_constraints,
            );
        }

        arrangement.reject_implicit_material_height_conflicts()?;
        Ok(arrangement)
    }

    pub(super) fn reject_implicit_material_height_conflicts(
        &self,
    ) -> Result<(), NodeArrangementError> {
        let mut vertices_by_key =
            BTreeMap::<NodeArrangementKey, Vec<NodeArrangementVertexId>>::new();
        for vertex in &self.vertices {
            vertices_by_key
                .entry(vertex.key)
                .or_default()
                .push(vertex.id);
        }

        for (key, vertex_ids) in vertices_by_key {
            for left_index in 0..vertex_ids.len() {
                for right_index in left_index + 1..vertex_ids.len() {
                    let left = &self.vertices[vertex_ids[left_index].0];
                    let right = &self.vertices[vertex_ids[right_index].0];
                    if left.height_key == right.height_key
                        || left.height_field_id == right.height_field_id
                        || owners_share_band_kind(&left.owners, &right.owners)
                    {
                        continue;
                    }
                    if !self.has_explicit_material_seam_at_key_between(
                        key,
                        &left.owners,
                        &right.owners,
                    ) && !self.has_explicit_material_seam_endpoint_path_at_key_between(
                        key,
                        &left.owners,
                        &right.owners,
                    ) {
                        return Err(NodeArrangementError::DuplicateVertexHeightConflict {
                            key,
                            existing_height_mm: left.height_key.0,
                            incoming_height_mm: right.height_key.0,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn has_explicit_material_seam_endpoint_path_at_key_between(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        let adjacency = self.material_seam_endpoint_owner_adjacency_at_key(key);
        if adjacency.is_empty() {
            return false;
        }

        let right_owners = right_owners.iter().copied().collect::<BTreeSet<_>>();
        let mut visited = BTreeSet::new();
        let mut pending = left_owners.to_vec();
        while let Some(owner) = pending.pop() {
            if !visited.insert(owner) {
                continue;
            }
            if right_owners.contains(&owner) {
                return true;
            }
            if let Some(neighbors) = adjacency.get(&owner) {
                pending.extend(neighbors.iter().copied());
            }
        }
        false
    }

    fn material_seam_endpoint_owner_adjacency_at_key(
        &self,
        key: NodeArrangementKey,
    ) -> BTreeMap<NodeBandOwner, BTreeSet<NodeBandOwner>> {
        let mut owners_by_constraint = BTreeMap::<usize, Vec<NodeBandOwner>>::new();
        let mut owners_by_kind = BTreeMap::<RoadSurfaceBandKind, Vec<NodeBandOwner>>::new();
        for region in &self.regions {
            let mut region_touches_key = false;
            for constraint in &region.seam_constraints {
                if !seam_constraint_touches_key(constraint, key) {
                    continue;
                }
                region_touches_key = true;
                if !constraint.is_material_transition {
                    continue;
                }
                let owners = owners_for_material_seam_constraint(constraint, region.owner);
                merge_sorted_unique(
                    owners_by_constraint
                        .entry(constraint.constraint_index)
                        .or_default(),
                    owners,
                );
            }
            if region_touches_key {
                owners_by_kind
                    .entry(region.owner.kind())
                    .or_default()
                    .push(region.owner);
            }
        }

        let mut adjacency = BTreeMap::<NodeBandOwner, BTreeSet<NodeBandOwner>>::new();
        for mut owners in owners_by_constraint.into_values() {
            owners.sort_unstable();
            owners.dedup();
            for left_index in 0..owners.len() {
                for right_index in left_index + 1..owners.len() {
                    let left = owners[left_index];
                    let right = owners[right_index];
                    adjacency.entry(left).or_default().insert(right);
                    adjacency.entry(right).or_default().insert(left);
                }
            }
        }
        for mut owners in owners_by_kind.into_values() {
            owners.sort_unstable();
            owners.dedup();
            for left_index in 0..owners.len() {
                for right_index in left_index + 1..owners.len() {
                    let left = owners[left_index];
                    let right = owners[right_index];
                    adjacency.entry(left).or_default().insert(right);
                    adjacency.entry(right).or_default().insert(left);
                }
            }
        }
        adjacency
    }

    fn has_explicit_material_seam_at_key_between(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        self.edges.iter().any(|edge| {
            edge.is_material_transition
                && !edge.source_constraint_indices.is_empty()
                && self.edge_has_applicable_material_source_constraint(edge)
                && (self.piece_kind != RoadSurfaceVisualNodePieceKind::Terminal
                    || !self.edge_has_owner_pair_source_constraint(edge))
                && self.edge_touches_key(edge, key)
                && edge.opposite_owner.is_some_and(|opposite_owner| {
                    owner_sets_match_edge(left_owners, right_owners, edge.owner, opposite_owner)
                })
        })
    }
}

pub(super) fn quantize_height_m(value_m: f64) -> i64 {
    SurfaceHeightMmKey::from_m_f64(value_m).as_i64()
}

fn canonical_non_empty_owners(
    key: NodeArrangementKey,
    owners: impl IntoIterator<Item = NodeBandOwner>,
) -> Result<Vec<NodeBandOwner>, NodeArrangementError> {
    let owners = canonical_sources(owners);
    if owners.is_empty() {
        return Err(NodeArrangementError::EmptyOwnerSet { key });
    }
    Ok(owners)
}

fn owners_share_band_kind(a: &[NodeBandOwner], b: &[NodeBandOwner]) -> bool {
    a.iter()
        .any(|a_owner| b.iter().any(|b_owner| a_owner.kind == b_owner.kind))
}

fn owners_overlap(a: &[NodeBandOwner], b: &[NodeBandOwner]) -> bool {
    a.iter()
        .any(|a_owner| b.iter().any(|b_owner| a_owner == b_owner))
}

fn merged_node_grade_authority(
    existing: NodeGradeVertexAuthority,
    incoming: NodeGradeVertexAuthority,
) -> NodeGradeVertexAuthority {
    if node_grade_decision_rank(incoming.decision) < node_grade_decision_rank(existing.decision) {
        incoming
    } else {
        existing
    }
}

fn node_grade_decision_rank(decision: NodeGradeCarrierDecision) -> u8 {
    match decision {
        NodeGradeCarrierDecision::ExplicitMaterialSeam => 0,
        NodeGradeCarrierDecision::SameMaterialSeam => 1,
        NodeGradeCarrierDecision::SameMaterialSharedEdge => 2,
        NodeGradeCarrierDecision::SameMaterialVertex => 3,
        NodeGradeCarrierDecision::SameOwnerCanonicalVertex => 4,
        NodeGradeCarrierDecision::SourceCarrier { .. } => 5,
    }
}

fn owner_sets_match_edge(
    left_owners: &[NodeBandOwner],
    right_owners: &[NodeBandOwner],
    edge_owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    (left_owners.contains(&edge_owner) && right_owners.contains(&opposite_owner))
        || (left_owners.contains(&opposite_owner) && right_owners.contains(&edge_owner))
}

pub(super) fn canonical_sources<T>(sources: impl IntoIterator<Item = T>) -> Vec<T>
where
    T: Ord,
{
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources
}

pub(super) fn merge_sorted_unique<T>(target: &mut Vec<T>, incoming: Vec<T>)
where
    T: Ord,
{
    if incoming.is_empty() {
        return;
    }
    target.extend(incoming);
    target.sort();
    target.dedup();
}
