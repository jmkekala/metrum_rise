//! Seam source identity and source-authority helpers for node arrangements.

use super::super::RoadSurfaceBandKind;
use super::super::backend::RoadVec2;
use super::super::keys::SurfaceXzKey;
use super::super::segments::{key_collinear_with_overlay_grid_segment, segment_parameter_key};
use super::{NodeArrangementKey, NodeBandOwner};
use std::collections::HashMap;

const SEAM_COVERAGE_TILE_KEYS: i64 = 8_000_000;
const SEAM_COVERAGE_MAX_INDEX_TILES: i64 = 256;
const OVERLAY_GRID_AXIS_TOLERANCE_KEYS: i64 = 2;

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

#[derive(Default)]
pub(in crate::simulation::network::surface::node) struct SeamConstraintCoverageScratch {
    intervals: Vec<(SeamConstraintCoverageKey, i128, i128, usize)>,
    candidate_indices: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SeamCoverageTile {
    x: i64,
    z: i64,
}

#[derive(Clone, Copy)]
struct PreparedSeamConstraintCoverage {
    start: SurfaceXzKey,
    end: SurfaceXzKey,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

pub(in crate::simulation::network::surface::node) struct PreparedSeamConstraintCoverages<'a> {
    constraints: &'a [NodeRegionSeamConstraint],
    prepared: Vec<PreparedSeamConstraintCoverage>,
    constraint_indices_by_tile: HashMap<SeamCoverageTile, Vec<usize>>,
    global_constraint_indices: Vec<usize>,
}

impl<'a> PreparedSeamConstraintCoverages<'a> {
    pub(in crate::simulation::network::surface::node) fn new(
        constraints: &'a [NodeRegionSeamConstraint],
    ) -> Self {
        let prepared = constraints
            .iter()
            .map(|constraint| {
                let start = SurfaceXzKey::from_road_xz(constraint.start_xz);
                let end = SurfaceXzKey::from_road_xz(constraint.end_xz);
                PreparedSeamConstraintCoverage {
                    start,
                    end,
                    min_x: start
                        .x_key()
                        .min(end.x_key())
                        .saturating_sub(OVERLAY_GRID_AXIS_TOLERANCE_KEYS),
                    min_z: start
                        .z_key()
                        .min(end.z_key())
                        .saturating_sub(OVERLAY_GRID_AXIS_TOLERANCE_KEYS),
                    max_x: start
                        .x_key()
                        .max(end.x_key())
                        .saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS),
                    max_z: start
                        .z_key()
                        .max(end.z_key())
                        .saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS),
                }
            })
            .collect::<Vec<_>>();
        let mut constraint_indices_by_tile = HashMap::new();
        let mut global_constraint_indices = Vec::new();
        for (constraint_index, constraint) in prepared.iter().enumerate() {
            if constraint.start == constraint.end {
                continue;
            }
            index_seam_coverage_constraint(
                &mut constraint_indices_by_tile,
                &mut global_constraint_indices,
                constraint_index,
                constraint,
            );
        }
        Self {
            constraints,
            prepared,
            constraint_indices_by_tile,
            global_constraint_indices,
        }
    }

    fn candidate_indices_for_bounds(
        &self,
        min_x: i64,
        min_z: i64,
        max_x: i64,
        max_z: i64,
        scratch: &mut SeamConstraintCoverageScratch,
    ) {
        scratch.candidate_indices.clear();
        let min_tile = SeamCoverageTile {
            x: min_x.div_euclid(SEAM_COVERAGE_TILE_KEYS),
            z: min_z.div_euclid(SEAM_COVERAGE_TILE_KEYS),
        };
        let max_tile = SeamCoverageTile {
            x: max_x.div_euclid(SEAM_COVERAGE_TILE_KEYS),
            z: max_z.div_euclid(SEAM_COVERAGE_TILE_KEYS),
        };
        let tile_width = max_tile.x.saturating_sub(min_tile.x).saturating_add(1);
        let tile_height = max_tile.z.saturating_sub(min_tile.z).saturating_add(1);
        if tile_width.saturating_mul(tile_height) > SEAM_COVERAGE_MAX_INDEX_TILES {
            scratch.candidate_indices.extend(0..self.prepared.len());
        } else {
            scratch
                .candidate_indices
                .extend_from_slice(&self.global_constraint_indices);
            for x in min_tile.x..=max_tile.x {
                for z in min_tile.z..=max_tile.z {
                    if let Some(indices) = self
                        .constraint_indices_by_tile
                        .get(&SeamCoverageTile { x, z })
                    {
                        scratch.candidate_indices.extend_from_slice(indices);
                    }
                }
            }
            if min_tile != max_tile || !self.global_constraint_indices.is_empty() {
                scratch.candidate_indices.sort_unstable();
                scratch.candidate_indices.dedup();
            }
        }
        scratch.candidate_indices.retain(|&constraint_index| {
            prepared_seam_constraint_bounds_overlap(
                &self.prepared[constraint_index],
                min_x,
                min_z,
                max_x,
                max_z,
            )
        });
    }
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

pub(in crate::simulation::network::surface::node) fn prepared_seam_constraints_covering_surface_key_edge_as_fragments_into<
    'a,
