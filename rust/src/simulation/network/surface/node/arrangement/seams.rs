//! Seam source identity and source-authority helpers for node arrangements.

use super::super::RoadSurfaceBandKind;
use super::super::backend::RoadVec2;
use super::super::keys::SurfaceXzKey;
use super::super::segments::{key_collinear_with_overlay_grid_segment, segment_parameter_key};
use super::build::merge_sorted_unique;
use super::{NodeArrangement, NodeArrangementKey, NodeBandOwner};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeSeamSource {
    AsphaltBoundary { owner_index: usize },
    RaisedStepContact { owner_index: usize },
    SidewalkOuter { owner_index: usize },
    FootprintBoundary { owner_index: usize },
}

impl NodeSeamSource {
    pub(crate) fn priority_key(self) -> usize {
        match self {
            NodeSeamSource::RaisedStepContact { .. } => 0,
            NodeSeamSource::AsphaltBoundary { .. } => 1,
            NodeSeamSource::SidewalkOuter { .. } => 2,
            NodeSeamSource::FootprintBoundary { .. } => 3,
        }
    }

    pub(crate) fn for_owner(owner: NodeBandOwner) -> Self {
        match owner.kind() {
            RoadSurfaceBandKind::Carriageway => NodeSeamSource::AsphaltBoundary {
                owner_index: owner.owner_index(),
            },
            RoadSurfaceBandKind::CurbOrShoulder => NodeSeamSource::RaisedStepContact {
                owner_index: owner.owner_index(),
            },
            RoadSurfaceBandKind::Sidewalk => NodeSeamSource::SidewalkOuter {
                owner_index: owner.owner_index(),
            },
            _ => NodeSeamSource::FootprintBoundary {
                owner_index: owner.owner_index(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeRegionSeamConstraint {
    pub(crate) constraint_index: usize,
    pub(crate) seam_source: NodeSeamSource,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) opposite_owner: Option<NodeBandOwner>,
    pub(crate) constrains_shared_height: bool,
    pub(crate) is_material_transition: bool,
    pub(crate) start_xz: RoadVec2,
    pub(crate) end_xz: RoadVec2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SeamConstraintCoverageKey {
    priority_key: (bool, bool, usize),
    constraint_index: usize,
    seam_source: NodeSeamSource,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    constrains_shared_height: bool,
    is_material_transition: bool,
}

impl NodeRegionSeamConstraint {
    pub(crate) fn priority_key(&self) -> (bool, bool, usize) {
        (
            !self.constrains_shared_height,
            !self.is_material_transition,
            self.seam_source.priority_key(),
        )
    }
}

pub(super) fn seam_constraint_touches_key(
    constraint: &NodeRegionSeamConstraint,
    key: NodeArrangementKey,
) -> bool {
    let start = NodeArrangementKey::from_point(constraint.start_xz);
    let end = NodeArrangementKey::from_point(constraint.end_xz);
    key.lies_on_segment(start, end)
}

pub(super) fn seam_constraint_matches_owner_pair(
    constraint: &NodeRegionSeamConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    (constraint.owner == Some(owner) && constraint.opposite_owner == Some(opposite_owner))
        || (constraint.owner == Some(opposite_owner) && constraint.opposite_owner == Some(owner))
}

pub(super) fn seam_constraint_opposite_owner_for_edge_owner(
    constraint: &NodeRegionSeamConstraint,
    owner: NodeBandOwner,
) -> Option<NodeBandOwner> {
    match (constraint.owner, constraint.opposite_owner) {
        (Some(left), Some(right)) if left == owner => Some(right),
        (Some(left), Some(right)) if right == owner => Some(left),
        _ => None,
    }
}

pub(super) fn seam_constraint_covers_edge(
    constraint: &NodeRegionSeamConstraint,
    edge_start: NodeArrangementKey,
    edge_end: NodeArrangementKey,
) -> bool {
    let constraint_start = NodeArrangementKey::from_point(constraint.start_xz);
    let constraint_end = NodeArrangementKey::from_point(constraint.end_xz);
    edge_start.lies_on_segment(constraint_start, constraint_end)
        && edge_end.lies_on_segment(constraint_start, constraint_end)
}

pub(in crate::simulation::network::surface::node) fn seam_constraints_covering_surface_key_edge_as_fragments<
    'a,
>(
    start: SurfaceXzKey,
    end: SurfaceXzKey,
    constraints: &'a [NodeRegionSeamConstraint],
) -> Vec<&'a NodeRegionSeamConstraint> {
    if start == end {
        return Vec::new();
    }
    let edge_end_parameter = segment_parameter_key(start, end, end);
    if edge_end_parameter <= 0 {
        return Vec::new();
    }

    let mut intervals_by_source = BTreeMap::<
        SeamConstraintCoverageKey,
        Vec<(i128, i128, &'a NodeRegionSeamConstraint)>,
    >::new();
    for constraint in constraints {
        let Some((overlap_start, overlap_end)) =
            seam_constraint_overlap_interval(start, end, edge_end_parameter, constraint)
        else {
            continue;
        };
        intervals_by_source
            .entry(seam_constraint_coverage_key(constraint))
            .or_default()
            .push((overlap_start, overlap_end, constraint));
    }

    let mut matches = Vec::new();
    for intervals in intervals_by_source.values_mut() {
        intervals.sort_by_key(|(start, end, _)| (*start, *end));
        let mut covered_end = 0;
        let mut covered_constraints = Vec::new();
        for (interval_start, interval_end, constraint) in intervals {
            if *interval_start > covered_end {
                break;
            }
            covered_end = covered_end.max(*interval_end);
            covered_constraints.push(*constraint);
            if covered_end >= edge_end_parameter {
                matches.extend(covered_constraints);
                break;
            }
        }
    }
    matches
}

fn seam_constraint_overlap_interval(
    start: SurfaceXzKey,
    end: SurfaceXzKey,
    edge_end_parameter: i128,
    constraint: &NodeRegionSeamConstraint,
) -> Option<(i128, i128)> {
    let constraint_start = SurfaceXzKey::from_road_xz(constraint.start_xz);
    let constraint_end = SurfaceXzKey::from_road_xz(constraint.end_xz);
    if constraint_start == constraint_end
        || !key_collinear_with_overlay_grid_segment(constraint_start, start, end)
        || !key_collinear_with_overlay_grid_segment(constraint_end, start, end)
    {
        return None;
    }
    let start_parameter = segment_parameter_key(start, end, constraint_start);
    let end_parameter = segment_parameter_key(start, end, constraint_end);
    let overlap_start = start_parameter.min(end_parameter).max(0);
    let overlap_end = start_parameter.max(end_parameter).min(edge_end_parameter);
    (overlap_start < overlap_end).then_some((overlap_start, overlap_end))
}

fn seam_constraint_coverage_key(
    constraint: &NodeRegionSeamConstraint,
) -> SeamConstraintCoverageKey {
    SeamConstraintCoverageKey {
        priority_key: constraint.priority_key(),
        constraint_index: constraint.constraint_index,
        seam_source: constraint.seam_source,
        owner: constraint.owner,
        opposite_owner: constraint.opposite_owner,
        constrains_shared_height: constraint.constrains_shared_height,
        is_material_transition: constraint.is_material_transition,
    }
}

pub(super) fn seam_constraint_covers_key(
    constraint: &NodeRegionSeamConstraint,
    key: NodeArrangementKey,
) -> bool {
    let constraint_start = NodeArrangementKey::from_point(constraint.start_xz);
    let constraint_end = NodeArrangementKey::from_point(constraint.end_xz);
    key.lies_on_segment(constraint_start, constraint_end)
}

pub(super) fn seam_constraint_can_source_edge_owner_pair(
    constraint: &NodeRegionSeamConstraint,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
) -> bool {
    match (constraint.owner, constraint.opposite_owner) {
        (Some(_), Some(_)) => opposite_owner.is_some_and(|opposite_owner| {
            seam_constraint_matches_owner_pair(constraint, owner, opposite_owner)
        }),
        (Some(constraint_owner), None) | (None, Some(constraint_owner)) => {
            constraint_owner == owner || opposite_owner == Some(constraint_owner)
        }
        (None, None) => true,
    }
}

pub(crate) fn seam_constraints_are_ambiguous(constraints: &[&NodeRegionSeamConstraint]) -> bool {
    let Some(first) = constraints.first() else {
        return false;
    };
    let first_priority = first.priority_key();
    constraints
        .iter()
        .skip(1)
        .take_while(|constraint| constraint.priority_key() == first_priority)
        .any(|constraint| constraint.seam_source != first.seam_source)
}

pub(super) fn owners_for_material_seam_constraint(
    constraint: &NodeRegionSeamConstraint,
    region_owner: NodeBandOwner,
) -> Vec<NodeBandOwner> {
    match (constraint.owner, constraint.opposite_owner) {
        (Some(owner), Some(opposite_owner)) => vec![owner, opposite_owner],
        (Some(owner), None) | (None, Some(owner)) => vec![owner],
        (None, None) => vec![region_owner],
    }
}

impl NodeArrangement {
    pub(super) fn has_explicit_material_seam_endpoint_path_at_key_between(
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
        for region in &self.regions {
            for constraint in &region.seam_constraints {
                if constraint.constrains_shared_height
                    || !constraint.is_material_transition
                    || !seam_constraint_touches_key(constraint, key)
                {
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
        adjacency
    }
}
