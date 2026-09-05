//! Explicit vertical-step extraction from arrangement edges.

use super::*;

impl NodeArrangement {
    pub(crate) fn explicit_vertical_step_segments(&self) -> Vec<NodeExplicitVerticalStepSegment> {
        let (segments, _) = self.explicit_vertical_step_segment_sets();
        segments.into_iter().collect()
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface::node) fn derived_overlap_explicit_vertical_step_segments(
        &self,
    ) -> BTreeSet<NodeExplicitVerticalStepSegment> {
        let (_, derived_overlap_segments) = self.explicit_vertical_step_segment_sets();
        derived_overlap_segments
    }

    fn explicit_vertical_step_segment_sets(
        &self,
    ) -> (
        BTreeSet<NodeExplicitVerticalStepSegment>,
        BTreeSet<NodeExplicitVerticalStepSegment>,
    ) {
        let profile_enabled = crate::debug::category_enabled("road");
        let total_start = profile_enabled.then(std::time::Instant::now);
        let index_start = profile_enabled.then(std::time::Instant::now);
        let constraint_index = ExplicitStepConstraintIndex::new(self);
        let index_ms = index_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let source_start = profile_enabled.then(std::time::Instant::now);
        let mut segments = BTreeSet::new();
        for edge in &self.edges {
            let Some(opposite_owner) =
                self.edge_explicit_vertical_step_opposite_owner(edge, &constraint_index)
            else {
                continue;
            };
            let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
                continue;
            };
            let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
                continue;
            };
            if let Some(segment) =
                NodeExplicitVerticalStepSegment::new(start, end, edge.owner, opposite_owner)
            {
                segments.insert(segment);
            }
        }
        let source_ms = source_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let source_segments = segments.clone();
        let exposed_start = profile_enabled.then(std::time::Instant::now);
        self.extend_exposed_owned_raised_step_overlap_segments(&mut segments);
        let exposed_ms = exposed_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let mut derived_overlap_segments = segments
            .difference(&source_segments)
            .copied()
            .collect::<BTreeSet<_>>();
        let final_boundary_start = profile_enabled.then(std::time::Instant::now);
        let final_boundary_segments =
            self.authorized_final_boundary_raised_step_overlap_segments(&segments);
        let final_boundary_ms = final_boundary_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        derived_overlap_segments.extend(final_boundary_segments.iter().copied());
        segments.extend(final_boundary_segments.iter().copied());
        if profile_enabled {
            crate::debug_log!(
                "road",
                "node_explicit_step_detail node={} edges={} source_steps={} derived_steps={} final_boundary_steps={} index_ms={:.3} source_ms={:.3} exposed_ms={:.3} final_boundary_ms={:.3} total_ms={:.3}",
                self.node_id,
                self.edges.len(),
                source_segments.len(),
                derived_overlap_segments.len(),
                final_boundary_segments.len(),
                index_ms,
                source_ms,
                exposed_ms,
                final_boundary_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0),
            );
        }
        (segments, derived_overlap_segments)
    }

    fn extend_exposed_owned_raised_step_overlap_segments(
        &self,
        segments: &mut BTreeSet<NodeExplicitVerticalStepSegment>,
    ) {
        let mut candidates = self
            .edges
            .iter()
            .filter(|edge| edge.exposed_boundary)
            .filter_map(|edge| {
                let rank = raised_step_band_rank(edge.owner.kind())?;
                let geometry = self.arrangement_edge_geometry(edge)?;
                Some(ExposedRaisedStepEdge {
                    edge,
                    geometry,
                    rank,
                    min_x: geometry.start.x_key.min(geometry.end.x_key),
                    min_z: geometry.start.z_key.min(geometry.end.z_key),
                    max_x: geometry.start.x_key.max(geometry.end.x_key),
                    max_z: geometry.start.z_key.max(geometry.end.z_key),
                })
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|candidate| {
            (
                candidate.min_x,
                candidate.min_z,
                candidate.max_x,
                candidate.max_z,
                candidate.edge.owner,
                candidate.geometry.start,
                candidate.geometry.end,
            )
        });

        // Exact sweep-line broad phase: positive segment overlap requires overlapping AABBs.
        // Sorting by min X lets us stop each inner scan once that necessary condition fails.
        for left_index in 0..candidates.len() {
            let left = candidates[left_index];
            for &right in candidates.iter().skip(left_index + 1) {
                if right.min_x > left.max_x {
                    break;
                }
                if right.min_z > left.max_z
                    || right.max_z < left.min_z
                    || left.rank == right.rank
                    || !raised_step_kinds_can_contact(
                        left.edge.owner.kind(),
                        right.edge.owner.kind(),
                    )
                {
                    continue;
                }
                if let Some(segment) =
                    self.exposed_owned_raised_step_overlap_segment_from_candidates(left, right)
                {
                    segments.insert(segment);
                }
            }
        }
    }

    fn exposed_owned_raised_step_overlap_segment_from_candidates(
        &self,
        left: ExposedRaisedStepEdge<'_>,
        right: ExposedRaisedStepEdge<'_>,
    ) -> Option<NodeExplicitVerticalStepSegment> {
        let (start, end) = arrangement_edge_overlap_segment(left.geometry, right.geometry)?;
        let (lower_edge, lower, raised_edge, raised) = if left.rank < right.rank {
            (left.edge, left.geometry, right.edge, right.geometry)
        } else {
            (right.edge, right.geometry, left.edge, left.geometry)
        };
        if !arrangement_edges_have_positive_raised_step_delta(lower, raised, start, end) {
            return None;
        }

        NodeExplicitVerticalStepSegment::new(start, end, lower_edge.owner, raised_edge.owner)
    }

    fn arrangement_edge_geometry(
        &self,
        edge: &NodeArrangementEdge,
    ) -> Option<NodeArrangementEdgeGeometry> {
        let start = self.vertices.get(edge.start.0)?;
        let end = self.vertices.get(edge.end.0)?;
        (start.key != end.key).then_some(NodeArrangementEdgeGeometry {
            start: start.key,
            end: end.key,
            start_height_mm: start.height_mm(),
            end_height_mm: end.height_mm(),
        })
    }

    fn authorized_final_boundary_raised_step_overlap_segments(
        &self,
        existing_segments: &BTreeSet<NodeExplicitVerticalStepSegment>,
    ) -> BTreeSet<NodeExplicitVerticalStepSegment> {
        let boundary_edges = self.final_owned_boundary_edge_geometries();
        self.authorized_final_boundary_raised_step_overlap_segments_from_edges(
            existing_segments,
            &boundary_edges,
        )
    }

    fn authorized_final_boundary_raised_step_overlap_segments_from_edges(
        &self,
        existing_segments: &BTreeSet<NodeExplicitVerticalStepSegment>,
        boundary_edges: &[NodeArrangementOwnedBoundaryEdgeGeometry],
    ) -> BTreeSet<NodeExplicitVerticalStepSegment> {
        let mut segments = BTreeSet::new();
        for left_index in 0..boundary_edges.len() {
            let left = boundary_edges[left_index];
            for right in boundary_edges.iter().skip(left_index + 1).copied() {
                if let Some(segment) =
                    self.authorized_final_boundary_raised_step_overlap_segment(left, right)
                    && !explicit_segments_cover_owned_boundary_step(existing_segments, segment)
                {
                    segments.insert(segment);
                }
            }
        }
        segments
    }

    fn authorized_final_boundary_raised_step_overlap_segment(
        &self,
        left: NodeArrangementOwnedBoundaryEdgeGeometry,
        right: NodeArrangementOwnedBoundaryEdgeGeometry,
    ) -> Option<NodeExplicitVerticalStepSegment> {
        if left.owner == right.owner
            || !raised_step_kinds_can_contact(left.owner.kind(), right.owner.kind())
        {
            return None;
        }
        let left_rank = raised_step_band_rank(left.owner.kind())?;
        let right_rank = raised_step_band_rank(right.owner.kind())?;
        if left_rank == right_rank {
            return None;
        }
        let (start, end) = arrangement_edge_overlap_segment(left.geometry(), right.geometry())?;
        let (lower, raised) = if left_rank < right_rank {
            (left, right)
        } else {
            (right, left)
        };
        if !arrangement_edges_have_positive_raised_step_delta(
            lower.geometry(),
            raised.geometry(),
            start,
            end,
        ) {
            return None;
        }
        if !self.final_boundary_overlap_has_source_authority(lower, raised, start, end) {
            return None;
        }

        NodeExplicitVerticalStepSegment::new(start, end, lower.owner, raised.owner)
    }

    fn final_boundary_overlap_has_source_authority(
        &self,
        lower: NodeArrangementOwnedBoundaryEdgeGeometry,
        raised: NodeArrangementOwnedBoundaryEdgeGeometry,
        start: NodeArrangementKey,
        end: NodeArrangementKey,
    ) -> bool {
        self.final_boundary_edge_has_step_source_authority(lower, raised.owner, start, end)
            && self.final_boundary_edge_has_step_source_authority(raised, lower.owner, start, end)
    }

    fn final_boundary_edge_has_step_source_authority(
        &self,
        edge: NodeArrangementOwnedBoundaryEdgeGeometry,
        opposite_owner: NodeBandOwner,
        start: NodeArrangementKey,
        end: NodeArrangementKey,
    ) -> bool {
        let Some(region) = self.regions.get(edge.region.0) else {
            return false;
        };
        region.seam_constraints.iter().any(|constraint| {
            seam_constraint_matches_owner_pair(constraint, edge.owner, opposite_owner)
                && seam_constraint_authorizes_explicit_height_split(constraint)
                && seam_constraint_covers_edge(constraint, start, end)
        })
    }

    /// Collects owner-local edges exposed by the final attached face set.
    pub(super) fn final_owned_boundary_edge_geometries(
        &self,
    ) -> Vec<NodeArrangementOwnedBoundaryEdgeGeometry> {
        let mut edge_counts = BTreeMap::<
            (NodeBandOwner, (NodeArrangementKey, NodeArrangementKey)),
            (
                usize,
                NodeBandOwner,
                NodeOwnedRegionId,
                NodeArrangementKey,
                NodeArrangementKey,
                i64,
                i64,
            ),
        >::new();
        for face in &self.faces {
            if self.arrangement_face_area_abs_m2(face)
                <= f64::from(crate::simulation::network::surface::NODE_OVERLAY_MIN_AREA_M2)
            {
                continue;
            }
            let vertices = face.vertices();
            for index in 0..vertices.len() {
                let Some(start) = self.vertices.get(vertices[index].0) else {
                    continue;
                };
                let Some(end) = self.vertices.get(vertices[(index + 1) % vertices.len()].0) else {
                    continue;
                };
                let edge_key = normalized_arrangement_key_pair(start.key(), end.key());
                edge_counts
                    .entry((face.owner(), edge_key))
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((
                        1,
                        face.owner(),
                        face.region,
                        start.key(),
                        end.key(),
                        start.height_mm(),
                        end.height_mm(),
                    ));
            }
        }

        edge_counts
            .into_values()
            .filter_map(
                |(count, owner, region, start, end, start_height_mm, end_height_mm)| {
                    (count == 1 && start != end).then_some(
                        NodeArrangementOwnedBoundaryEdgeGeometry {
                            owner,
                            region,
                            start,
                            end,
                            start_height_mm,
                            end_height_mm,
                        },
                    )
                },
            )
            .collect()
    }

    fn arrangement_face_area_abs_m2(&self, face: &super::super::NodeArrangementFace) -> f64 {
        let vertices = face.vertices();
        let Some(a) = self
            .vertices
            .get(vertices[0].0)
            .map(|vertex| vertex.point_xz())
        else {
            return 0.0;
        };
        let Some(b) = self
            .vertices
            .get(vertices[1].0)
            .map(|vertex| vertex.point_xz())
        else {
            return 0.0;
        };
        let Some(c) = self
            .vertices
            .get(vertices[2].0)
            .map(|vertex| vertex.point_xz())
        else {
            return 0.0;
        };
        ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5
    }

    pub(in crate::simulation::network::surface::node::arrangement) fn edge_has_owner_pair_source_constraint(
        &self,
        edge: &NodeArrangementEdge,
    ) -> bool {
        let Some(opposite_owner) = edge.opposite_owner else {
            return false;
        };
        self.edge_has_owner_pair_source_constraint_for_opposite(edge, opposite_owner)
    }

    fn edge_has_owner_pair_source_constraint_for_opposite(
        &self,
        edge: &NodeArrangementEdge,
        opposite_owner: NodeBandOwner,
    ) -> bool {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return false;
        };
        self.regions.iter().any(|region| {
            region.seam_constraints.iter().any(|constraint| {
                seam_constraint_matches_owner_pair(constraint, edge.owner, opposite_owner)
                    && seam_constraint_authorizes_explicit_height_split(constraint)
                    && edge
                        .source_constraint_indices
                        .contains(&constraint.constraint_index)
                    && seam_constraint_covers_edge(constraint, start, end)
            })
        })
    }

    fn edge_has_owner_pair_source_constraint_for_opposite_indexed(
        &self,
        edge: &NodeArrangementEdge,
        opposite_owner: NodeBandOwner,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> bool {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return false;
        };
        constraint_index.for_edge(edge).any(|constraint| {
            seam_constraint_matches_owner_pair(constraint, edge.owner, opposite_owner)
                && seam_constraint_authorizes_explicit_height_split(constraint)
                && seam_constraint_covers_edge(constraint, start, end)
        })
    }

    fn edge_explicit_vertical_step_opposite_owner(
        &self,
        edge: &NodeArrangementEdge,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> Option<NodeBandOwner> {
        if edge.constrains_shared_height {
            if let Some(opposite_owner) = edge.opposite_owner
                && edge.owner.kind() == opposite_owner.kind()
                && owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                && (self.edge_has_owner_pair_endpoint_source_constraints_for_opposite(
                    edge,
                    opposite_owner,
                    constraint_index,
                ) || self.edge_has_distributed_endpoint_source_constraints_for_opposite(
                    edge,
                    opposite_owner,
                    constraint_index,
                ))
            {
                return Some(opposite_owner);
            }
            return None;
        }

        if edge.is_material_transition {
            let mut candidates = BTreeSet::new();
            if let Some(opposite_owner) = edge.opposite_owner {
                if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                    && self.edge_has_owner_pair_source_constraint_for_opposite_indexed(
                        edge,
                        opposite_owner,
                        constraint_index,
                    )
                {
                    return Some(opposite_owner);
                }
                if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                    && self.edge_has_material_endpoint_path_for_opposite(
                        edge,
                        opposite_owner,
                        constraint_index,
                    )
                {
                    return Some(opposite_owner);
                }
            }
            candidates.extend(
                self.edge_source_constraint_opposite_owners(edge, constraint_index)
                    .into_iter()
                    .filter(|opposite_owner| {
                        owners_form_explicit_vertical_step_pair(edge.owner, *opposite_owner)
                    }),
            );

            let candidates = candidates.into_iter().collect::<Vec<_>>();
            if candidates.len() == 1 {
                return Some(candidates[0]);
            } else if edge.exposed_boundary
                && let Some(opposite_owner) =
                    self.edge_selected_source_constraint_opposite_owner(edge, constraint_index)
            {
                return Some(opposite_owner);
            }
        }

        if let Some(opposite_owner) = edge.opposite_owner {
            if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                && self.edge_has_owner_pair_endpoint_source_constraints_for_opposite(
                    edge,
                    opposite_owner,
                    constraint_index,
                )
            {
                return Some(opposite_owner);
            }
            if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                && self.edge_has_distributed_material_transition_point_sources_for_opposite(
                    edge,
                    opposite_owner,
                    constraint_index,
                )
            {
                return Some(opposite_owner);
            }
            if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                && self.edge_has_distributed_endpoint_source_constraints_for_opposite(
                    edge,
                    opposite_owner,
                    constraint_index,
                )
            {
                return Some(opposite_owner);
            }
        }
        let endpoint_path_candidates =
            self.edge_endpoint_material_path_step_opposite_owners(edge, constraint_index);
        if let Some(opposite_owner) =
            select_endpoint_path_step_opposite_owner(edge.owner, &endpoint_path_candidates)
        {
            return Some(opposite_owner);
        }
        let endpoint_candidates =
            self.edge_endpoint_source_constraint_opposite_owners(edge, constraint_index);
        if endpoint_candidates.len() == 1
            && owners_form_explicit_vertical_step_pair(edge.owner, endpoint_candidates[0])
        {
            return Some(endpoint_candidates[0]);
        }
        None
    }

    fn edge_selected_source_constraint_opposite_owner(
        &self,
        edge: &NodeArrangementEdge,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> Option<NodeBandOwner> {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return None;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return None;
        };
        let mut constraints = constraint_index
            .for_edge(edge)
            .filter(|constraint| seam_constraint_authorizes_explicit_height_split(constraint))
            .filter(|constraint| seam_constraint_covers_edge(constraint, start, end))
            .collect::<Vec<_>>();
        constraints
            .sort_by_key(|constraint| (constraint.priority_key(), constraint.constraint_index));
        constraints.dedup_by_key(|constraint| constraint.constraint_index);
        constraints.into_iter().find_map(|constraint| {
            let opposite_owner =
                seam_constraint_opposite_owner_for_edge_owner(constraint, edge.owner)?;
            owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                .then_some(opposite_owner)
        })
    }

    fn edge_source_constraint_opposite_owners(
        &self,
        edge: &NodeArrangementEdge,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> Vec<NodeBandOwner> {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let mut owners = constraint_index
            .for_edge(edge)
            .filter(|constraint| seam_constraint_authorizes_explicit_height_split(constraint))
            .filter(|constraint| seam_constraint_covers_edge(constraint, start, end))
            .filter_map(|constraint| {
                seam_constraint_opposite_owner_for_edge_owner(constraint, edge.owner)
            })
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();
        owners
    }

    fn edge_endpoint_source_constraint_opposite_owners(
        &self,
        edge: &NodeArrangementEdge,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> Vec<NodeBandOwner> {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let start_owners =
            self.endpoint_source_constraint_opposite_owners(edge.owner, start, constraint_index);
        let end_owners =
            self.endpoint_source_constraint_opposite_owners(edge.owner, end, constraint_index);
        let mut owners = start_owners
            .into_iter()
            .filter(|owner| end_owners.contains(owner))
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();
        owners
    }

    fn endpoint_source_constraint_opposite_owners(
        &self,
        owner: NodeBandOwner,
        key: NodeArrangementKey,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> Vec<NodeBandOwner> {
        let mut owners = constraint_index
            .at_key(key)
            .iter()
            .map(|indexed| indexed.constraint)
            .filter(|constraint| seam_constraint_authorizes_explicit_height_split(constraint))
            .filter_map(|constraint| {
                seam_constraint_opposite_owner_for_edge_owner(constraint, owner)
            })
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();
        owners
    }

    fn edge_has_owner_pair_endpoint_source_constraints_for_opposite(
        &self,
        edge: &NodeArrangementEdge,
        opposite_owner: NodeBandOwner,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> bool {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return false;
        };
        constraint_index.at_key(start).iter().any(|start_indexed| {
            let constraint = start_indexed.constraint;
            if !(seam_constraint_matches_owner_pair(constraint, edge.owner, opposite_owner)
                && seam_constraint_authorizes_explicit_height_split(constraint))
            {
                return false;
            }
            constraint_index.at_key(end).iter().any(|end_indexed| {
                end_indexed.region_index == start_indexed.region_index
                    && seam_constraint_matches_owner_pair(
                        end_indexed.constraint,
                        edge.owner,
                        opposite_owner,
                    )
                    && seam_constraint_authorizes_explicit_height_split(end_indexed.constraint)
            })
        })
    }

    fn edge_has_distributed_endpoint_source_constraints_for_opposite(
        &self,
        edge: &NodeArrangementEdge,
        opposite_owner: NodeBandOwner,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> bool {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return false;
        };
        self.endpoint_source_constraint_opposite_owners(edge.owner, start, constraint_index)
            .contains(&opposite_owner)
            && self
                .endpoint_source_constraint_opposite_owners(edge.owner, end, constraint_index)
                .contains(&opposite_owner)
    }

    fn edge_has_distributed_material_transition_point_sources_for_opposite(
        &self,
        edge: &NodeArrangementEdge,
        opposite_owner: NodeBandOwner,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> bool {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return false;
        };
        self.owner_pair_has_material_transition_point_sources_at_key(
            edge.owner,
            opposite_owner,
            start,
            constraint_index,
        ) && self.owner_pair_has_material_transition_point_sources_at_key(
            edge.owner,
            opposite_owner,
            end,
            constraint_index,
        )
    }

    fn edge_has_material_endpoint_path_for_opposite(
        &self,
        edge: &NodeArrangementEdge,
        opposite_owner: NodeBandOwner,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> bool {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return false;
        };
        constraint_index.has_material_endpoint_path(start, edge.owner, opposite_owner)
            && constraint_index.has_material_endpoint_path(end, edge.owner, opposite_owner)
    }

    fn edge_endpoint_material_path_step_opposite_owners(
        &self,
        edge: &NodeArrangementEdge,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> Vec<NodeBandOwner> {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let start_owners =
            self.material_endpoint_path_step_opposite_owners(edge.owner, start, constraint_index);
        let end_owners =
            self.material_endpoint_path_step_opposite_owners(edge.owner, end, constraint_index);
        let mut owners = start_owners
            .into_iter()
            .filter(|owner| end_owners.contains(owner))
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();
        owners
    }

    fn material_endpoint_path_step_opposite_owners(
        &self,
        owner: NodeBandOwner,
        key: NodeArrangementKey,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> Vec<NodeBandOwner> {
        let mut owners = constraint_index
            .at_key(key)
            .iter()
            .filter(|indexed| !indexed.constraint.constrains_shared_height)
            .flat_map(|indexed| {
                owners_for_material_seam_constraint(indexed.constraint, indexed.region_owner)
            })
            .filter(|candidate| {
                *candidate != owner
                    && owners_form_explicit_vertical_step_pair(owner, *candidate)
                    && constraint_index.has_material_endpoint_path(key, owner, *candidate)
            })
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();
        owners
    }

    fn owner_pair_has_material_transition_point_sources_at_key(
        &self,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        key: NodeArrangementKey,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> bool {
        self.owner_has_material_transition_source_at_key(
            owner,
            opposite_owner,
            key,
            false,
            constraint_index,
        ) && self.owner_has_material_transition_source_at_key(
            opposite_owner,
            owner,
            key,
            false,
            constraint_index,
        ) && (self.owner_has_material_transition_source_at_key(
            owner,
            opposite_owner,
            key,
            true,
            constraint_index,
        ) || self.owner_has_material_transition_source_at_key(
            opposite_owner,
            owner,
            key,
            true,
            constraint_index,
        ))
    }

    fn owner_has_material_transition_source_at_key(
        &self,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        key: NodeArrangementKey,
        require_height_split: bool,
        constraint_index: &ExplicitStepConstraintIndex<'_>,
    ) -> bool {
        constraint_index
            .at_key(key)
            .iter()
            .filter(|indexed| indexed.region_owner == owner)
            .any(|indexed| {
                let constraint = indexed.constraint;
                constraint.is_material_transition
                    && (!require_height_split || !constraint.constrains_shared_height)
                    && super::super::build::seam_constraint_can_source_region_owner_for_pair(
                        constraint,
                        owner,
                        opposite_owner,
                    )
            })
    }
}

struct ExplicitStepConstraintIndex<'a> {
    constraints_by_index: Vec<Vec<&'a NodeRegionSeamConstraint>>,
    constraints_by_key: BTreeMap<NodeArrangementKey, Vec<ExplicitStepIndexedConstraint<'a>>>,
    material_endpoint_adjacency_by_key:
        BTreeMap<NodeArrangementKey, BTreeMap<NodeBandOwner, BTreeSet<NodeBandOwner>>>,
}

impl<'a> ExplicitStepConstraintIndex<'a> {
    fn new(arrangement: &'a NodeArrangement) -> Self {
        let constraint_count = arrangement
            .regions
            .iter()
            .flat_map(|region| region.seam_constraints.iter())
            .map(|constraint| constraint.constraint_index)
            .max()
            .map_or(0, |max_index| max_index + 1);
        let mut constraints_by_index = vec![Vec::new(); constraint_count];
        for constraint in arrangement
            .regions
            .iter()
            .flat_map(|region| region.seam_constraints.iter())
        {
            constraints_by_index[constraint.constraint_index].push(constraint);
        }
        let relevant_keys = arrangement
            .vertices
            .iter()
            .map(|vertex| vertex.key)
            .collect::<BTreeSet<_>>();
        let relevant_key_index = ExplicitStepRelevantKeyIndex::new(&relevant_keys);
        let mut constraints_by_key =
            BTreeMap::<NodeArrangementKey, Vec<ExplicitStepIndexedConstraint<'_>>>::new();
        for (region_index, region) in arrangement.regions.iter().enumerate() {
            for constraint in &region.seam_constraints {
                if !constraint.is_material_transition {
                    continue;
                }
                let indexed = ExplicitStepIndexedConstraint {
                    region_index,
                    region_owner: region.owner,
                    constraint,
                };
                relevant_key_index.for_each_key_on_segment(
                    NodeArrangementKey::from_point(constraint.start_xz),
                    NodeArrangementKey::from_point(constraint.end_xz),
                    |key| constraints_by_key.entry(key).or_default().push(indexed),
                );
            }
        }
        let material_endpoint_adjacency_by_key = constraints_by_key
            .iter()
            .filter_map(|(&key, constraints)| {
                let mut owners_by_constraint = BTreeMap::<usize, Vec<NodeBandOwner>>::new();
                for indexed in constraints {
                    let constraint = indexed.constraint;
                    if constraint.constrains_shared_height {
                        continue;
                    }
                    let entry = owners_by_constraint
                        .entry(constraint.constraint_index)
                        .or_default();
                    for owner in
                        owners_for_material_seam_constraint(constraint, indexed.region_owner)
                    {
                        if let Err(insert_at) = entry.binary_search(&owner) {
                            entry.insert(insert_at, owner);
                        }
                    }
                }
                let mut adjacency = BTreeMap::<NodeBandOwner, BTreeSet<NodeBandOwner>>::new();
                for owners in owners_by_constraint.into_values() {
                    for left_index in 0..owners.len() {
                        for right_index in left_index + 1..owners.len() {
                            let left = owners[left_index];
                            let right = owners[right_index];
                            adjacency.entry(left).or_default().insert(right);
                            adjacency.entry(right).or_default().insert(left);
                        }
                    }
                }
                (!adjacency.is_empty()).then_some((key, adjacency))
            })
            .collect();
        Self {
            constraints_by_index,
            constraints_by_key,
            material_endpoint_adjacency_by_key,
        }
    }

    fn for_edge<'b>(
        &'b self,
        edge: &'b NodeArrangementEdge,
    ) -> impl Iterator<Item = &'a NodeRegionSeamConstraint> + 'b
    where
        'a: 'b,
    {
        edge.source_constraint_indices.iter().flat_map(|&index| {
            self.constraints_by_index
                .get(index)
                .into_iter()
                .flat_map(|constraints| constraints.iter().copied())
        })
    }

    fn at_key(&self, key: NodeArrangementKey) -> &[ExplicitStepIndexedConstraint<'a>] {
        self.constraints_by_key
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn has_material_endpoint_path(
        &self,
        key: NodeArrangementKey,
        left_owner: NodeBandOwner,
        right_owner: NodeBandOwner,
    ) -> bool {
        let Some(adjacency) = self.material_endpoint_adjacency_by_key.get(&key) else {
            return false;
        };
        let mut visited = BTreeSet::new();
        let mut pending = vec![left_owner];
        while let Some(owner) = pending.pop() {
            if !visited.insert(owner) {
                continue;
            }
            if owner == right_owner {
                return true;
            }
            if let Some(neighbors) = adjacency.get(&owner) {
                pending.extend(neighbors.iter().copied());
            }
        }
        false
    }
}

#[derive(Clone, Copy)]
struct ExplicitStepIndexedConstraint<'a> {
    region_index: usize,
    region_owner: NodeBandOwner,
    constraint: &'a NodeRegionSeamConstraint,
}

struct ExplicitStepRelevantKeyIndex {
    by_x: Vec<NodeArrangementKey>,
    by_z: Vec<NodeArrangementKey>,
}

impl ExplicitStepRelevantKeyIndex {
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

#[derive(Clone, Copy)]
struct ExposedRaisedStepEdge<'a> {
    edge: &'a NodeArrangementEdge,
    geometry: NodeArrangementEdgeGeometry,
    rank: u8,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

/// Quantized geometry and endpoint heights for one arrangement edge.
#[derive(Clone, Copy)]
pub(super) struct NodeArrangementEdgeGeometry {
    /// First quantized XZ endpoint.
    pub(super) start: NodeArrangementKey,
    /// Second quantized XZ endpoint.
    pub(super) end: NodeArrangementKey,
    /// Height at `start`, in canonical millimetres.
    pub(super) start_height_mm: i64,
    /// Height at `end`, in canonical millimetres.
    pub(super) end_height_mm: i64,
}

/// One owner-local final face-boundary edge and its source region.
#[derive(Clone, Copy, Debug)]
pub(super) struct NodeArrangementOwnedBoundaryEdgeGeometry {
    /// Owner whose final face exposes this edge.
    pub(super) owner: NodeBandOwner,
    /// Region supplying the edge's seam authority.
    pub(super) region: NodeOwnedRegionId,
    /// First quantized XZ endpoint.
    pub(super) start: NodeArrangementKey,
    /// Second quantized XZ endpoint.
    pub(super) end: NodeArrangementKey,
    /// Height at `start`, in canonical millimetres.
    pub(super) start_height_mm: i64,
    /// Height at `end`, in canonical millimetres.
    pub(super) end_height_mm: i64,
}

impl NodeArrangementOwnedBoundaryEdgeGeometry {
    fn geometry(self) -> NodeArrangementEdgeGeometry {
        NodeArrangementEdgeGeometry {
            start: self.start,
            end: self.end,
            start_height_mm: self.start_height_mm,
            end_height_mm: self.end_height_mm,
        }
    }
}

fn normalized_arrangement_key_pair(
    start: NodeArrangementKey,
    end: NodeArrangementKey,
) -> (NodeArrangementKey, NodeArrangementKey) {
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

/// Returns whether an existing owner-pair segment fully covers a boundary-derived candidate.
pub(super) fn explicit_segments_cover_owned_boundary_step(
    segments: &BTreeSet<NodeExplicitVerticalStepSegment>,
    candidate: NodeExplicitVerticalStepSegment,
) -> bool {
    segments.iter().copied().any(|segment| {
        owner_sets_match_step(
            &[candidate.owner()],
            &[candidate.opposite_owner()],
            segment.owner(),
            segment.opposite_owner(),
        ) && candidate
            .start()
            .lies_on_segment(segment.start(), segment.end())
            && candidate
                .end()
                .lies_on_segment(segment.start(), segment.end())
    })
}

/// Finds the positive exact/overlay-grid overlap shared by two quantized edges.
pub(super) fn arrangement_edge_overlap_segment(
    left: NodeArrangementEdgeGeometry,
    right: NodeArrangementEdgeGeometry,
) -> Option<(NodeArrangementKey, NodeArrangementKey)> {
    let left_start = left.start.surface_key();
    let left_end = left.end.surface_key();
    let right_start_t = arrangement_edge_endpoint_parameter_on_segment(
        right.start.surface_key(),
        left_start,
        left_end,
    )?;
    let right_end_t = arrangement_edge_endpoint_parameter_on_segment(
        right.end.surface_key(),
        left_start,
        left_end,
    )?;
    let low = right_start_t.min(right_end_t);
    let high = right_start_t.max(right_end_t);
    let start_t = low.max(SurfaceSegmentParameter::zero());
    let end_t = high.min(SurfaceSegmentParameter::one());
    if end_t <= start_t {
        return None;
    }
    let start =
        NodeArrangementKey::from_surface_key(interpolate_key(left_start, left_end, start_t));
    let end = NodeArrangementKey::from_surface_key(interpolate_key(left_start, left_end, end_t));
    (start != end).then_some((start, end))
}

fn arrangement_edge_endpoint_parameter_on_segment(
    point: SurfaceXzKey,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
) -> Option<SurfaceSegmentParameter> {
    overlay_segment_parameter(point, start, end)
        .or_else(|| exact_line_parameter(point, start, end))
        .or_else(|| arrangement_overlay_grid_line_parameter(point, start, end))
}

fn arrangement_overlay_grid_line_parameter(
    point: SurfaceXzKey,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
) -> Option<SurfaceSegmentParameter> {
    if !key_collinear_with_overlay_grid_segment(point, start, end) {
        return None;
    }
    let dx = i128::from(end.x_key() - start.x_key());
    let dz = i128::from(end.z_key() - start.z_key());
    SurfaceSegmentParameter::new(segment_parameter_key(start, end, point), dx * dx + dz * dz)
}

/// Returns whether the ranked raised edge stays at or above the lower edge and differs somewhere.
pub(super) fn arrangement_edges_have_positive_raised_step_delta(
    lower: NodeArrangementEdgeGeometry,
    raised: NodeArrangementEdgeGeometry,
    start: NodeArrangementKey,
    end: NodeArrangementKey,
) -> bool {
    let Some(lower_start) = arrangement_edge_height_mm_at_key(lower, start) else {
        return false;
    };
    let Some(lower_end) = arrangement_edge_height_mm_at_key(lower, end) else {
        return false;
    };
    let Some(raised_start) = arrangement_edge_height_mm_at_key(raised, start) else {
        return false;
    };
    let Some(raised_end) = arrangement_edge_height_mm_at_key(raised, end) else {
        return false;
    };
    let start_delta = raised_start - lower_start;
    let end_delta = raised_end - lower_end;
    start_delta >= 0 && end_delta >= 0 && (start_delta > 0 || end_delta > 0)
}

fn arrangement_edge_height_mm_at_key(
    edge: NodeArrangementEdgeGeometry,
    key: NodeArrangementKey,
) -> Option<i64> {
    let parameter = key
        .surface_key()
        .overlay_segment_parameter(edge.start.surface_key(), edge.end.surface_key())?;
    Some(interpolate_height_i64(
        edge.start_height_mm,
        edge.end_height_mm,
        parameter,
    ))
}

fn select_endpoint_path_step_opposite_owner(
    owner: NodeBandOwner,
    candidates: &[NodeBandOwner],
) -> Option<NodeBandOwner> {
    if candidates.len() == 1 {
        return Some(candidates[0]);
    }

    let mut cross_kind_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.kind() != owner.kind())
        .collect::<Vec<_>>();
    cross_kind_candidates.sort_unstable();
    cross_kind_candidates.dedup();
    if cross_kind_candidates.len() == 1 {
        Some(cross_kind_candidates[0])
    } else {
        None
    }
}

/// Returns whether a seam is explicit authority for an owner-pair height split.
pub(super) fn seam_constraint_authorizes_explicit_height_split(
    constraint: &NodeRegionSeamConstraint,
) -> bool {
    if !constraint.is_material_transition || constraint.constrains_shared_height {
        return false;
    }
    if matches!(
        constraint.seam_source,
        NodeSeamSource::RaisedStepContact { .. }
    ) {
        return true;
    }
    matches!(
        (constraint.owner, constraint.opposite_owner),
        (Some(owner), Some(opposite_owner))
            if owner.kind() == opposite_owner.kind()
                && owners_form_explicit_vertical_step_pair(owner, opposite_owner)
    )
}
