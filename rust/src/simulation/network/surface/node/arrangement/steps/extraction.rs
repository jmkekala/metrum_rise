//! Explicit vertical-step extraction from arrangement edges.

use super::*;

impl NodeArrangement {
    pub(crate) fn explicit_vertical_step_segments(&self) -> Vec<NodeExplicitVerticalStepSegment> {
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
        segments.into_iter().collect()
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
                    && constraint.is_material_transition
                    && !constraint.constrains_shared_height
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
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
            .filter(|constraint| constraint.is_material_transition)
            .filter(|constraint| !constraint.constrains_shared_height)
            .filter(|constraint| {
                matches!(
                    constraint.seam_source,
                    NodeSeamSource::RaisedStepContact { .. }
                )
            })
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
            .filter(|constraint| constraint.is_material_transition)
            .filter(|constraint| !constraint.constrains_shared_height)
            .filter(|constraint| {
                matches!(
                    constraint.seam_source,
                    NodeSeamSource::RaisedStepContact { .. }
                )
            })
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
            .filter(|constraint| constraint.is_material_transition)
            .filter(|constraint| !constraint.constrains_shared_height)
            .filter(|constraint| {
                matches!(
                    constraint.seam_source,
                    NodeSeamSource::RaisedStepContact { .. }
                )
            })
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
                    && constraint.is_material_transition
                    && !constraint.constrains_shared_height
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
                    && seam_constraint_covers_key(constraint, start)
            });
            let has_end = region.seam_constraints.iter().any(|constraint| {
                seam_constraint_matches_owner_pair(constraint, edge.owner, opposite_owner)
                    && constraint.is_material_transition
                    && !constraint.constrains_shared_height
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
                    && seam_constraint_covers_key(constraint, end)
            });
            has_start && has_end
        })
    }

    pub(in crate::simulation::network::surface::node::arrangement) fn has_explicit_vertical_step_at_key_between(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        let segments = self.explicit_vertical_step_segments();
        segments.iter().copied().any(|segment| {
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
            &segments,
        )
    }
}
