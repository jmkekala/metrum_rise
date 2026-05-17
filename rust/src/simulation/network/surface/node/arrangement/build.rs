//! Construction pipeline for canonical node arrangements.

use super::super::backend::RoadVec2;
use super::super::grade::{NodeGradeCarrierDecision, NodeGradeVertexAuthority};
use super::super::height::{NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex};
use super::super::keys::{SurfaceHeightMmKey, SurfaceXzSegmentKey};
use super::super::triangulation::{NodeTriangulatedRegion, NodeTriangulationSolution};
use super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::seams::{
    NodeRegionSeamConstraint, NodeSeamSource, owners_for_material_seam_constraint,
    seam_constraint_can_source_edge_owner_pair, seam_constraint_covers_edge,
    seam_constraint_touches_key, seam_constraints_are_ambiguous,
};
use super::{
    NodeArrangement, NodeArrangementDiagnostic, NodeArrangementEdge, NodeArrangementEdgeId,
    NodeArrangementError, NodeArrangementFace, NodeArrangementFaceId, NodeArrangementHeightKey,
    NodeArrangementKey, NodeArrangementVertex, NodeArrangementVertexContextKey,
    NodeArrangementVertexId, NodeBandHeightFieldId, NodeBandOwner, NodeOwnedRegion,
    NodeOwnedRegionId,
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

    fn insert_vertex_with_grade_authority(
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

    pub(crate) fn push_region(
        &mut self,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        outer_loop: Vec<NodeArrangementVertexId>,
        holes: Vec<Vec<NodeArrangementVertexId>>,
        boundary_edges: Vec<NodeArrangementEdgeId>,
        area_m2: f32,
        seam_constraints: Vec<NodeRegionSeamConstraint>,
    ) -> NodeOwnedRegionId {
        let id = NodeOwnedRegionId(self.regions.len());
        self.regions.push(NodeOwnedRegion {
            id,
            owner,
            height_field_id,
            outer_loop,
            holes,
            boundary_edges,
            area_m2,
            seam_constraints,
        });
        id
    }

    pub(crate) fn push_face(
        &mut self,
        region: NodeOwnedRegionId,
        owner: NodeBandOwner,
        vertices: [NodeArrangementVertexId; 3],
    ) -> NodeArrangementFaceId {
        let id = NodeArrangementFaceId(self.faces.len());
        self.faces.push(NodeArrangementFace {
            id,
            region,
            owner,
            vertices,
        });
        id
    }

    pub(crate) fn from_height_solution(
        heights: &NodeHeightSolution,
    ) -> Result<Self, NodeArrangementError> {
        let mut arrangement = Self::new(heights.node_id, heights.piece_kind);
        let mut pending_regions = Vec::with_capacity(heights.regions.len());
        let mut edge_owners =
            BTreeMap::<NodeArrangementEdgeKey, Vec<NodeArrangementEdgeOwner>>::new();
        let mut edge_use_counts = BTreeMap::<NodeArrangementEdgeKey, usize>::new();

        for (region_index, height_region) in heights.regions.iter().enumerate() {
            let pending = arrangement.pending_region(region_index, height_region)?;
            let pending_edge_owner = NodeArrangementEdgeOwner {
                owner: pending.owner,
                height_field_id: pending.height_field_id,
            };
            for edge in pending.loop_edges(&arrangement.vertices) {
                *edge_use_counts.entry(edge.key).or_default() += 1;
                edge_owners
                    .entry(edge.key)
                    .and_modify(|owners| merge_sorted_unique(owners, vec![pending_edge_owner]))
                    .or_insert_with(|| vec![pending_edge_owner]);
            }
            pending_regions.push(pending);
        }

        for pending in pending_regions {
            let mut boundary_edges = Vec::with_capacity(pending.edge_count());
            for edge in pending.loop_edges(&arrangement.vertices) {
                let opposite = edge_owners.get(&edge.key).and_then(|owners| {
                    owners
                        .iter()
                        .copied()
                        .find(|owner| owner.owner != pending.owner)
                });
                let opposite_owner = opposite.map(|owner| owner.owner);
                let opposite_height_field_id = opposite.map(|owner| owner.height_field_id);
                let source_constraints = source_constraints_for_edge(
                    edge,
                    &pending.seam_constraints,
                    &arrangement.vertices,
                );
                if let Some(opposite_owner) = opposite_owner {
                    if owners_require_explicit_boundary_seam(pending.owner, opposite_owner) {
                        if source_constraints.is_empty() {
                            arrangement.diagnostics.push(
                                NodeArrangementDiagnostic::MissingSeamConstraint {
                                    region_index: pending.region_index,
                                    owner: pending.owner,
                                    opposite_owner,
                                    start: edge.key.start,
                                    end: edge.key.end,
                                },
                            );
                        } else if seam_constraints_are_ambiguous(&source_constraints) {
                            arrangement.diagnostics.push(
                                NodeArrangementDiagnostic::AmbiguousSeamConstraint {
                                    region_index: pending.region_index,
                                    owner: pending.owner,
                                    opposite_owner,
                                    start: edge.key.start,
                                    end: edge.key.end,
                                },
                            );
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
                boundary_edges.push(arrangement.push_edge(
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

    pub(crate) fn attach_triangulation(
        &mut self,
        triangulation: &NodeTriangulationSolution,
    ) -> Result<(), NodeArrangementError> {
        if self.node_id != triangulation.node_id || self.piece_kind != triangulation.piece_kind {
            return Err(NodeArrangementError::InputSolutionMismatch {
                height_node_id: self.node_id,
                triangulation_node_id: triangulation.node_id,
                height_piece_kind: self.piece_kind,
                triangulation_piece_kind: triangulation.piece_kind,
            });
        }
        if self.regions.len() != triangulation.regions.len() {
            return Err(NodeArrangementError::TriangulationRegionCountMismatch {
                arrangement_region_count: self.regions.len(),
                triangulation_region_count: triangulation.regions.len(),
            });
        }

        for (region_index, triangulated_region) in triangulation.regions.iter().enumerate() {
            let region_id = NodeOwnedRegionId(region_index);
            let arrangement_region = self
                .regions
                .get(region_index)
                .ok_or(NodeArrangementError::MissingHeightRegion { region_index })?;
            if arrangement_region.owner != triangulated_region.owner {
                return Err(NodeArrangementError::RegionOwnerMismatch { region_index });
            }
            for triangle in &triangulated_region.triangles {
                let vertices = [
                    self.insert_triangulated_vertex(
                        region_index,
                        triangulated_region,
                        triangle.vertices[0],
                    )?,
                    self.insert_triangulated_vertex(
                        region_index,
                        triangulated_region,
                        triangle.vertices[1],
                    )?,
                    self.insert_triangulated_vertex(
                        region_index,
                        triangulated_region,
                        triangle.vertices[2],
                    )?,
                ];
                self.push_face(region_id, triangulated_region.owner, vertices);
            }
        }

        self.reject_implicit_material_height_conflicts()?;
        Ok(())
    }

    fn pending_region(
        &mut self,
        region_index: usize,
        region: &NodeHeightedRegion,
    ) -> Result<PendingArrangementRegion, NodeArrangementError> {
        if region.shape.is_empty() {
            return Err(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index: 0,
            });
        }

        let mut contours = Vec::with_capacity(region.shape.len());
        for (contour_index, contour) in region.shape.iter().enumerate() {
            contours.push(self.insert_heighted_contour(
                region_index,
                contour_index,
                region,
                contour,
            )?);
        }
        let mut contours = contours.into_iter();
        let outer_loop = contours
            .next()
            .ok_or(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index: 0,
            })?;
        let holes = contours.collect();
        Ok(PendingArrangementRegion {
            region_index,
            owner: region.owner,
            height_field_id: region.height_field_id,
            outer_loop,
            holes,
            area_m2: region.area_m2,
            seam_constraints: region.seam_constraints.clone(),
        })
    }

    fn insert_heighted_contour(
        &mut self,
        region_index: usize,
        contour_index: usize,
        region: &NodeHeightedRegion,
        contour: &[NodeHeightedVertex],
    ) -> Result<Vec<NodeArrangementVertexId>, NodeArrangementError> {
        if contour.len() < 3 {
            return Err(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index,
            });
        }

        contour
            .iter()
            .map(|vertex| {
                let grade_authority = vertex.grade_authority.ok_or_else(|| {
                    NodeArrangementError::MissingGradeAuthority {
                        region_index,
                        contour_index,
                        key: NodeArrangementKey::from_point(vertex.point_xz),
                        owner: region.owner,
                        height_field_id: vertex.height_field_id,
                        height_mm: quantize_height_m(vertex.height_m),
                    }
                })?;
                self.insert_vertex_with_grade_authority(
                    vertex.point_xz,
                    vertex.height_m,
                    [region.owner],
                    vertex.height_field_id,
                    [NodeSeamSource::for_owner(region.owner)],
                    grade_authority,
                )
            })
            .collect()
    }

    fn insert_triangulated_vertex(
        &mut self,
        region_index: usize,
        region: &NodeTriangulatedRegion,
        vertex_index: usize,
    ) -> Result<NodeArrangementVertexId, NodeArrangementError> {
        let vertex = region.vertices.get(vertex_index).ok_or(
            NodeArrangementError::MissingTriangulatedVertex {
                region_index,
                vertex_index,
            },
        )?;
        self.insert_vertex_with_grade_authority(
            RoadVec2::new(vertex.point_world.x, vertex.point_world.z),
            vertex.point_world.y,
            [region.owner],
            vertex.height_field_id,
            [NodeSeamSource::for_owner(region.owner)],
            vertex.grade_authority,
        )
    }

    fn reject_implicit_material_height_conflicts(&self) -> Result<(), NodeArrangementError> {
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

    fn edge_touches_key(&self, edge: &NodeArrangementEdge, key: NodeArrangementKey) -> bool {
        let Some(start) = self.vertices.get(edge.start.0) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0) else {
            return false;
        };
        start.key == key || end.key == key
    }

    fn edge_has_applicable_material_source_constraint(&self, edge: &NodeArrangementEdge) -> bool {
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

fn quantize_height_m(value_m: f64) -> i64 {
    SurfaceHeightMmKey::from_m_f64(value_m).as_i64()
}

#[derive(Clone)]
struct PendingArrangementRegion {
    region_index: usize,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    outer_loop: Vec<NodeArrangementVertexId>,
    holes: Vec<Vec<NodeArrangementVertexId>>,
    area_m2: f32,
    seam_constraints: Vec<NodeRegionSeamConstraint>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeArrangementEdgeOwner {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeArrangementEdgeKey {
    start: NodeArrangementKey,
    end: NodeArrangementKey,
}

#[derive(Clone, Copy)]
struct PendingArrangementEdge {
    key: NodeArrangementEdgeKey,
    start: NodeArrangementVertexId,
    end: NodeArrangementVertexId,
}

impl PendingArrangementRegion {
    fn edge_count(&self) -> usize {
        self.outer_loop.len() + self.holes.iter().map(Vec::len).sum::<usize>()
    }

    fn loop_edges(&self, vertices: &[NodeArrangementVertex]) -> Vec<PendingArrangementEdge> {
        let mut edges = loop_edges(&self.outer_loop, vertices);
        for hole in &self.holes {
            edges.extend(loop_edges(hole, vertices));
        }
        edges
    }
}

fn loop_edges(
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

fn owners_require_explicit_boundary_seam(a: NodeBandOwner, b: NodeBandOwner) -> bool {
    a.kind() != b.kind()
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

fn canonical_sources<T>(sources: impl IntoIterator<Item = T>) -> Vec<T>
where
    T: Ord,
{
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources
}

fn merge_sorted_unique<T>(target: &mut Vec<T>, incoming: Vec<T>)
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
