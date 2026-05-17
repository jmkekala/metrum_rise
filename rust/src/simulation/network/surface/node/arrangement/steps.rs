//! Explicit vertical-step authority extraction from canonical arrangement edges.

use super::super::band_semantics::{ordered_raised_step_kinds, raised_step_band_rank};
use super::seams::{
    seam_constraint_covers_edge, seam_constraint_covers_key, seam_constraint_matches_owner_pair,
    seam_constraint_opposite_owner_for_edge_owner,
};
use super::{NodeArrangement, NodeArrangementEdge, NodeArrangementKey, NodeBandOwner};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeExplicitVerticalStepSegment {
    start: NodeArrangementKey,
    end: NodeArrangementKey,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
}

impl NodeExplicitVerticalStepSegment {
    pub(crate) fn new(
        a: NodeArrangementKey,
        b: NodeArrangementKey,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
    ) -> Option<Self> {
        if a == b {
            return None;
        }
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let (owner, opposite_owner) = if owner <= opposite_owner {
            (owner, opposite_owner)
        } else {
            (opposite_owner, owner)
        };
        Some(Self {
            start,
            end,
            owner,
            opposite_owner,
        })
    }

    pub(crate) fn start(self) -> NodeArrangementKey {
        self.start
    }

    pub(crate) fn end(self) -> NodeArrangementKey {
        self.end
    }

    pub(crate) fn owner(self) -> NodeBandOwner {
        self.owner
    }

    pub(crate) fn opposite_owner(self) -> NodeBandOwner {
        self.opposite_owner
    }
}

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

    pub(super) fn edge_has_owner_pair_source_constraint(&self, edge: &NodeArrangementEdge) -> bool {
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
        if !edge.is_material_transition || edge.constrains_shared_height {
            return None;
        }

        let mut candidates = BTreeSet::new();
        if let Some(opposite_owner) = edge.opposite_owner {
            if owners_form_explicit_vertical_step_pair(edge.owner, opposite_owner)
                && (self.edge_has_owner_pair_source_constraint_for_opposite(edge, opposite_owner)
                    || self.edge_has_owner_pair_endpoint_source_constraints_for_opposite(
                        edge,
                        opposite_owner,
                    ))
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
            Some(candidates[0])
        } else if edge.exposed_boundary {
            self.edge_selected_source_constraint_opposite_owner(edge)
        } else {
            None
        }
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
                    && seam_constraint_covers_key(constraint, start)
            });
            let has_end = region.seam_constraints.iter().any(|constraint| {
                seam_constraint_matches_owner_pair(constraint, edge.owner, opposite_owner)
                    && constraint.is_material_transition
                    && !constraint.constrains_shared_height
                    && seam_constraint_covers_key(constraint, end)
            });
            has_start && has_end
        })
    }

    pub(super) fn has_explicit_vertical_step_at_key_between(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        self.explicit_vertical_step_segments()
            .into_iter()
            .any(|segment| {
                key.lies_on_segment(segment.start(), segment.end())
                    && owner_sets_match_step(
                        left_owners,
                        right_owners,
                        segment.owner(),
                        segment.opposite_owner(),
                    )
            })
    }
}

pub(crate) fn owners_form_explicit_vertical_step_pair(a: NodeBandOwner, b: NodeBandOwner) -> bool {
    if a == b {
        return false;
    }
    if a.kind() == b.kind() {
        return raised_step_band_rank(a.kind()).is_some();
    }
    ordered_raised_step_kinds(a.kind(), b.kind()).is_some()
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
