//! Seam extraction from owned shapes and source rail constraints.

use super::super::super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint};
use super::super::super::backend::{RoadVec2, overlay_point_to_road};
use super::super::super::rails::{NodeRailConstraint, NodeRailConstraintKind};
use super::super::super::{NodeOverlayShape, RoadSurfaceSystem};
use super::super::topology_keys::{
    NodeOwnershipPointKey, ownership_key_from_overlay_point, ownership_key_from_road_point,
    point_key_collinear_with_edge, point_key_lies_on_segment, road_point_from_key,
    segment_parameter_key,
};
use super::ConstraintOverlapMode;
use super::predicates::{
    canonicalize_seam_constraints, constraint_applies_to_owner,
    constraint_constrains_shared_height, constraint_is_material_transition,
    edge_lies_on_constraint, point_lies_on_point_constraint, point_lies_on_source_segment,
    seam_source_from_constraint, shape_edge_carries_full_seam_constraint,
};

struct PreparedRailConstraint<'a> {
    source: &'a NodeRailConstraint,
    points: Vec<NodeOwnershipPointKey>,
    constrains_shared_height: bool,
    is_material_transition: bool,
    protects_numeric_boundary: bool,
}

pub(in crate::simulation::network::surface::node::ownership) struct PreparedRailConstraints<'a> {
    constraints: Vec<PreparedRailConstraint<'a>>,
}

impl<'a> PreparedRailConstraints<'a> {
    pub(in crate::simulation::network::surface::node::ownership) fn new(
        constraints: &'a [NodeRailConstraint],
    ) -> Self {
        Self {
            constraints: constraints
                .iter()
                .map(|constraint| PreparedRailConstraint {
                    source: constraint,
                    points: constraint
                        .points_xz
                        .iter()
                        .copied()
                        .map(ownership_key_from_road_point)
                        .collect(),
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
                })
                .collect(),
        }
    }

    pub(in crate::simulation::network::surface::node::ownership) fn applicable_indices(
        &self,
        owner: NodeBandOwner,
    ) -> Vec<usize> {
        self.constraints
            .iter()
            .enumerate()
            .filter_map(|(index, constraint)| {
                constraint_applies_to_owner(constraint.source, owner).then_some(index)
            })
            .collect()
    }

    pub(in crate::simulation::network::surface::node::ownership) fn shape_is_discardable_numeric_dust(
        &self,
        shape: &PreparedOwnedShape<'_>,
        area_m2: f32,
        applicable_indices: &[usize],
    ) -> bool {
        area_m2 <= RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(shape.source)
            && !self.shape_touches_protected_boundary_constraint(shape, applicable_indices)
    }

