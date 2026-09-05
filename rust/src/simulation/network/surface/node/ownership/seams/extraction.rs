//! Seam extraction from owned shapes and source rail constraints.

use super::super::super::super::keys::surface_overlay_grid_collinearity_error_bound;
use super::super::super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint};
use super::super::super::backend::{RoadVec2, overlay_point_to_road};
use super::super::super::rails::{NodeRailConstraint, NodeRailConstraintKind};
use super::super::super::{NodeOverlayShape, RoadSurfaceSystem};
use super::super::topology_keys::{
    NodeOwnershipPointKey, ownership_key_from_overlay_point, ownership_key_from_road_point,
    road_point_from_key,
};
use super::ConstraintOverlapMode;
use super::predicates::{
    canonicalize_seam_constraints, constraint_applies_to_owner,
    constraint_constrains_shared_height, constraint_is_material_transition,
    edge_lies_on_constraint_polyline_or_path, seam_source_from_constraint,
};
use std::collections::HashMap;

const PREPARED_RAIL_CONSTRAINT_TILE_KEYS: i64 = 8_000_000;
const PREPARED_RAIL_CONSTRAINT_MAX_INDEX_TILES: i64 = 256;
const OVERLAY_GRID_AXIS_TOLERANCE_KEYS: i64 = 2;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PreparedRailConstraintTile {
    x: i64,
    z: i64,
}

impl PreparedRailConstraintTile {
    fn from_key(point: NodeOwnershipPointKey) -> Self {
        Self {
            x: point.0.div_euclid(PREPARED_RAIL_CONSTRAINT_TILE_KEYS),
            z: point.1.div_euclid(PREPARED_RAIL_CONSTRAINT_TILE_KEYS),
        }
    }
}

struct PreparedRailConstraint<'a> {
    source: &'a NodeRailConstraint,
    points: Vec<NodeOwnershipPointKey>,
    segments: Vec<PreparedRailConstraintSegment>,
    segment_indices_by_min_z: Vec<usize>,
    point_constraint_keys: Vec<NodeOwnershipPointKey>,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
    constrains_shared_height: bool,
    is_material_transition: bool,
    protects_numeric_boundary: bool,
}

#[derive(Clone, Copy)]
struct PreparedRailConstraintSegment {
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
    dx: i128,
    dz: i128,
    grid_collinearity_error_bound: i128,
}

#[derive(Clone, Copy)]
struct PreparedOwnedShapeEdge {
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    dx: i128,
    dz: i128,
    end_parameter: i128,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
    grid_collinearity_error_bound: i128,
}

impl PreparedOwnedShapeEdge {
    fn new(start: NodeOwnershipPointKey, end: NodeOwnershipPointKey) -> Self {
        let dx = i128::from(end.0 - start.0);
        let dz = i128::from(end.1 - start.1);
        Self {
            start,
            end,
            dx,
            dz,
            end_parameter: dx * dx + dz * dz,
            min_x: start.0.min(end.0),
            min_z: start.1.min(end.1),
            max_x: start.0.max(end.0),
            max_z: start.1.max(end.1),
            grid_collinearity_error_bound: surface_overlay_grid_collinearity_error_bound(dx, dz),
        }
    }

    fn parameter_for(self, point: NodeOwnershipPointKey) -> i128 {
        let px = i128::from(point.0 - self.start.0);
        let pz = i128::from(point.1 - self.start.1);
        px * self.dx + pz * self.dz
    }

    fn contains_grid_bounded_with_cross(self, point: NodeOwnershipPointKey, cross: i128) -> bool {
        if point == self.start || point == self.end {
            return true;
        }
        if (self.dx != 0 && (point.0 < self.min_x || self.max_x < point.0))
            || (self.dz != 0 && (point.1 < self.min_z || self.max_z < point.1))
        {
            return false;
        }
        cross.abs() <= self.grid_collinearity_error_bound
    }
}

impl PreparedRailConstraintSegment {
    fn new(start: NodeOwnershipPointKey, end: NodeOwnershipPointKey) -> Self {
        let dx = i128::from(end.0 - start.0);
        let dz = i128::from(end.1 - start.1);
        Self {
            start,
            end,
            min_x: start.0.min(end.0),
            min_z: start.1.min(end.1),
            max_x: start.0.max(end.0),
            max_z: start.1.max(end.1),
            dx,
            dz,
            grid_collinearity_error_bound: surface_overlay_grid_collinearity_error_bound(dx, dz),
        }
    }

    fn bounds_contain_point(&self, point: NodeOwnershipPointKey) -> bool {
        (self.start.0 == self.end.0 || (self.min_x <= point.0 && point.0 <= self.max_x))
            && (self.start.1 == self.end.1 || (self.min_z <= point.1 && point.1 <= self.max_z))
    }

