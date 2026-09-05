// SPDX-License-Identifier: GPL-2.0-only

//! Shared seam predicates and sorting helpers.

use super::super::super::RoadSurfaceBandKind;
use super::super::super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource};
use super::super::super::rails::{NodeRailConstraint, NodeRailConstraintKind};
use super::super::contact_semantics::{
    band_boundary_constrains_shared_height, raised_step_contact_constrains_shared_height,
};
use super::super::topology_keys::{
    NodeOwnershipPointKey, ownership_key_from_road_point, point_key_collinear_with_edge,
    point_key_collinear_with_edge_on_overlay_grid, point_key_lies_on_segment,
    segment_parameter_key,
};

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_seam_constraints(
    seams: &mut Vec<NodeRegionSeamConstraint>,
) {
    seams.sort_by_cached_key(seam_constraint_sort_key);
    let mut previous_key = None;
    seams.retain(|constraint| {
        let key = seam_constraint_sort_key(constraint);
        if previous_key == Some(key) {
            return false;
        }
        previous_key = Some(key);
        true
    });
}

fn seam_constraint_sort_key(
    constraint: &NodeRegionSeamConstraint,
) -> (
    usize,
    NodeOwnershipPointKey,
    NodeOwnershipPointKey,
    Option<NodeBandOwner>,
    Option<NodeBandOwner>,
) {
    (
        constraint.constraint_index,
        ownership_key_from_road_point(constraint.start_xz),
        ownership_key_from_road_point(constraint.end_xz),
        constraint.owner,
        constraint.opposite_owner,
    )
}

pub(super) fn constraint_constrains_shared_height(constraint: &NodeRailConstraint) -> bool {
    if constraint_is_point_contact(constraint) {
        return false;
    }
    match constraint.kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::AsphaltBoundary { .. } => true,
        NodeRailConstraintKind::RaisedStepContact => {
            let Some((owner, opposite_owner)) = constraint.owner.zip(constraint.opposite_owner)
            else {
                return false;
            };
            raised_step_contact_constrains_shared_height(owner, opposite_owner)
        }
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => band_boundary_constrains_shared_height(left_kind, right_kind),
        _ => false,
    }
}

pub(super) fn constraint_is_point_contact(constraint: &NodeRailConstraint) -> bool {
    let Some(first) = constraint
        .points_xz
        .first()
        .copied()
        .map(ownership_key_from_road_point)
    else {
        return false;
    };
    constraint
        .points_xz
        .iter()
        .copied()
        .map(ownership_key_from_road_point)
        .all(|point| point == first)
}

pub(super) fn constraint_is_material_transition(constraint: &NodeRailConstraint) -> bool {
    match constraint.kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::RaisedStepContact
        | NodeRailConstraintKind::BandBoundary { .. } => true,
        NodeRailConstraintKind::AsphaltBoundary { adjacent_kind } => {
            adjacent_kind != RoadSurfaceBandKind::Carriageway
        }
        _ => false,
    }
}

pub(in crate::simulation::network::surface::node::ownership) fn constraint_applies_to_owner(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
) -> bool {
    if constraint.owner.is_some() || constraint.opposite_owner.is_some() {
        return constraint.owner == Some(owner) || constraint.opposite_owner == Some(owner);
    }
    match constraint.kind {
        NodeRailConstraintKind::FullRoadbedContour => true,
        NodeRailConstraintKind::BandContour { kind }
        | NodeRailConstraintKind::SpanHandoff { kind }
        | NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: kind,
        } => kind == owner.kind(),
        NodeRailConstraintKind::AsphaltBoundary { adjacent_kind } => {
            owner.kind() == RoadSurfaceBandKind::Carriageway || adjacent_kind == owner.kind()
        }
        NodeRailConstraintKind::RaisedStepContact => false,
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => left_kind == owner.kind() || right_kind == owner.kind(),
    }
}

pub(super) fn edge_lies_on_constraint_polyline_or_path(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
    constraint_points: &[NodeOwnershipPointKey],
    intervals: &mut Vec<(i128, i128)>,
) -> bool {
    edge_lies_on_constraint_polyline(edge_start, edge_end, constraint_points, intervals)
        || edge_endpoints_lie_on_constraint_path(
            edge_start,
            edge_end,
            constraint,
            constraint_points,
        )
}

pub(super) fn edge_lies_on_single_constraint_segment_with_points(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint_points: &[NodeOwnershipPointKey],
) -> bool {
    constraint_points.windows(2).any(|segment| {
        let [start, end] = [segment[0], segment[1]];
        point_key_lies_on_segment(edge_start, start, end)
            && point_key_lies_on_segment(edge_end, start, end)
    })
}