>(
    start: SurfaceXzKey,
    end: SurfaceXzKey,
    constraints: &PreparedSeamConstraintCoverages<'a>,
    scratch: &mut SeamConstraintCoverageScratch,
    matches: &mut Vec<&'a NodeRegionSeamConstraint>,
) {
    matches.clear();
    if start == end {
        return;
    }
    let edge_end_parameter = segment_parameter_key(start, end, end);
    if edge_end_parameter <= 0 {
        return;
    }
    let min_x = start.x_key().min(end.x_key());
    let min_z = start.z_key().min(end.z_key());
    let max_x = start.x_key().max(end.x_key());
    let max_z = start.z_key().max(end.z_key());
    constraints.candidate_indices_for_bounds(min_x, min_z, max_x, max_z, scratch);
    scratch.intervals.clear();
    scratch.intervals.reserve(scratch.candidate_indices.len());
    for &constraint_index in &scratch.candidate_indices {
        let prepared = &constraints.prepared[constraint_index];
        let Some((overlap_start, overlap_end)) = seam_constraint_key_overlap_interval(
            start,
            end,
            edge_end_parameter,
            prepared.start,
            prepared.end,
        ) else {
            continue;
        };
        let constraint = &constraints.constraints[constraint_index];
        scratch.intervals.push((
            seam_constraint_coverage_key(constraint),
            overlap_start,
            overlap_end,
            constraint_index,
        ));
    }
    append_fully_covering_seam_constraint_groups(
        edge_end_parameter,
        constraints.constraints,
        scratch,
        matches,
    );
}

fn prepared_seam_constraint_bounds_overlap(
    constraint: &PreparedSeamConstraintCoverage,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
) -> bool {
    constraint.min_x <= max_x
        && min_x <= constraint.max_x
        && constraint.min_z <= max_z
        && min_z <= constraint.max_z
}

fn index_seam_coverage_constraint(
    constraint_indices_by_tile: &mut HashMap<SeamCoverageTile, Vec<usize>>,
    global_constraint_indices: &mut Vec<usize>,
    constraint_index: usize,
    constraint: &PreparedSeamConstraintCoverage,
) {
    let min_tile = SeamCoverageTile {
        x: constraint.min_x.div_euclid(SEAM_COVERAGE_TILE_KEYS),
        z: constraint.min_z.div_euclid(SEAM_COVERAGE_TILE_KEYS),
    };
    let max_tile = SeamCoverageTile {
        x: constraint.max_x.div_euclid(SEAM_COVERAGE_TILE_KEYS),
        z: constraint.max_z.div_euclid(SEAM_COVERAGE_TILE_KEYS),
    };
    let tile_width = max_tile.x.saturating_sub(min_tile.x).saturating_add(1);
    let tile_height = max_tile.z.saturating_sub(min_tile.z).saturating_add(1);
    if tile_width.saturating_mul(tile_height) > SEAM_COVERAGE_MAX_INDEX_TILES {
        global_constraint_indices.push(constraint_index);
        return;
    }
    for x in min_tile.x..=max_tile.x {
        for z in min_tile.z..=max_tile.z {
            constraint_indices_by_tile
                .entry(SeamCoverageTile { x, z })
                .or_default()
                .push(constraint_index);
        }
    }
}

fn append_fully_covering_seam_constraint_groups<'a>(
    edge_end_parameter: i128,
    constraints: &'a [NodeRegionSeamConstraint],
    scratch: &mut SeamConstraintCoverageScratch,
    matches: &mut Vec<&'a NodeRegionSeamConstraint>,
) {
    scratch
        .intervals
        .sort_by_key(|(source, start, end, _)| (*source, *start, *end));

    let mut group_start = 0;
    while group_start < scratch.intervals.len() {
        let source = scratch.intervals[group_start].0;
        let group_end = scratch.intervals[group_start..]
            .partition_point(|(candidate, _, _, _)| *candidate == source)
            + group_start;
        let mut covered_end = 0;
        for candidate_index in group_start..group_end {
            let (_, interval_start, interval_end, _) = scratch.intervals[candidate_index];
            if interval_start > covered_end {
                break;
            }
            covered_end = covered_end.max(interval_end);
            if covered_end >= edge_end_parameter {
                matches.extend(
                    scratch.intervals[group_start..=candidate_index]
                        .iter()
                        .map(|(_, _, _, constraint_index)| &constraints[*constraint_index]),
                );
                break;
            }
        }
        group_start = group_end;
    }
}

fn seam_constraint_key_overlap_interval(
    start: SurfaceXzKey,
    end: SurfaceXzKey,
    edge_end_parameter: i128,
    constraint_start: SurfaceXzKey,
    constraint_end: SurfaceXzKey,
) -> Option<(i128, i128)> {
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
) -> impl Iterator<Item = NodeBandOwner> {
    let owners = match (constraint.owner, constraint.opposite_owner) {
        (Some(owner), Some(opposite_owner)) => [Some(owner), Some(opposite_owner)],
        (Some(owner), None) | (None, Some(owner)) => [Some(owner), None],
        (None, None) => [Some(region_owner), None],
    };
    owners.into_iter().flatten()
}