    fn contains_grid_bounded(&self, point: NodeOwnershipPointKey) -> bool {
        let px = i128::from(point.0 - self.start.0);
        let pz = i128::from(point.1 - self.start.1);
        self.contains_grid_bounded_with_cross(point, self.dx * pz - self.dz * px)
    }

    fn contains_grid_bounded_with_cross(&self, point: NodeOwnershipPointKey, cross: i128) -> bool {
        if point == self.start || point == self.end {
            return true;
        }
        if !self.bounds_contain_point(point) {
            return false;
        }
        cross.abs() <= self.grid_collinearity_error_bound
    }
}

pub(in crate::simulation::network::surface::node::ownership) struct PreparedRailConstraints<'a> {
    constraints: Vec<PreparedRailConstraint<'a>>,
    constraint_indices_by_tile: HashMap<PreparedRailConstraintTile, Vec<usize>>,
    global_constraint_indices: Vec<usize>,
}

/// Owner-specific constraint indices and their dense membership mask.
pub(in crate::simulation::network::surface::node::ownership) struct PreparedRailConstraintApplicability
{
    owner: NodeBandOwner,
    indices: Vec<usize>,
    bits: Vec<u64>,
}

impl PreparedRailConstraintApplicability {
    pub(in crate::simulation::network::surface::node::ownership) fn indices(&self) -> &[usize] {
        &self.indices
    }
}

/// Reusable buffers for spatial rail-constraint queries against owned shapes.
#[derive(Default)]
pub(in crate::simulation::network::surface::node::ownership) struct PreparedRailConstraintQueryScratch
{
    intervals: Vec<(i128, i128)>,
    overlaps: Vec<(NodeOwnershipPointKey, NodeOwnershipPointKey)>,
    candidate_indices: Vec<usize>,
    candidate_constraint_bits: Vec<u64>,
    cached_single_tile: Option<(NodeBandOwner, PreparedRailConstraintTile)>,
    cached_single_tile_indices: Vec<usize>,
}