    pub(in crate::simulation::network::surface::node::ownership) fn seam_constraints_for_shape(
        &self,
        shape: &PreparedOwnedShape<'_>,
        owner: NodeBandOwner,
        applicable_indices: &[usize],
        overlap_mode: ConstraintOverlapMode,
    ) -> Vec<NodeRegionSeamConstraint> {
        let mut seams = Vec::new();
        let mut intervals = Vec::new();
        let mut overlaps = Vec::new();
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
                for &constraint_index in applicable_indices {
                    let constraint = &self.constraints[constraint_index];
                    if shape_edge_carries_full_seam_constraint(
                        start_key,
                        end_key,
                        constraint.source,
                        &constraint.points,
                        &mut intervals,
                    ) {
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
                    constraint_overlaps_shape_edge(
                        &mut overlaps,
                        start_key,
                        end_key,
                        constraint,
                        overlap_mode,
                    );
                    for &(overlap_start, overlap_end) in &overlaps {
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
            }
            for (&point, &point_key) in contour.iter().zip(contour_keys) {
                for &constraint_index in applicable_indices {
                    let constraint = &self.constraints[constraint_index];
                    if point_lies_on_point_constraint(point_key, &constraint.points)
                        || (constraint.is_material_transition
                            && point_lies_on_source_segment(point_key, &constraint.points))
                    {
                        let point_xz = overlay_point_to_road(point);
                        push_region_seam_constraint(
                            &mut seams, constraint, owner, point_key, point_key, point_xz, point_xz,
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
        applicable_indices: &[usize],
    ) -> bool {
        let mut intervals = Vec::new();
        for contour in &shape.contour_keys {
            for &point in contour {
                if applicable_indices.iter().any(|&constraint_index| {
                    let constraint = &self.constraints[constraint_index];
                    constraint.protects_numeric_boundary
                        && constraint
                            .points
                            .windows(2)
                            .any(|segment| point_key_lies_on_segment(point, segment[0], segment[1]))
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
                if applicable_indices.iter().any(|&constraint_index| {
                    let constraint = &self.constraints[constraint_index];
                    constraint.protects_numeric_boundary
                        && edge_lies_on_constraint(
                            start,
                            end,
                            constraint.source,
                            &constraint.points,
                            &mut intervals,
                        )
                }) {
                    return true;
                }
            }
        }
        false
    }
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
    let applicable_indices = prepared_constraints.applicable_indices(owner);
    let prepared_shape = PreparedOwnedShape::new(shape);
    prepared_constraints.shape_is_discardable_numeric_dust(
        &prepared_shape,
        area_m2,
        &applicable_indices,
    )
}

pub(in crate::simulation::network::surface::node::ownership) fn seam_constraints_for_shape(
    shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
    overlap_mode: ConstraintOverlapMode,
) -> Vec<NodeRegionSeamConstraint> {
    let prepared_constraints = PreparedRailConstraints::new(rail_constraints);
    let applicable_indices = prepared_constraints.applicable_indices(owner);
    let prepared_shape = PreparedOwnedShape::new(shape);
    prepared_constraints.seam_constraints_for_shape(
        &prepared_shape,
        owner,
        &applicable_indices,
        overlap_mode,
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

fn constraint_overlaps_shape_edge(
    overlaps: &mut Vec<(NodeOwnershipPointKey, NodeOwnershipPointKey)>,
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &PreparedRailConstraint<'_>,
    overlap_mode: ConstraintOverlapMode,
) {
    overlaps.clear();
    if edge_start == edge_end || constraint.points.len() < 2 {
        return;
    }
    for segment in constraint.points.windows(2) {
        let [start, end] = [segment[0], segment[1]];
        if start == end {
            continue;
        }
        if !overlap_mode.allows_grid_bounded_constraint_overlap()
            && (!point_key_collinear_with_edge(start, edge_start, edge_end)
                || !point_key_collinear_with_edge(end, edge_start, edge_end))
        {
            continue;
        }
        let mut points = [(0, 0); 4];
        let mut point_count = 0;
        for point in [edge_start, edge_end, start, end] {
            if point_key_lies_on_segment(point, edge_start, edge_end)
                && point_key_lies_on_segment(point, start, end)
            {
                points[point_count] = point;
                point_count += 1;
            }
        }
        let points = &mut points[..point_count];
        points.sort_by_key(|point| segment_parameter_key(edge_start, edge_end, *point));
        let Some(first) = points.first().copied() else {
            continue;
        };
        let Some(last) = points.iter().rev().copied().find(|point| *point != first) else {
            continue;
        };
        let first = canonical_constraint_overlap_endpoint(first, start, end);
        let last = canonical_constraint_overlap_endpoint(last, start, end);
        if first != last {
            overlaps.push((first, last));
        }
    }
    overlaps.sort_unstable();
    overlaps.dedup();
}

fn canonical_constraint_overlap_endpoint(
    point: NodeOwnershipPointKey,
    constraint_start: NodeOwnershipPointKey,
    constraint_end: NodeOwnershipPointKey,
) -> NodeOwnershipPointKey {
    if point == constraint_start {
        return constraint_start;
    }
    if point == constraint_end {
        return constraint_end;
    }
    point
}
