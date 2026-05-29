//! Explicit vertical-step extraction from arrangement edges.

use super::*;

impl NodeArrangement {
    pub(crate) fn explicit_vertical_step_segments(&self) -> Vec<NodeExplicitVerticalStepSegment> {
        let (segments, _) = self.explicit_vertical_step_segment_sets();
        segments.into_iter().collect()
    }

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
        let mut segments = BTreeSet::new();
        for edge in &self.edges {
            let Some(opposite_owner) = self.edge_explicit_vertical_step_opposite_owner(edge) else {
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
        let source_segments = segments.clone();
        self.extend_exposed_owned_raised_step_overlap_segments(&mut segments);
        let mut derived_overlap_segments = segments
            .difference(&source_segments)
            .copied()
            .collect::<BTreeSet<_>>();
        let final_boundary_segments =
            self.authorized_final_boundary_raised_step_overlap_segments(&segments);
        derived_overlap_segments.extend(final_boundary_segments.iter().copied());
        segments.extend(final_boundary_segments.iter().copied());
        (segments, derived_overlap_segments)
    }

    fn extend_exposed_owned_raised_step_overlap_segments(
        &self,
        segments: &mut BTreeSet<NodeExplicitVerticalStepSegment>,
    ) {
        for left_index in 0..self.edges.len() {
            for right_edge in self.edges.iter().skip(left_index + 1) {
                let left_edge = &self.edges[left_index];
                if let Some(segment) =
                    self.exposed_owned_raised_step_overlap_segment(left_edge, right_edge)
                {
                    segments.insert(segment);
                }
            }
        }
    }

    fn exposed_owned_raised_step_overlap_segment(
        &self,
        left_edge: &NodeArrangementEdge,
        right_edge: &NodeArrangementEdge,
    ) -> Option<NodeExplicitVerticalStepSegment> {
        if !left_edge.exposed_boundary || !right_edge.exposed_boundary {
            return None;
        }
        if !raised_step_kinds_can_contact(left_edge.owner.kind(), right_edge.owner.kind()) {
            return None;
        }
        let left_rank = raised_step_band_rank(left_edge.owner.kind())?;
        let right_rank = raised_step_band_rank(right_edge.owner.kind())?;
        if left_rank == right_rank {
            return None;
        }

        let left = self.arrangement_edge_geometry(left_edge)?;
        let right = self.arrangement_edge_geometry(right_edge)?;
        let (start, end) = arrangement_edge_overlap_segment(left, right)?;
        let (lower_edge, lower, raised_edge, raised) = if left_rank < right_rank {
            (left_edge, left, right_edge, right)
        } else {
            (right_edge, right, left_edge, left)
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
        let mut segments = BTreeSet::new();
        let boundary_edges = self.final_owned_boundary_edge_geometries();
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

    fn final_owned_boundary_edge_geometries(
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

    fn edge_explicit_vertical_step_opposite_owner(
        &self,
        edge: &NodeArrangementEdge,
    ) -> Option<NodeBandOwner> {
        if edge.constrains_shared_height {
            if let Some(opposite_owner) = edge.opposite_owner
                && edge.owner.kind() == opposite_owner.kind()
                && owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                && (self.edge_has_owner_pair_endpoint_source_constraints_for_opposite(
                    edge,
                    opposite_owner,
                ) || self.edge_has_distributed_endpoint_source_constraints_for_opposite(
                    edge,
                    opposite_owner,
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
                    && self.edge_has_owner_pair_source_constraint_for_opposite(edge, opposite_owner)
                {
                    return Some(opposite_owner);
                }
                if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                    && self.edge_has_material_endpoint_path_for_opposite(edge, opposite_owner)
                {
                    return Some(opposite_owner);
                }
            }
            candidates.extend(
                self.edge_source_constraint_opposite_owners(edge)
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
                    self.edge_selected_source_constraint_opposite_owner(edge)
            {
                return Some(opposite_owner);
            }
        }

        if let Some(opposite_owner) = edge.opposite_owner {
            if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                && self.edge_has_owner_pair_endpoint_source_constraints_for_opposite(
                    edge,
                    opposite_owner,
                )
            {
                return Some(opposite_owner);
            }
            if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                && self.edge_has_distributed_material_transition_point_sources_for_opposite(
                    edge,
                    opposite_owner,
                )
            {
                return Some(opposite_owner);
            }
            if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                && self.edge_has_distributed_endpoint_source_constraints_for_opposite(
                    edge,
                    opposite_owner,
                )
            {
                return Some(opposite_owner);
            }
        }
        let endpoint_path_candidates = self.edge_endpoint_material_path_step_opposite_owners(edge);
        if let Some(opposite_owner) =
            select_endpoint_path_step_opposite_owner(edge.owner, &endpoint_path_candidates)
        {
            return Some(opposite_owner);
        }
        let endpoint_candidates = self.edge_endpoint_source_constraint_opposite_owners(edge);
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
    ) -> Option<NodeBandOwner> {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return None;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return None;
        };
        let mut constraints = self
            .regions
            .iter()
            .flat_map(|region| region.seam_constraints.iter())
            .filter(|constraint| seam_constraint_authorizes_explicit_height_split(constraint))
            .filter(|constraint| {
                edge.source_constraint_indices
                    .contains(&constraint.constraint_index)
                    && seam_constraint_covers_edge(constraint, start, end)
            })
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
    ) -> Vec<NodeBandOwner> {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let mut owners = self
            .regions
            .iter()
            .flat_map(|region| region.seam_constraints.iter())
            .filter(|constraint| seam_constraint_authorizes_explicit_height_split(constraint))
            .filter(|constraint| {
                edge.source_constraint_indices
                    .contains(&constraint.constraint_index)
                    && seam_constraint_covers_edge(constraint, start, end)
            })
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
    ) -> Vec<NodeBandOwner> {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let start_owners = self.endpoint_source_constraint_opposite_owners(edge.owner, start);
        let end_owners = self.endpoint_source_constraint_opposite_owners(edge.owner, end);
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
    ) -> Vec<NodeBandOwner> {
        let mut owners = self
            .regions
            .iter()
            .flat_map(|region| region.seam_constraints.iter())
            .filter(|constraint| seam_constraint_authorizes_explicit_height_split(constraint))
            .filter(|constraint| seam_constraint_covers_key(constraint, key))
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
    ) -> bool {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return false;
        };
        self.regions.iter().any(|region| {
            let has_start = region.seam_constraints.iter().any(|constraint| {
                seam_constraint_matches_owner_pair(constraint, edge.owner, opposite_owner)
                    && seam_constraint_authorizes_explicit_height_split(constraint)
                    && seam_constraint_covers_key(constraint, start)
            });
            let has_end = region.seam_constraints.iter().any(|constraint| {
                seam_constraint_matches_owner_pair(constraint, edge.owner, opposite_owner)
                    && seam_constraint_authorizes_explicit_height_split(constraint)
                    && seam_constraint_covers_key(constraint, end)
            });
            has_start && has_end
        })
    }

    fn edge_has_distributed_endpoint_source_constraints_for_opposite(
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
        self.endpoint_source_constraint_opposite_owners(edge.owner, start)
            .contains(&opposite_owner)
            && self
                .endpoint_source_constraint_opposite_owners(edge.owner, end)
                .contains(&opposite_owner)
    }

    fn edge_has_distributed_material_transition_point_sources_for_opposite(
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
        self.owner_pair_has_material_transition_point_sources_at_key(
            edge.owner,
            opposite_owner,
            start,
        ) && self.owner_pair_has_material_transition_point_sources_at_key(
            edge.owner,
            opposite_owner,
            end,
        )
    }

    fn edge_has_material_endpoint_path_for_opposite(
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
        self.has_explicit_material_seam_endpoint_path_at_key_between(
            start,
            &[edge.owner],
            &[opposite_owner],
        ) && self.has_explicit_material_seam_endpoint_path_at_key_between(
            end,
            &[edge.owner],
            &[opposite_owner],
        )
    }

    fn edge_endpoint_material_path_step_opposite_owners(
        &self,
        edge: &NodeArrangementEdge,
    ) -> Vec<NodeBandOwner> {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return Vec::new();
        };
        let start_owners = self.material_endpoint_path_step_opposite_owners(edge.owner, start);
        let end_owners = self.material_endpoint_path_step_opposite_owners(edge.owner, end);
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
    ) -> Vec<NodeBandOwner> {
        let mut owners = self
            .regions
            .iter()
            .flat_map(|region| {
                region
                    .seam_constraints
                    .iter()
                    .filter(move |constraint| {
                        constraint.is_material_transition
                            && !constraint.constrains_shared_height
                            && seam_constraint_covers_key(constraint, key)
                    })
                    .flat_map(move |constraint| {
                        owners_for_material_seam_constraint(constraint, region.owner)
                    })
            })
            .filter(|candidate| {
                *candidate != owner
                    && owners_form_explicit_vertical_step_pair(owner, *candidate)
                    && self.has_explicit_material_seam_endpoint_path_at_key_between(
                        key,
                        &[owner],
                        &[*candidate],
                    )
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
    ) -> bool {
        self.owner_has_material_transition_source_at_key(owner, opposite_owner, key, false)
            && self.owner_has_material_transition_source_at_key(opposite_owner, owner, key, false)
            && (self.owner_has_material_transition_source_at_key(owner, opposite_owner, key, true)
                || self.owner_has_material_transition_source_at_key(
                    opposite_owner,
                    owner,
                    key,
                    true,
                ))
    }

    fn owner_has_material_transition_source_at_key(
        &self,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        key: NodeArrangementKey,
        require_height_split: bool,
    ) -> bool {
        self.regions
            .iter()
            .filter(|region| region.owner == owner)
            .flat_map(|region| region.seam_constraints.iter())
            .any(|constraint| {
                constraint.is_material_transition
                    && (!require_height_split || !constraint.constrains_shared_height)
                    && super::super::build::seam_constraint_can_source_region_owner_for_pair(
                        constraint,
                        owner,
                        opposite_owner,
                    )
                    && seam_constraint_covers_key(constraint, key)
            })
    }
}

#[derive(Clone, Copy)]
struct NodeArrangementEdgeGeometry {
    start: NodeArrangementKey,
    end: NodeArrangementKey,
    start_height_mm: i64,
    end_height_mm: i64,
}

#[derive(Clone, Copy, Debug)]
struct NodeArrangementOwnedBoundaryEdgeGeometry {
    owner: NodeBandOwner,
    region: NodeOwnedRegionId,
    start: NodeArrangementKey,
    end: NodeArrangementKey,
    start_height_mm: i64,
    end_height_mm: i64,
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

fn explicit_segments_cover_owned_boundary_step(
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

fn arrangement_edge_overlap_segment(
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

fn arrangement_edges_have_positive_raised_step_delta(
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

fn seam_constraint_authorizes_explicit_height_split(constraint: &NodeRegionSeamConstraint) -> bool {
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