impl<'a> PreparedRailConstraints<'a> {
    pub(in crate::simulation::network::surface::node::ownership) fn new(
        constraints: &'a [NodeRailConstraint],
    ) -> Self {
        let constraints = constraints
            .iter()
            .map(|constraint| {
                let points = constraint
                    .points_xz
                    .iter()
                    .copied()
                    .map(ownership_key_from_road_point)
                    .collect::<Vec<_>>();
                let mut segments = Vec::with_capacity(points.len().saturating_sub(1));
                let mut point_constraint_keys = Vec::new();
                for segment in points.windows(2) {
                    if segment[0] == segment[1] {
                        point_constraint_keys.push(segment[0]);
                    } else {
                        segments.push(PreparedRailConstraintSegment::new(segment[0], segment[1]));
                    }
                }
                segments.sort_unstable_by_key(|segment| {
                    (
                        segment.min_x,
                        segment.min_z,
                        segment.max_x,
                        segment.max_z,
                        segment.start,
                        segment.end,
                    )
                });
                let mut segment_indices_by_min_z = (0..segments.len()).collect::<Vec<_>>();
                segment_indices_by_min_z.sort_unstable_by_key(|&index| {
                    let segment = &segments[index];
                    (
                        segment.min_z,
                        segment.min_x,
                        segment.max_z,
                        segment.max_x,
                        segment.start,
                        segment.end,
                    )
                });
                point_constraint_keys.sort_unstable();
                point_constraint_keys.dedup();
                let (min_x, min_z, max_x, max_z) = constraint_bounds(&points);
                PreparedRailConstraint {
                    source: constraint,
                    points,
                    segments,
                    segment_indices_by_min_z,
                    point_constraint_keys,
                    // Keep broad-phase bounds conservative for the axis-aligned
                    // two-key tolerance used by the exact contact predicate.
                    min_x: min_x.saturating_sub(OVERLAY_GRID_AXIS_TOLERANCE_KEYS),
                    min_z: min_z.saturating_sub(OVERLAY_GRID_AXIS_TOLERANCE_KEYS),
                    max_x: max_x.saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS),
                    max_z: max_z.saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS),
                    constrains_shared_height: constraint_constrains_shared_height(constraint),
                    is_material_transition: constraint_is_material_transition(constraint),
                    protects_numeric_boundary: matches!(
                        constraint.kind,
                        NodeRailConstraintKind::SpanHandoff { .. }
                            | NodeRailConstraintKind::FootprintSeam { .. }
                            | NodeRailConstraintKind::AsphaltBoundary { .. }
                            | NodeRailConstraintKind::RaisedStepContact
                            | NodeRailConstraintKind::BandBoundary { .. }
                    ),
                }
            })
            .collect::<Vec<_>>();
        let mut constraint_indices_by_tile = HashMap::new();
        let mut global_constraint_indices = Vec::new();
        for (constraint_index, constraint) in constraints.iter().enumerate() {
            for segment in &constraint.segments {
                // Axis-aligned tolerant membership admits two key units of
                // perpendicular projection drift.
                let min_x = segment
                    .min_x
                    .saturating_sub(i64::from(segment.dx == 0) * OVERLAY_GRID_AXIS_TOLERANCE_KEYS);
                let max_x = segment
                    .max_x
                    .saturating_add(i64::from(segment.dx == 0) * OVERLAY_GRID_AXIS_TOLERANCE_KEYS);
                let min_z = segment
                    .min_z
                    .saturating_sub(i64::from(segment.dz == 0) * OVERLAY_GRID_AXIS_TOLERANCE_KEYS);
                let max_z = segment
                    .max_z
                    .saturating_add(i64::from(segment.dz == 0) * OVERLAY_GRID_AXIS_TOLERANCE_KEYS);
                index_prepared_constraint_bounds(
                    &mut constraint_indices_by_tile,
                    &mut global_constraint_indices,
                    constraint_index,
                    min_x,
                    min_z,
                    max_x,
                    max_z,
                );
            }
            for &point in &constraint.point_constraint_keys {
                index_prepared_constraint_bounds(
                    &mut constraint_indices_by_tile,
                    &mut global_constraint_indices,
                    constraint_index,
                    point.0,
                    point.1,
                    point.0,
                    point.1,
                );
            }
        }
        for indices in constraint_indices_by_tile.values_mut() {
            indices.sort_unstable();
            indices.dedup();
        }
        global_constraint_indices.sort_unstable();
        global_constraint_indices.dedup();
        Self {
            constraints,
            constraint_indices_by_tile,
            global_constraint_indices,
        }
    }

    pub(in crate::simulation::network::surface::node::ownership) fn applicable_constraints(
        &self,
        owner: NodeBandOwner,
    ) -> PreparedRailConstraintApplicability {
        let indices = self
            .constraints
            .iter()
            .enumerate()
            .filter_map(|(index, constraint)| {
                constraint_applies_to_owner(constraint.source, owner).then_some(index)
            })
            .collect::<Vec<_>>();
        let mut bits = vec![0; self.constraints.len().div_ceil(u64::BITS as usize)];
        for &index in &indices {
            bits[index / u64::BITS as usize] |= 1_u64 << (index % u64::BITS as usize);
        }
        PreparedRailConstraintApplicability {
            owner,
            indices,
            bits,
        }
    }

    pub(in crate::simulation::network::surface::node::ownership) fn shape_is_discardable_numeric_dust(
        &self,
        shape: &PreparedOwnedShape<'_>,
        area_m2: f32,
        applicable: &PreparedRailConstraintApplicability,
        scratch: &mut PreparedRailConstraintQueryScratch,
    ) -> bool {
        if area_m2 > RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(shape.source) {
            return false;
        }
        scratch
            .candidate_constraint_bits
            .resize(applicable.bits.len(), 0);
        !self.shape_touches_protected_boundary_constraint(shape, applicable, scratch)
    }

    pub(in crate::simulation::network::surface::node::ownership) fn seam_constraints_for_shape(
        &self,
        shape: &PreparedOwnedShape<'_>,
        owner: NodeBandOwner,
        applicable: &PreparedRailConstraintApplicability,
        overlap_mode: ConstraintOverlapMode,
        scratch: &mut PreparedRailConstraintQueryScratch,
    ) -> Vec<NodeRegionSeamConstraint> {
        if overlap_mode.allows_grid_bounded_constraint_overlap() {
            self.seam_constraints_for_shape_mode::<false>(shape, owner, applicable, scratch)
        } else {
            self.seam_constraints_for_shape_mode::<true>(shape, owner, applicable, scratch)
        }
    }

    fn seam_constraints_for_shape_mode<const EXACT_OVERLAP: bool>(
        &self,
        shape: &PreparedOwnedShape<'_>,
        owner: NodeBandOwner,
        applicable: &PreparedRailConstraintApplicability,
        scratch: &mut PreparedRailConstraintQueryScratch,
    ) -> Vec<NodeRegionSeamConstraint> {
        scratch
            .candidate_constraint_bits
            .resize(applicable.bits.len(), 0);
        let mut seams = Vec::with_capacity(shape.contour_keys.iter().map(Vec::len).sum());
        for (contour, contour_keys) in shape.source.iter().zip(&shape.contour_keys) {
            if contour.len() < 2 {
                continue;
            }
            for edge_index in 0..contour.len() {
                let start = contour[edge_index];
                let end = contour[(edge_index + 1) % contour.len()];
                let start_key = contour_keys[edge_index];
                let end_key = contour_keys[(edge_index + 1) % contour_keys.len()];
                if start_key == end_key {
                    continue;
                }
                let prepared_edge = PreparedOwnedShapeEdge::new(start_key, end_key);
                self.candidate_indices_for_bounds(
                    start_key.0.min(end_key.0),
                    start_key.1.min(end_key.1),
                    start_key.0.max(end_key.0),
                    start_key.1.max(end_key.1),
                    applicable,
                    scratch,
                );
                for &constraint_index in &scratch.candidate_indices {
                    let constraint = &self.constraints[constraint_index];
                    let (contact_summary, single_segment_overlap) =
                        if let [segment] = constraint.segments.as_slice() {
                            let contact = constraint_segment_contact::<EXACT_OVERLAP, false>(
                                prepared_edge,
                                segment,
                            );
                            (
                                ConstraintContactSummary::from_segment_contact(contact),
                                contact.overlap,
                            )
                        } else {
                            (
                                constraint_contacts_for_shape_edge::<EXACT_OVERLAP>(
                                    &mut scratch.overlaps,
                                    prepared_edge,
                                    constraint,
                                ),
                                None,
                            )
                        };
                    let carries_full_edge = contact_summary.carries_full_edge
                        || (constraint.source.kind != NodeRailConstraintKind::RaisedStepContact
                            && contact_summary.touches_edge_start
                            && contact_summary.touches_edge_end
                            && edge_lies_on_constraint_polyline_or_path(
                                start_key,
                                end_key,
                                constraint.source,
                                &constraint.points,
                                &mut scratch.intervals,
                            ));
                    if carries_full_edge {
                        push_region_seam_constraint(
                            &mut seams,
                            constraint,
                            owner,
                            start_key,
                            end_key,
                            overlay_point_to_road(start),
                            overlay_point_to_road(end),
                        );
                    }
                    if constraint.segments.len() == 1 {
                        if let Some((overlap_start, overlap_end)) = (!carries_full_edge)
                            .then_some(single_segment_overlap)
                            .flatten()
                        {
                            push_region_seam_constraint(
                                &mut seams,
                                constraint,
                                owner,
                                overlap_start,
                                overlap_end,
                                road_point_from_key(overlap_start),
                                road_point_from_key(overlap_end),
                            );
                        }
                    } else {
                        for &(overlap_start, overlap_end) in &scratch.overlaps {
                            push_region_seam_constraint(
                                &mut seams,
                                constraint,
                                owner,
                                overlap_start,
                                overlap_end,
                                road_point_from_key(overlap_start),
                                road_point_from_key(overlap_end),
                            );
                        }
                    }
                    if constraint.bounds_contains_point(start_key)
                        && (constraint
                            .point_constraint_keys
                            .binary_search(&start_key)
                            .is_ok()
                            || (constraint.is_material_transition
                                && contact_summary.touches_edge_start))
                    {
                        let point_xz = overlay_point_to_road(start);
                        push_region_seam_constraint(
                            &mut seams, constraint, owner, start_key, start_key, point_xz, point_xz,
                        );
                    }
                }
            }
        }
        canonicalize_seam_constraints(&mut seams);
        seams
    }

    fn shape_touches_protected_boundary_constraint(
        &self,
        shape: &PreparedOwnedShape<'_>,
        applicable: &PreparedRailConstraintApplicability,
        scratch: &mut PreparedRailConstraintQueryScratch,
    ) -> bool {
        for contour in &shape.contour_keys {
            for &point in contour {
                self.candidate_indices_for_bounds(
                    point.0, point.1, point.0, point.1, applicable, scratch,
                );
                if scratch.candidate_indices.iter().any(|&constraint_index| {
                    let constraint = &self.constraints[constraint_index];
                    constraint.protects_numeric_boundary
                        && constraint.source_segments_contain_point(point)
                }) {
                    return true;
                }
            }
            if contour.len() < 2 {
                continue;
            }
            for edge_index in 0..contour.len() {
                let start = contour[edge_index];
                let end = contour[(edge_index + 1) % contour.len()];
                self.candidate_indices_for_bounds(
                    start.0.min(end.0),
                    start.1.min(end.1),
                    start.0.max(end.0),
                    start.1.max(end.1),
                    applicable,
                    scratch,
                );
                if scratch.candidate_indices.iter().any(|&constraint_index| {
                    let constraint = &self.constraints[constraint_index];
                    constraint.protects_numeric_boundary
                        && (constraint.source_segments_contain_edge(start, end)
                            || edge_lies_on_constraint_polyline_or_path(
                                start,
                                end,
                                constraint.source,
                                &constraint.points,
                                &mut scratch.intervals,
                            ))
                }) {
                    return true;
                }
            }
        }
        false
    }

    fn candidate_indices_for_bounds(
        &self,
        min_x: i64,
        min_z: i64,
        max_x: i64,
        max_z: i64,
        applicable: &PreparedRailConstraintApplicability,
        scratch: &mut PreparedRailConstraintQueryScratch,
    ) {
        scratch.candidate_indices.clear();
        let min_tile = PreparedRailConstraintTile::from_key((min_x, min_z));
        let max_tile = PreparedRailConstraintTile::from_key((max_x, max_z));
        if min_tile == max_tile && self.global_constraint_indices.is_empty() {
            let cache_key = (applicable.owner, min_tile);
            if scratch.cached_single_tile != Some(cache_key) {
                scratch.cached_single_tile_indices.clear();
                if let Some(indices) = self.constraint_indices_by_tile.get(&min_tile) {
                    scratch
                        .cached_single_tile_indices
                        .extend(indices.iter().copied().filter(|&index| {
                            let mask = 1_u64 << (index % u64::BITS as usize);
                            applicable.bits[index / u64::BITS as usize] & mask != 0
                        }));
                }
                scratch.cached_single_tile = Some(cache_key);
            }
            for &index in &scratch.cached_single_tile_indices {
                let constraint = &self.constraints[index];
                if constraint.min_x <= max_x
                    && min_x <= constraint.max_x
                    && constraint.min_z <= max_z
                    && min_z <= constraint.max_z
                {
                    scratch.candidate_indices.push(index);
                }
            }
            return;
        }
        let tile_width = max_tile.x.saturating_sub(min_tile.x).saturating_add(1);
        let tile_height = max_tile.z.saturating_sub(min_tile.z).saturating_add(1);
        if tile_width.saturating_mul(tile_height) > PREPARED_RAIL_CONSTRAINT_MAX_INDEX_TILES {
            scratch
                .candidate_indices
                .extend(applicable.indices.iter().copied().filter(|&index| {
                    let constraint = &self.constraints[index];
                    constraint.min_x <= max_x
                        && min_x <= constraint.max_x
                        && constraint.min_z <= max_z
                        && min_z <= constraint.max_z
                }));
            return;
        }
        scratch.candidate_constraint_bits.fill(0);
        self.mark_candidate_indices_for_bounds(
            &self.global_constraint_indices,
            min_x,
            min_z,
            max_x,
            max_z,
            &applicable.bits,
            &mut scratch.candidate_constraint_bits,
        );
        for x in min_tile.x..=max_tile.x {
            for z in min_tile.z..=max_tile.z {
                if let Some(indices) = self
                    .constraint_indices_by_tile
                    .get(&PreparedRailConstraintTile { x, z })
                {
                    self.mark_candidate_indices_for_bounds(
                        indices,
                        min_x,
                        min_z,
                        max_x,
                        max_z,
                        &applicable.bits,
                        &mut scratch.candidate_constraint_bits,
                    );
                }
            }
        }
        for (word_index, &word) in scratch.candidate_constraint_bits.iter().enumerate() {
            let mut remaining = word;
            while remaining != 0 {
                let bit = remaining.trailing_zeros() as usize;
                scratch
                    .candidate_indices
                    .push(word_index * u64::BITS as usize + bit);
                remaining &= remaining - 1;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mark_candidate_indices_for_bounds(
        &self,
        indices: &[usize],
        min_x: i64,
        min_z: i64,
        max_x: i64,
        max_z: i64,
        applicable_constraint_bits: &[u64],
        candidate_constraint_bits: &mut [u64],
    ) {
        for &index in indices {
            let word = index / u64::BITS as usize;
            let bit = index % u64::BITS as usize;
            let mask = 1_u64 << bit;
            if applicable_constraint_bits.get(word).copied().unwrap_or(0) & mask == 0 {
                continue;
            }
            let constraint = &self.constraints[index];
            if constraint.min_x <= max_x
                && min_x <= constraint.max_x
                && constraint.min_z <= max_z
                && min_z <= constraint.max_z
            {
                candidate_constraint_bits[word] |= mask;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn index_prepared_constraint_bounds(
    constraint_indices_by_tile: &mut HashMap<PreparedRailConstraintTile, Vec<usize>>,
    global_constraint_indices: &mut Vec<usize>,
    constraint_index: usize,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
) {
    let min_tile = PreparedRailConstraintTile::from_key((min_x, min_z));
    let max_tile = PreparedRailConstraintTile::from_key((max_x, max_z));
    let tile_width = max_tile.x.saturating_sub(min_tile.x).saturating_add(1);
    let tile_height = max_tile.z.saturating_sub(min_tile.z).saturating_add(1);
    if tile_width.saturating_mul(tile_height) > PREPARED_RAIL_CONSTRAINT_MAX_INDEX_TILES {
        global_constraint_indices.push(constraint_index);
        return;
    }
    for x in min_tile.x..=max_tile.x {
        for z in min_tile.z..=max_tile.z {
            constraint_indices_by_tile
                .entry(PreparedRailConstraintTile { x, z })
                .or_default()
                .push(constraint_index);
        }
    }
}

impl PreparedRailConstraint<'_> {
    fn bounds_contains_point(&self, point: NodeOwnershipPointKey) -> bool {
        self.min_x <= point.0
            && point.0 <= self.max_x
            && self.min_z <= point.1
            && point.1 <= self.max_z
    }

    fn source_segments_contain_point(&self, point: NodeOwnershipPointKey) -> bool {
        let x_last = self.segments.partition_point(|segment| {
            segment.min_x <= point.0.saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS)
        });
        let z_last = self.segment_indices_by_min_z.partition_point(|&index| {
            self.segments[index].min_z <= point.1.saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS)
        });
        if x_last <= z_last {
            self.segments[..x_last]
                .iter()
                .any(|segment| segment.contains_grid_bounded(point))
        } else {
            self.segment_indices_by_min_z[..z_last]
                .iter()
                .any(|&index| self.segments[index].contains_grid_bounded(point))
        }
    }

    fn source_segments_contain_edge(
        &self,
        start: NodeOwnershipPointKey,
        end: NodeOwnershipPointKey,
    ) -> bool {
        let max_x = start
            .0
            .max(end.0)
            .saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS);
        let max_z = start
            .1
            .max(end.1)
            .saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS);
        let x_last = self
            .segments
            .partition_point(|segment| segment.min_x <= max_x);
        let z_last = self
            .segment_indices_by_min_z
            .partition_point(|&index| self.segments[index].min_z <= max_z);
        let contains_edge = |segment: &PreparedRailConstraintSegment| {
            segment.contains_grid_bounded(start) && segment.contains_grid_bounded(end)
        };
        if x_last <= z_last {
            self.segments[..x_last].iter().any(contains_edge)
        } else {
            self.segment_indices_by_min_z[..z_last]
                .iter()
                .any(|&index| contains_edge(&self.segments[index]))
        }
    }
}

fn constraint_bounds(points: &[NodeOwnershipPointKey]) -> (i64, i64, i64, i64) {
    let mut min_x = i64::MAX;
    let mut min_z = i64::MAX;
    let mut max_x = i64::MIN;
    let mut max_z = i64::MIN;
    for &(x, z) in points {
        min_x = min_x.min(x);
        min_z = min_z.min(z);
        max_x = max_x.max(x);
        max_z = max_z.max(z);
    }
    (min_x, min_z, max_x, max_z)
}

pub(in crate::simulation::network::surface::node::ownership) struct PreparedOwnedShape<'a> {
    source: &'a NodeOverlayShape,
    contour_keys: Vec<Vec<NodeOwnershipPointKey>>,
}

