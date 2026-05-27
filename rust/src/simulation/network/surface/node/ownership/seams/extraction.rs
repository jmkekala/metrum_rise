//! Seam extraction from owned shapes and source rail constraints.

use super::super::super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint};
use super::super::super::backend::{RoadVec2, overlay_point_to_road};
use super::super::super::rails::{NodeRailConstraint, NodeRailConstraintKind};
use super::super::super::{NodeOverlayPoint, NodeOverlayShape, RoadSurfaceSystem};
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
use std::collections::BTreeSet;

pub(in crate::simulation::network::surface::node::ownership) fn owned_shape_is_discardable_numeric_dust(
    shape: &NodeOverlayShape,
    area_m2: f32,
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
) -> bool {
    let protected_constraints = protected_constraints_for_owner(owner, rail_constraints);
    area_m2 <= RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(shape)
        && !shape_touches_protected_boundary_constraint(shape, &protected_constraints)
}

fn shape_touches_protected_boundary_constraint(
    shape: &NodeOverlayShape,
    protected_constraints: &[&NodeRailConstraint],
) -> bool {
    for contour in shape {
        for &point in contour {
            let point = ownership_key_from_overlay_point(point);
            if protected_constraints.iter().any(|constraint| {
                constraint.points_xz.windows(2).any(|segment| {
                    point_key_lies_on_segment(
                        point,
                        ownership_key_from_road_point(segment[0]),
                        ownership_key_from_road_point(segment[1]),
                    )
                })
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
            if protected_constraints
                .iter()
                .any(|constraint| edge_lies_on_constraint(start, end, constraint))
            {
                return true;
            }
        }
    }
    false
}

fn protected_constraints_for_owner(
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
) -> Vec<&NodeRailConstraint> {
    rail_constraints
        .iter()
        .filter(move |constraint| constraint_applies_to_owner(constraint, owner))
        .filter(|constraint| {
            matches!(
                constraint.kind,
                NodeRailConstraintKind::SpanHandoff { .. }
                    | NodeRailConstraintKind::FootprintSeam { .. }
                    | NodeRailConstraintKind::AsphaltBoundary { .. }
                    | NodeRailConstraintKind::RaisedStepContact
                    | NodeRailConstraintKind::BandBoundary { .. }
            )
        })
        .collect()
}

pub(in crate::simulation::network::surface::node::ownership) fn seam_constraints_for_shape(
    shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
    overlap_mode: ConstraintOverlapMode,
) -> Vec<NodeRegionSeamConstraint> {
    let mut seams = Vec::new();
    for contour in shape {
        if contour.len() < 2 {
            continue;
        }
        for edge_index in 0..contour.len() {
            let start = contour[edge_index];
            let end = contour[(edge_index + 1) % contour.len()];
            if ownership_key_from_overlay_point(start) == ownership_key_from_overlay_point(end) {
                continue;
            }
            for constraint in rail_constraints
                .iter()
                .filter(|constraint| constraint_applies_to_owner(constraint, owner))
            {
                if shape_edge_carries_full_seam_constraint(start, end, constraint) {
                    push_region_seam_constraint(
                        &mut seams,
                        constraint,
                        owner,
                        overlay_point_to_road(start),
                        overlay_point_to_road(end),
                    );
                }
                for (overlap_start, overlap_end) in
                    constraint_overlaps_shape_edge(start, end, constraint, overlap_mode)
                {
                    push_region_seam_constraint(
                        &mut seams,
                        constraint,
                        owner,
                        road_point_from_key(overlap_start),
                        road_point_from_key(overlap_end),
                    );
                }
            }
        }
        for point in contour.iter().copied() {
            for constraint in rail_constraints
                .iter()
                .filter(|constraint| constraint_applies_to_owner(constraint, owner))
                .filter(|constraint| {
                    point_lies_on_point_constraint(point, constraint)
                        || (constraint_is_material_transition(constraint)
                            && point_lies_on_source_segment(point, constraint))
                })
            {
                let point_xz = overlay_point_to_road(point);
                push_region_seam_constraint(&mut seams, constraint, owner, point_xz, point_xz);
            }
        }
    }
    canonicalize_seam_constraints(&mut seams);
    seams
}

fn push_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    seams.push(NodeRegionSeamConstraint {
        constraint_index: constraint.constraint_index,
        seam_source: seam_source_from_constraint(constraint, owner),
        owner: constraint.owner,
        opposite_owner: constraint.opposite_owner,
        constrains_shared_height: constraint_constrains_shared_height(constraint)
            && ownership_key_from_road_point(start_xz) != ownership_key_from_road_point(end_xz),
        is_material_transition: constraint_is_material_transition(constraint),
        start_xz,
        end_xz,
    });
}

fn constraint_overlaps_shape_edge(
    edge_start: NodeOverlayPoint,
    edge_end: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
    overlap_mode: ConstraintOverlapMode,
) -> Vec<(NodeOwnershipPointKey, NodeOwnershipPointKey)> {
    let edge_start = ownership_key_from_overlay_point(edge_start);
    let edge_end = ownership_key_from_overlay_point(edge_end);
    if edge_start == edge_end || constraint.points_xz.len() < 2 {
        return Vec::new();
    }
    let mut overlaps = BTreeSet::new();
    for segment in constraint.points_xz.windows(2) {
        let start = ownership_key_from_road_point(segment[0]);
        let end = ownership_key_from_road_point(segment[1]);
        if start == end {
            continue;
        }
        if !overlap_mode.allows_grid_bounded_constraint_overlap()
            && (!point_key_collinear_with_edge(start, edge_start, edge_end)
                || !point_key_collinear_with_edge(end, edge_start, edge_end))
        {
            continue;
        }
        let mut points = [edge_start, edge_end, start, end]
            .into_iter()
            .filter(|point| {
                point_key_lies_on_segment(*point, edge_start, edge_end)
                    && point_key_lies_on_segment(*point, start, end)
            })
            .collect::<Vec<_>>();
        points.sort_by_key(|point| segment_parameter_key(edge_start, edge_end, *point));
        points.dedup();
        let Some(first) = points.first().copied() else {
            continue;
        };
        let Some(last) = points.last().copied() else {
            continue;
        };
        let first = canonical_constraint_overlap_endpoint(first, start, end);
        let last = canonical_constraint_overlap_endpoint(last, start, end);
        if first != last {
            overlaps.insert((first, last));
        }
    }
    overlaps.into_iter().collect()
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
