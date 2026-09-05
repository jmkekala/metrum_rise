//! Construction pipeline for canonical node arrangements.

use super::super::backend::RoadVec2;
use super::super::band_semantics::raised_step_band_rank;
use super::super::height::{
    NodeGradeCarrierDecision, NodeGradeVertexAuthority, NodeHeightCarrierProvenanceKey,
    NodeHeightSolution,
};
use super::super::keys::SurfaceHeightMmKey;
use super::super::ownership::NodeCarrierProvenanceOrigin;
use super::super::rails::{NodeGeneratedContourClaimPriority, NodeGeneratedContourPurpose};
use super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::edges::collect_pending_region_edge_support;
use super::model::{NodeArrangementHeightKey, NodeArrangementVertexContextKey};
use super::seams::{NodeRegionSeamConstraint, NodeSeamSource, owners_for_material_seam_constraint};
use super::steps::{
    NodeExplicitVerticalStepSegment, explicit_vertical_step_segments_authorize_height_side_at_key,
    owners_form_explicit_vertical_step_pair,
};
use super::{
    NodeArrangement, NodeArrangementBuildProfile, NodeArrangementError, NodeArrangementKey,
    NodeArrangementVertex, NodeArrangementVertexId, NodeBandHeightFieldId, NodeBandOwner,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

fn elapsed_profile_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

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
            source_provenance: grade_authority.source_provenance,
        };

        if let Some(conflict) = self.height_owner_conflict_at_key(
            key,
            height_key,
            &owners,
            height_field_id,
            grade_authority,
        ) {
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
        grade_authority: NodeGradeVertexAuthority,
    ) -> Option<NodeArrangementHeightKey> {
        self.vertices
            .iter()
            .find(|vertex| {
                vertex.key == key
                    && vertex.height_key != height_key
                    && (vertex.height_field_id == height_field_id
                        || owners_overlap(&vertex.owners, owners))
                    && !grade_authorities_have_distinct_source_carrier_provenance(
                        vertex.grade_authority,
                        grade_authority,
                    )
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

    #[cfg(test)]
    pub(crate) fn from_height_solution(
        heights: &NodeHeightSolution,
    ) -> Result<Self, NodeArrangementError> {
        Self::from_height_solution_with_profile(heights, false)
            .map(|(arrangement, _, _)| arrangement)
    }

    pub(crate) fn from_height_solution_with_profile(
        heights: &NodeHeightSolution,
        profile_enabled: bool,
    ) -> Result<
        (
            Self,
            NodeArrangementBuildProfile,
            Option<Vec<NodeExplicitVerticalStepSegment>>,
        ),
        NodeArrangementError,
    > {
        let total_start = profile_enabled.then(Instant::now);
        let mut profile = NodeArrangementBuildProfile {
            height_regions: heights.regions.len(),
            ..NodeArrangementBuildProfile::default()
        };
        let mut arrangement = Self::new(heights.node_id, heights.piece_kind);
        let mut pending_regions = Vec::with_capacity(heights.regions.len());

        let pending_start = profile_enabled.then(Instant::now);
        for (region_index, height_region) in heights.regions.iter().enumerate() {
            let pending = arrangement.pending_region(region_index, height_region)?;
            pending_regions.push(pending);
        }
        profile.pending_regions_ms = elapsed_profile_ms(pending_start);
        profile.pending_edges_before = pending_regions
            .iter()
            .map(|pending| pending.edge_count())
            .sum();

        let noding_start = profile_enabled.then(Instant::now);
        arrangement.node_pending_region_edges(&mut pending_regions)?;
        profile.noding_ms = elapsed_profile_ms(noding_start);
        profile.pending_edges_after = pending_regions
            .iter()
            .map(|pending| pending.edge_count())
            .sum();

        let edge_support_start = profile_enabled.then(Instant::now);
        let (edge_owners, edge_use_counts) =
            collect_pending_region_edge_support(&pending_regions, &arrangement.vertices);
        profile.edge_support_ms = elapsed_profile_ms(edge_support_start);

        for pending in pending_regions.iter().cloned() {
            let boundary_start = profile_enabled.then(Instant::now);
            let boundary_edges = arrangement.push_boundary_edges_for_pending_region(
                &pending,
                &pending_regions,
                &edge_owners,
                &edge_use_counts,
            );
            profile.boundary_edges_ms += elapsed_profile_ms(boundary_start);
            let push_region_start = profile_enabled.then(Instant::now);
            arrangement.push_region(
                pending.owner,
                pending.height_field_id,
                pending.outer_loop,
                pending.holes,
                boundary_edges,
                pending.area_m2,
                pending.seam_constraints,
            );
            profile.push_regions_ms += elapsed_profile_ms(push_region_start);
        }

        let conflict_start = profile_enabled.then(Instant::now);
        let precomputed_explicit_vertical_step_segments =
            arrangement.reject_implicit_material_height_conflicts()?;
        profile.conflict_ms = elapsed_profile_ms(conflict_start);
        profile.vertices = arrangement.vertices.len();
        profile.edges = arrangement.edges.len();
        profile.regions = arrangement.regions.len();
        profile.seam_constraints = arrangement
            .regions
            .iter()
            .map(|region| region.seam_constraints.len())
            .sum();
        profile.diagnostics = arrangement.diagnostics.len();
        profile.total_ms = elapsed_profile_ms(total_start);
        Ok((
            arrangement,
            profile,
            precomputed_explicit_vertical_step_segments,
        ))
    }

    pub(super) fn reject_implicit_material_height_conflicts(
        &self,
    ) -> Result<Option<Vec<NodeExplicitVerticalStepSegment>>, NodeArrangementError> {
        let profile_enabled = crate::debug::category_enabled("road");
        let total_start = profile_enabled.then(Instant::now);
        let grouping_start = profile_enabled.then(Instant::now);
        let mut vertices_by_key =
            BTreeMap::<NodeArrangementKey, Vec<NodeArrangementVertexId>>::new();
        for vertex in &self.vertices {
            vertices_by_key
                .entry(vertex.key)
                .or_default()
                .push(vertex.id);
        }

        vertices_by_key.retain(|_, vertex_ids| {
            let Some(first_id) = vertex_ids.first().copied() else {
                return false;
            };
            let first_height_key = self.vertices[first_id.0].height_key;
            vertex_ids
                .iter()
                .copied()
                .any(|vertex_id| self.vertices[vertex_id.0].height_key != first_height_key)
        });
        if vertices_by_key.is_empty() {
            return Ok(None);
        }
        let grouping_ms = elapsed_profile_ms(grouping_start);

        let relevant_keys = vertices_by_key.keys().copied().collect::<BTreeSet<_>>();
        let index_start = profile_enabled.then(Instant::now);
        let conflict_index = MaterialHeightConflictIndex::new(self, &relevant_keys);
        let index_ms = elapsed_profile_ms(index_start);
        let check_start = profile_enabled.then(Instant::now);
        let mut pair_checks = 0_usize;

        for (key, vertex_ids) in vertices_by_key {
            for left_index in 0..vertex_ids.len() {
                for right_index in left_index + 1..vertex_ids.len() {
                    pair_checks += 1;
                    let left = &self.vertices[vertex_ids[left_index].0];
                    let right = &self.vertices[vertex_ids[right_index].0];
                    if left.height_key == right.height_key {
                        continue;
                    }
                    let crosses_band_kind = !owners_share_band_kind(&left.owners, &right.owners);
                    if crosses_band_kind
                        && vertices_form_source_authorized_side_join_asphalt_sidewalk_split(
                            left, right,
                        )
                    {
                        continue;
                    }
                    if !crosses_band_kind
                        && vertices_have_distinct_source_carrier_provenance(left, right)
                    {
                        continue;
                    }
                    if crosses_band_kind
                        && !conflict_index.owner_sets_have_boundary_edge(
                            key,
                            &left.owners,
                            &right.owners,
                        )
                    {
                        continue;
                    }
                    let has_explicit_material_seam = crosses_band_kind
                        && (conflict_index.owner_sets_have_material_seam(
                            key,
                            &left.owners,
                            &right.owners,
                        ) || conflict_index.owner_sets_have_material_endpoint_path(
                            key,
                            &left.owners,
                            &right.owners,
                        ));
                    let has_same_material_endpoint_path = !crosses_band_kind
                        && conflict_index.owner_sets_have_material_endpoint_path(
                            key,
                            &left.owners,
                            &right.owners,
                        );
                    let has_explicit_vertical_step = conflict_index.owner_sets_have_vertical_step(
                        key,
                        &left.owners,
                        &right.owners,
                    ) || conflict_index
                        .owner_sets_have_vertical_step_point_sources(
                            key,
                            &left.owners,
                            &right.owners,
                        );
                    if !has_explicit_material_seam
                        && !has_same_material_endpoint_path
                        && !has_explicit_vertical_step
                    {
                        return Err(NodeArrangementError::DuplicateVertexHeightConflict {
                            key,
                            existing_height_mm: left.height_key.0,
                            incoming_height_mm: right.height_key.0,
                        });
                    }
                }
            }
        }
        if profile_enabled {
            crate::debug_log!(
                "road",
                "node_arrangement_conflict_detail node={} relevant_keys={} pair_checks={} vertical_steps={} grouping_ms={:.3} index_ms={:.3} checks_ms={:.3} total_ms={:.3}",
                self.node_id,
                relevant_keys.len(),
                pair_checks,
                conflict_index.vertical_step_segments.len(),
                grouping_ms,
                index_ms,
                elapsed_profile_ms(check_start),
                elapsed_profile_ms(total_start),
            );
        }
        Ok(Some(conflict_index.vertical_step_segments))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeOwnerPair {
    lower: NodeBandOwner,
    upper: NodeBandOwner,
}

impl NodeOwnerPair {
    fn new(a: NodeBandOwner, b: NodeBandOwner) -> Self {
        if a <= b {
            Self { lower: a, upper: b }
        } else {
            Self { lower: b, upper: a }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct MaterialSourceFlags {
    any: bool,
    height_split: bool,
}

struct MaterialHeightConflictIndex {
    boundary_pairs_by_key: BTreeMap<NodeArrangementKey, BTreeSet<NodeOwnerPair>>,
    material_seam_pairs_by_key: BTreeMap<NodeArrangementKey, BTreeSet<NodeOwnerPair>>,
    material_endpoint_adjacency_by_key:
        BTreeMap<NodeArrangementKey, BTreeMap<NodeBandOwner, BTreeSet<NodeBandOwner>>>,
    material_source_by_key_owner:
        BTreeMap<(NodeArrangementKey, NodeBandOwner), MaterialSourceFlags>,
    vertical_step_segments: Vec<NodeExplicitVerticalStepSegment>,
}

impl MaterialHeightConflictIndex {
    fn new(arrangement: &NodeArrangement, relevant_keys: &BTreeSet<NodeArrangementKey>) -> Self {
        let profile_enabled = crate::debug::category_enabled("road");
        let total_start = profile_enabled.then(Instant::now);
        let steps_start = profile_enabled.then(Instant::now);
        let vertical_step_segments = arrangement.explicit_vertical_step_segments();
        let steps_ms = elapsed_profile_ms(steps_start);
        let mut index = Self {
            boundary_pairs_by_key: BTreeMap::new(),
            material_seam_pairs_by_key: BTreeMap::new(),
            material_endpoint_adjacency_by_key: BTreeMap::new(),
            material_source_by_key_owner: BTreeMap::new(),
            vertical_step_segments,
        };
        let edges_start = profile_enabled.then(Instant::now);
        index.collect_edge_pairs(arrangement);
        let edges_ms = elapsed_profile_ms(edges_start);
        let constraints_start = profile_enabled.then(Instant::now);
        index.collect_material_constraint_index(arrangement, relevant_keys);
        if profile_enabled {
            crate::debug_log!(
                "road",
                "node_arrangement_conflict_index_detail node={} relevant_keys={} boundary_keys={} material_seam_keys={} material_endpoint_keys={} material_source_entries={} vertical_steps={} steps_ms={:.3} edges_ms={:.3} constraints_ms={:.3} total_ms={:.3}",
                arrangement.node_id,
                relevant_keys.len(),
                index.boundary_pairs_by_key.len(),
                index.material_seam_pairs_by_key.len(),
                index.material_endpoint_adjacency_by_key.len(),
                index.material_source_by_key_owner.len(),
                index.vertical_step_segments.len(),
                steps_ms,
                edges_ms,
                elapsed_profile_ms(constraints_start),
                elapsed_profile_ms(total_start),
            );
        }
        index
    }

    fn collect_edge_pairs(&mut self, arrangement: &NodeArrangement) {
        for edge in arrangement.edges() {
            let Some(opposite_owner) = edge.opposite_owner else {
                continue;
            };
            let Some(start_key) = arrangement
                .vertices()
                .get(edge.start.0)
                .map(NodeArrangementVertex::key)
            else {
                continue;
            };
            let Some(end_key) = arrangement
                .vertices()
                .get(edge.end.0)
                .map(NodeArrangementVertex::key)
            else {
                continue;
            };
            let pair = NodeOwnerPair::new(edge.owner, opposite_owner);
            for key in [start_key, end_key] {
                self.boundary_pairs_by_key
                    .entry(key)
                    .or_default()
                    .insert(pair);
            }
            if edge.is_material_transition
                && !edge.constrains_shared_height
                && !edge.source_constraint_indices.is_empty()
                && edge.has_applicable_material_source_constraint
                && (arrangement.piece_kind != RoadSurfaceVisualNodePieceKind::Terminal
                    || !arrangement.edge_has_owner_pair_source_constraint(edge))
            {
                for key in [start_key, end_key] {
                    self.material_seam_pairs_by_key
                        .entry(key)
                        .or_default()
                        .insert(pair);
                }
            }
        }
    }

    fn collect_material_constraint_index(
        &mut self,
        arrangement: &NodeArrangement,
        relevant_keys: &BTreeSet<NodeArrangementKey>,
    ) {
        let relevant_key_index = RelevantArrangementKeyIndex::new(relevant_keys);
        let mut owners_by_key_constraint =
            HashMap::<(NodeArrangementKey, usize), Vec<NodeBandOwner>>::new();
        for region in arrangement.regions() {
            for constraint in &region.seam_constraints {
                if !constraint.is_material_transition {
                    continue;
                }
                let start = NodeArrangementKey::from_point(constraint.start_xz);
                let end = NodeArrangementKey::from_point(constraint.end_xz);
                relevant_key_index.for_each_key_on_segment(start, end, |key| {
                    if !constraint.constrains_shared_height {
                        let owners = owners_by_key_constraint
                            .entry((key, constraint.constraint_index))
                            .or_default();
                        for owner in owners_for_material_seam_constraint(constraint, region.owner) {
                            if let Err(insert_at) = owners.binary_search(&owner) {
                                owners.insert(insert_at, owner);
                            }
                        }
                    }
                    if seam_constraint_can_source_region_owner_for_pair(
                        constraint,
                        region.owner,
                        region.owner,
                    ) {
                        let flags = self
                            .material_source_by_key_owner
                            .entry((key, region.owner))
                            .or_default();
                        flags.any = true;
                        flags.height_split |= !constraint.constrains_shared_height;
                    }
                });
            }
        }

        for ((key, _), owners) in owners_by_key_constraint {
            for left_index in 0..owners.len() {
                for right_index in left_index + 1..owners.len() {
                    let left = owners[left_index];
                    let right = owners[right_index];
                    self.material_endpoint_adjacency_by_key
                        .entry(key)
                        .or_default()
                        .entry(left)
                        .or_default()
                        .insert(right);
                    self.material_endpoint_adjacency_by_key
                        .entry(key)
                        .or_default()
                        .entry(right)
                        .or_default()
                        .insert(left);
                }
            }
        }
    }

    fn owner_sets_have_boundary_edge(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        self.owner_sets_have_pair(&self.boundary_pairs_by_key, key, left_owners, right_owners)
    }

    fn owner_sets_have_material_seam(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        self.owner_sets_have_pair(
            &self.material_seam_pairs_by_key,
            key,
            left_owners,
            right_owners,
        )
    }

    fn owner_sets_have_pair(
        &self,
        pairs_by_key: &BTreeMap<NodeArrangementKey, BTreeSet<NodeOwnerPair>>,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        let Some(pairs) = pairs_by_key.get(&key) else {
            return false;
        };
        left_owners.iter().copied().any(|left_owner| {
            right_owners
                .iter()
                .copied()
                .any(|right_owner| pairs.contains(&NodeOwnerPair::new(left_owner, right_owner)))
        })
    }

    fn owner_sets_have_material_endpoint_path(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        let Some(adjacency) = self.material_endpoint_adjacency_by_key.get(&key) else {
            return false;
        };
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

    fn owner_sets_have_vertical_step(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        self.vertical_step_segments.iter().copied().any(|segment| {
            key.lies_on_segment(segment.start(), segment.end())
                && owner_sets_match_step(
                    left_owners,
                    right_owners,
                    segment.owner(),
                    segment.opposite_owner(),
                )
        }) || owner_sets_have_explicit_vertical_step_endpoint_authority(
            key,
            left_owners,
            right_owners,
            &self.vertical_step_segments,
        )
    }

    fn owner_sets_have_vertical_step_point_sources(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        left_owners.iter().copied().any(|left_owner| {
            right_owners.iter().copied().any(|right_owner| {
                owners_form_explicit_vertical_step_pair(left_owner, right_owner)
                    && self.owner_has_material_source(key, left_owner, false)
                    && self.owner_has_material_source(key, right_owner, false)
                    && (self.owner_has_material_source(key, left_owner, true)
                        || self.owner_has_material_source(key, right_owner, true))
            })
        })
    }

    fn owner_has_material_source(
        &self,
        key: NodeArrangementKey,
        owner: NodeBandOwner,
        require_height_split: bool,
    ) -> bool {
        self.material_source_by_key_owner
            .get(&(key, owner))
            .is_some_and(|flags| flags.any && (!require_height_split || flags.height_split))
    }
}

struct RelevantArrangementKeyIndex {
    by_x: Vec<NodeArrangementKey>,
    by_z: Vec<NodeArrangementKey>,
}

impl RelevantArrangementKeyIndex {
    fn new(keys: &BTreeSet<NodeArrangementKey>) -> Self {
        let mut by_x = keys.iter().copied().collect::<Vec<_>>();
        by_x.sort_unstable_by_key(|key| (key.x_key, key.z_key));
        let mut by_z = keys.iter().copied().collect::<Vec<_>>();
        by_z.sort_unstable_by_key(|key| (key.z_key, key.x_key));
        Self { by_x, by_z }
    }

    fn for_each_key_on_segment(
        &self,
        start: NodeArrangementKey,
        end: NodeArrangementKey,
        mut visit: impl FnMut(NodeArrangementKey),
    ) {
        let x_span = start.x_key.abs_diff(end.x_key);
        let z_span = start.z_key.abs_diff(end.z_key);
        let (keys, start_axis, end_axis, axis_value): (
            &[NodeArrangementKey],
            i64,
            i64,
            fn(NodeArrangementKey) -> i64,
        ) = if x_span >= z_span {
            (&self.by_x, start.x_key, end.x_key, |key| key.x_key)
        } else {
            (&self.by_z, start.z_key, end.z_key, |key| key.z_key)
        };
        let min_axis = start_axis.min(end_axis);
        let max_axis = start_axis.max(end_axis);
        let first = keys.partition_point(|key| axis_value(*key) < min_axis);
        let after_last = keys.partition_point(|key| axis_value(*key) <= max_axis);
        for &key in &keys[first..after_last] {
            if key.lies_on_segment(start, end) {
                visit(key);
            }
        }
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

fn vertices_have_distinct_source_carrier_provenance(
    left: &NodeArrangementVertex,
    right: &NodeArrangementVertex,
) -> bool {
    grade_authorities_have_distinct_source_carrier_provenance(
        left.grade_authority,
        right.grade_authority,
    )
}

fn grade_authorities_have_distinct_source_carrier_provenance(
    left: NodeGradeVertexAuthority,
    right: NodeGradeVertexAuthority,
) -> bool {
    match (left.source_provenance, right.source_provenance) {
        (Some(left), Some(right)) => left != right,
        _ => false,
    }
}

fn vertices_form_source_authorized_side_join_asphalt_sidewalk_split(
    left: &NodeArrangementVertex,
    right: &NodeArrangementVertex,
) -> bool {
    source_authorities_form_side_join_asphalt_sidewalk_split(
        left.grade_authority,
        right.grade_authority,
    )
}

pub(crate) fn source_authorities_form_side_join_asphalt_sidewalk_split(
    left: NodeGradeVertexAuthority,
    right: NodeGradeVertexAuthority,
) -> bool {
    let Some((carriageway, sidewalk)) = ordered_carriageway_sidewalk_authorities(left, right)
    else {
        return false;
    };
    source_authority_is_junction_side_join_carriageway(carriageway)
        && source_authority_is_source_join_sidewalk(sidewalk)
}

fn ordered_carriageway_sidewalk_authorities(
    left: NodeGradeVertexAuthority,
    right: NodeGradeVertexAuthority,
) -> Option<(NodeGradeVertexAuthority, NodeGradeVertexAuthority)> {
    match (left.owner.kind(), right.owner.kind()) {
        (RoadSurfaceBandKind::Carriageway, RoadSurfaceBandKind::Sidewalk) => Some((left, right)),
        (RoadSurfaceBandKind::Sidewalk, RoadSurfaceBandKind::Carriageway) => Some((right, left)),
        _ => None,
    }
}

fn source_authority_is_junction_side_join_carriageway(authority: NodeGradeVertexAuthority) -> bool {
    let Some(provenance) = authority.source_provenance else {
        return false;
    };
    provenance_matches_authority(provenance, authority)
        && provenance.source_kind == RoadSurfaceBandKind::Carriageway
        && provenance.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
        && match provenance.origin {
            NodeCarrierProvenanceOrigin::SourceIntersection { peer_count } => peer_count > 0,
            NodeCarrierProvenanceOrigin::GeneratedCarrierVertex {
                purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                ..
            } => true,
            _ => false,
        }
}

fn source_authority_is_source_join_sidewalk(authority: NodeGradeVertexAuthority) -> bool {
    let Some(provenance) = authority.source_provenance else {
        return false;
    };
    provenance_matches_authority(provenance, authority)
        && provenance.source_kind == RoadSurfaceBandKind::Sidewalk
        && matches!(
            provenance.claim_priority,
            NodeGeneratedContourClaimPriority::MouthBand
                | NodeGeneratedContourClaimPriority::SideJoin
        )
        && match provenance.origin {
            NodeCarrierProvenanceOrigin::SourceIntersection { peer_count } => peer_count > 0,
            NodeCarrierProvenanceOrigin::GeneratedCarrierVertex {
                purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
                ..
            } => true,
            _ => false,
        }
}

fn provenance_matches_authority(
    provenance: NodeHeightCarrierProvenanceKey,
    authority: NodeGradeVertexAuthority,
) -> bool {
    provenance.owner == authority.owner && provenance.height_field_id == authority.height_field_id
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

pub(super) fn node_grade_decision_rank(decision: NodeGradeCarrierDecision) -> u8 {
    match decision {
        NodeGradeCarrierDecision::ExplicitMaterialSeam => 0,
        NodeGradeCarrierDecision::SameMaterialSeam => 1,
        NodeGradeCarrierDecision::SameMaterialSharedEdge => 2,
        NodeGradeCarrierDecision::SameMaterialVertex => 3,
        NodeGradeCarrierDecision::SameOwnerCanonicalVertex => 4,
        NodeGradeCarrierDecision::SourceCarrier { .. } => 5,
    }
}

fn owner_sets_match_step(
    left_owners: &[NodeBandOwner],
    right_owners: &[NodeBandOwner],
    step_owner: NodeBandOwner,
    step_opposite_owner: NodeBandOwner,
) -> bool {
    (left_owners.contains(&step_owner) && right_owners.contains(&step_opposite_owner))
        || (left_owners.contains(&step_opposite_owner) && right_owners.contains(&step_owner))
}

fn owner_sets_have_explicit_vertical_step_endpoint_authority(
    key: NodeArrangementKey,
    left_owners: &[NodeBandOwner],
    right_owners: &[NodeBandOwner],
    segments: &[NodeExplicitVerticalStepSegment],
) -> bool {
    left_owners.iter().copied().any(|left_owner| {
        right_owners.iter().copied().any(|right_owner| {
            let Some(left_rank) = raised_step_band_rank(left_owner.kind()) else {
                return false;
            };
            let Some(right_rank) = raised_step_band_rank(right_owner.kind()) else {
                return false;
            };
            match left_rank.cmp(&right_rank) {
                std::cmp::Ordering::Less => {
                    explicit_vertical_step_segments_authorize_height_side_at_key(
                        key, left_owner, true, segments,
                    ) && explicit_vertical_step_segments_authorize_height_side_at_key(
                        key,
                        right_owner,
                        false,
                        segments,
                    )
                }
                std::cmp::Ordering::Greater => {
                    explicit_vertical_step_segments_authorize_height_side_at_key(
                        key, left_owner, false, segments,
                    ) && explicit_vertical_step_segments_authorize_height_side_at_key(
                        key,
                        right_owner,
                        true,
                        segments,
                    )
                }
                std::cmp::Ordering::Equal => false,
            }
        })
    })
}

pub(super) fn seam_constraint_can_source_region_owner_for_pair(
    constraint: &NodeRegionSeamConstraint,
    owner: NodeBandOwner,
    _opposite_owner: NodeBandOwner,
) -> bool {
    match (constraint.owner, constraint.opposite_owner) {
        (Some(left), Some(right)) => left == owner || right == owner,
        (Some(constraint_owner), None) | (None, Some(constraint_owner)) => {
            constraint_owner == owner
        }
        (None, None) => true,
    }
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

pub(super) fn merge_sorted_unique<T>(target: &mut Vec<T>, incoming: impl IntoIterator<Item = T>)
where
    T: Ord,
{
    let previous_len = target.len();
    target.extend(incoming);
    if target.len() == previous_len {
        return;
    }
    target.sort();
    target.dedup();
}