impl<'a> PreparedOwnedShape<'a> {
    pub(in crate::simulation::network::surface::node::ownership) fn new(
        shape: &'a NodeOverlayShape,
    ) -> Self {
        Self {
            source: shape,
            contour_keys: shape
                .iter()
                .map(|contour| {
                    contour
                        .iter()
                        .copied()
                        .map(ownership_key_from_overlay_point)
                        .collect()
                })
                .collect(),
        }
    }
}

pub(in crate::simulation::network::surface::node::ownership) fn owned_shape_is_discardable_numeric_dust(
    shape: &NodeOverlayShape,
    area_m2: f32,
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
) -> bool {
    if area_m2 > RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(shape) {
        return false;
    }
    let prepared_constraints = PreparedRailConstraints::new(rail_constraints);
    let applicable = prepared_constraints.applicable_constraints(owner);
    let prepared_shape = PreparedOwnedShape::new(shape);
    let mut scratch = PreparedRailConstraintQueryScratch::default();
    prepared_constraints.shape_is_discardable_numeric_dust(
        &prepared_shape,
        area_m2,
        &applicable,
        &mut scratch,
    )
}

#[cfg(test)]
pub(in crate::simulation::network::surface::node::ownership) fn seam_constraints_for_shape(
    shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
    overlap_mode: ConstraintOverlapMode,
) -> Vec<NodeRegionSeamConstraint> {
    let prepared_constraints = PreparedRailConstraints::new(rail_constraints);
    let applicable = prepared_constraints.applicable_constraints(owner);
    let prepared_shape = PreparedOwnedShape::new(shape);
    let mut scratch = PreparedRailConstraintQueryScratch::default();
    prepared_constraints.seam_constraints_for_shape(
        &prepared_shape,
        owner,
        &applicable,
        overlap_mode,
        &mut scratch,
    )
}