pub(super) fn edge_lies_on_constraint_polyline_on_overlay_grid(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint_points: &[NodeOwnershipPointKey],
    intervals: &mut Vec<(i128, i128)>,
) -> bool {
    if edge_start == edge_end || constraint_points.len() < 2 {
        return false;
    }
    let edge_end_parameter = segment_parameter_key(edge_start, edge_end, edge_end);
    if edge_end_parameter <= 0 {
        return false;
    }
    intervals.clear();
    for segment in constraint_points.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start == end
            || !point_key_collinear_with_edge_on_overlay_grid(start, edge_start, edge_end)
            || !point_key_collinear_with_edge_on_overlay_grid(end, edge_start, edge_end)
        {
            continue;
        }
        let start_parameter = segment_parameter_key(edge_start, edge_end, start);
        let end_parameter = segment_parameter_key(edge_start, edge_end, end);
        let overlap_start = start_parameter.min(end_parameter).max(0);
        let overlap_end = start_parameter.max(end_parameter).min(edge_end_parameter);
        if overlap_start < overlap_end {
            intervals.push((overlap_start, overlap_end));
        }
    }
    if intervals.is_empty() {
        return false;
    }
    intervals.sort_unstable();
    let mut covered_end = 0;
    for &(start, end) in intervals.iter() {
        if start > covered_end {
            return false;
        }
        covered_end = covered_end.max(end);
        if covered_end >= edge_end_parameter {
            return true;
        }
    }
    false
}

fn edge_lies_on_constraint_polyline(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint_points: &[NodeOwnershipPointKey],
    intervals: &mut Vec<(i128, i128)>,
) -> bool {
    if edge_start == edge_end || constraint_points.len() < 2 {
        return false;
    }
    let edge_end_parameter = segment_parameter_key(edge_start, edge_end, edge_end);
    if edge_end_parameter <= 0 {
        return false;
    }
    intervals.clear();
    for segment in constraint_points.windows(2) {
        let [start, end] = [segment[0], segment[1]];
        if start == end
            || !point_key_collinear_with_edge(start, edge_start, edge_end)
            || !point_key_collinear_with_edge(end, edge_start, edge_end)
        {
            continue;
        }
        let start_parameter = segment_parameter_key(edge_start, edge_end, start);
        let end_parameter = segment_parameter_key(edge_start, edge_end, end);
        let overlap_start = start_parameter.min(end_parameter).max(0);
        let overlap_end = start_parameter.max(end_parameter).min(edge_end_parameter);
        if overlap_start < overlap_end {
            intervals.push((overlap_start, overlap_end));
        }
    }
    if intervals.is_empty() {
        return false;
    }
    intervals.sort_unstable();
    let mut covered_end = 0;
    for &(start, end) in intervals.iter() {
        if start > covered_end {
            return false;
        }
        covered_end = covered_end.max(end);
        if covered_end >= edge_end_parameter {
            return true;
        }
    }
    false
}

fn edge_endpoints_lie_on_constraint_path(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
    constraint_points: &[NodeOwnershipPointKey],
) -> bool {
    if edge_start == edge_end
        || constraint_points.len() < 2
        || !constraint_allows_path_chord(constraint)
    {
        return false;
    }
    constraint_path_contains_ordered_endpoints(edge_start, edge_end, constraint_points)
        || constraint_path_contains_ordered_endpoints(edge_end, edge_start, constraint_points)
}

fn constraint_path_contains_ordered_endpoints(
    first: NodeOwnershipPointKey,
    second: NodeOwnershipPointKey,
    constraint_points: &[NodeOwnershipPointKey],
) -> bool {
    let mut first_seen = false;
    for segment in constraint_points.windows(2) {
        let [start, end] = [segment[0], segment[1]];
        if point_key_lies_on_segment(first, start, end) {
            first_seen = true;
        }
        if first_seen && point_key_lies_on_segment(second, start, end) {
            return true;
        }
    }
    false
}

fn constraint_allows_path_chord(constraint: &NodeRailConstraint) -> bool {
    matches!(
        constraint.kind,
        NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::RaisedStepContact
            | NodeRailConstraintKind::BandBoundary { .. }
    )
}

pub(super) fn seam_source_from_constraint(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
) -> NodeSeamSource {
    match constraint.kind {
        NodeRailConstraintKind::RaisedStepContact => NodeSeamSource::RaisedStepContact {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::AsphaltBoundary { .. } => NodeSeamSource::AsphaltBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::FootprintSeam { .. }
        | NodeRailConstraintKind::FullRoadbedContour => NodeSeamSource::FootprintBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::BandBoundary { .. } => NodeSeamSource::for_owner(owner),
    }
}