fn push_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint: &PreparedRailConstraint<'_>,
    owner: NodeBandOwner,
    start_key: NodeOwnershipPointKey,
    end_key: NodeOwnershipPointKey,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    seams.push(NodeRegionSeamConstraint {
        constraint_index: constraint.source.constraint_index,
        seam_source: seam_source_from_constraint(constraint.source, owner),
        owner: constraint.source.owner,
        opposite_owner: constraint.source.opposite_owner,
        constrains_shared_height: constraint.constrains_shared_height && start_key != end_key,
        is_material_transition: constraint.is_material_transition,
        start_xz,
        end_xz,
    });
}

#[derive(Clone, Copy)]
struct ConstraintSegmentContact {
    carries_full_edge: bool,
    contains_edge_start: bool,
    contains_edge_end: bool,
    overlap: Option<(NodeOwnershipPointKey, NodeOwnershipPointKey)>,
}

#[derive(Clone, Copy, Default)]
struct ConstraintContactSummary {
    carries_full_edge: bool,
    touches_edge_start: bool,
    touches_edge_end: bool,
}

impl ConstraintContactSummary {
    fn from_segment_contact(contact: ConstraintSegmentContact) -> Self {
        Self {
            carries_full_edge: contact.carries_full_edge,
            touches_edge_start: contact.contains_edge_start,
            touches_edge_end: contact.contains_edge_end,
        }
    }
}

fn constraint_contacts_for_shape_edge<const EXACT_OVERLAP: bool>(
    overlaps: &mut Vec<(NodeOwnershipPointKey, NodeOwnershipPointKey)>,
    edge: PreparedOwnedShapeEdge,
    constraint: &PreparedRailConstraint<'_>,
) -> ConstraintContactSummary {
    overlaps.clear();
    if constraint.segments.is_empty() {
        return ConstraintContactSummary::default();
    }
    let mut summary = ConstraintContactSummary::default();
    let mut merge_segment_contact = |segment: &PreparedRailConstraintSegment| {
        let contact = constraint_segment_contact::<EXACT_OVERLAP, true>(edge, segment);
        summary.carries_full_edge |= contact.carries_full_edge;
        summary.touches_edge_start |= contact.contains_edge_start;
        summary.touches_edge_end |= contact.contains_edge_end;
        if let Some(overlap) = contact.overlap {
            overlaps.push(overlap);
        }
    };
    let max_x = edge.max_x.saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS);
    let max_z = edge.max_z.saturating_add(OVERLAY_GRID_AXIS_TOLERANCE_KEYS);
    let x_last = constraint
        .segments
        .partition_point(|segment| segment.min_x <= max_x);
    let z_last = constraint
        .segment_indices_by_min_z
        .partition_point(|&index| constraint.segments[index].min_z <= max_z);
    if x_last <= z_last {
        for segment in &constraint.segments[..x_last] {
            merge_segment_contact(segment);
        }
    } else {
        for &index in &constraint.segment_indices_by_min_z[..z_last] {
            merge_segment_contact(&constraint.segments[index]);
        }
    }
    overlaps.sort_unstable();
    overlaps.dedup();
    summary
}

fn constraint_segment_contact<const EXACT_OVERLAP: bool, const RETAIN_OVERLAP_WHEN_FULL: bool>(
    edge: PreparedOwnedShapeEdge,
    segment: &PreparedRailConstraintSegment,
) -> ConstraintSegmentContact {
    let separated_x = segment.max_x < edge.min_x || edge.max_x < segment.min_x;
    let separated_z = segment.max_z < edge.min_z || edge.max_z < segment.min_z;
    if (separated_x && (EXACT_OVERLAP || (edge.dx != 0 && segment.dx != 0)))
        || (separated_z && (EXACT_OVERLAP || (edge.dz != 0 && segment.dz != 0)))
    {
        return ConstraintSegmentContact {
            carries_full_edge: false,
            contains_edge_start: false,
            contains_edge_end: false,
            overlap: None,
        };
    }
    let (start, end) = (segment.start, segment.end);
    let direction_cross = segment.dx * edge.dz - segment.dz * edge.dx;
    let segment_to_edge_start_x = i128::from(edge.start.0 - segment.start.0);
    let segment_to_edge_start_z = i128::from(edge.start.1 - segment.start.1);
    let edge_start_cross_on_segment =
        segment.dx * segment_to_edge_start_z - segment.dz * segment_to_edge_start_x;
    let edge_end_cross_on_segment = edge_start_cross_on_segment + direction_cross;
    let edge_start_on_segment =
        segment.contains_grid_bounded_with_cross(edge.start, edge_start_cross_on_segment);
    let edge_end_on_segment =
        segment.contains_grid_bounded_with_cross(edge.end, edge_end_cross_on_segment);
    let carries_full_edge = edge_start_on_segment && edge_end_on_segment;
    if carries_full_edge && !RETAIN_OVERLAP_WHEN_FULL {
        return ConstraintSegmentContact {
            carries_full_edge: true,
            contains_edge_start: true,
            contains_edge_end: true,
            overlap: None,
        };
    }
    let edge_to_segment_start_x = i128::from(start.0 - edge.start.0);
    let edge_to_segment_start_z = i128::from(start.1 - edge.start.1);
    let segment_start_cross_on_edge =
        edge.dx * edge_to_segment_start_z - edge.dz * edge_to_segment_start_x;
    if EXACT_OVERLAP {
        if direction_cross != 0 || segment_start_cross_on_edge != 0 {
            return ConstraintSegmentContact {
                carries_full_edge,
                contains_edge_start: edge_start_on_segment,
                contains_edge_end: edge_end_on_segment,
                overlap: None,
            };
        }
        let start_parameter = edge.parameter_for(start);
        let end_parameter = edge.parameter_for(end);
        let first_parameter = 0.max(start_parameter.min(end_parameter));
        let last_parameter = edge.end_parameter.min(start_parameter.max(end_parameter));
        if first_parameter >= last_parameter {
            return ConstraintSegmentContact {
                carries_full_edge,
                contains_edge_start: edge_start_on_segment,
                contains_edge_end: edge_end_on_segment,
                overlap: None,
            };
        }
        let point_at_parameter = |parameter| {
            if parameter == 0 {
                edge.start
            } else if parameter == edge.end_parameter {
                edge.end
            } else if parameter == start_parameter {
                start
            } else {
                end
            }
        };
        return ConstraintSegmentContact {
            carries_full_edge,
            contains_edge_start: edge_start_on_segment,
            contains_edge_end: edge_end_on_segment,
            overlap: Some((
                point_at_parameter(first_parameter),
                point_at_parameter(last_parameter),
            )),
        };
    }
    // Each candidate is already an endpoint of one segment, so only test
    // membership in the opposite segment.
    let start_parameter = edge.parameter_for(start);
    let end_parameter = edge.parameter_for(end);
    let segment_end_cross_on_edge = segment_start_cross_on_edge - direction_cross;
    let start_on_edge = edge.contains_grid_bounded_with_cross(start, segment_start_cross_on_edge);
    let end_on_edge = edge.contains_grid_bounded_with_cross(end, segment_end_cross_on_edge);
    let mut first = None;
    let mut consider_first = |point, parameter, member| {
        if member && first.is_none_or(|(_, first_parameter)| parameter < first_parameter) {
            first = Some((point, parameter));
        }
    };
    consider_first(edge.start, 0, edge_start_on_segment);
    consider_first(edge.end, edge.end_parameter, edge_end_on_segment);
    consider_first(start, start_parameter, start_on_edge);
    consider_first(end, end_parameter, end_on_edge);
    let Some((first, _)) = first else {
        return ConstraintSegmentContact {
            carries_full_edge,
            contains_edge_start: edge_start_on_segment,
            contains_edge_end: edge_end_on_segment,
            overlap: None,
        };
    };
    let mut last = None;
    let mut consider_last = |point, parameter, member| {
        if member
            && point != first
            && last.is_none_or(|(_, last_parameter)| parameter >= last_parameter)
        {
            last = Some((point, parameter));
        }
    };
    consider_last(edge.start, 0, edge_start_on_segment);
    consider_last(edge.end, edge.end_parameter, edge_end_on_segment);
    consider_last(start, start_parameter, start_on_edge);
    consider_last(end, end_parameter, end_on_edge);
    let overlap = last
        .map(|(last, _)| (first, last))
        .filter(|(first, last)| first != last);
    ConstraintSegmentContact {
        carries_full_edge,
        contains_edge_start: edge_start_on_segment,
        contains_edge_end: edge_end_on_segment,
        overlap,
    }
}
