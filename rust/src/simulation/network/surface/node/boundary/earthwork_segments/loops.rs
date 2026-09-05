// SPDX-License-Identifier: GPL-2.0-only

//! Loop-level earthwork boundary segment assembly.

use super::splits::push_sourced_node_earthwork_boundary_segments;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::simulation::network::surface) fn node_earthwork_boundary_segments_from_footprint_loops(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    footprint_loops: &[Vec<NodeFootprintBoundaryPoint>],
    sources: &NodeFootprintBoundaryExportSources,
) -> Result<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>, NodeBoundaryExportError> {
    if sources.source_edges.is_empty() {
        return Err(NodeBoundaryExportError::MissingEarthworkBoundarySource);
    }

    let mut loops = Vec::new();
    for footprint_loop in footprint_loops {
        for points in same_winding_boundary_point_loops_from_loop(footprint_loop) {
            let mut segments = Vec::new();
            for index in 0..points.len() {
                push_sourced_node_earthwork_boundary_segments(
                    node_id,
                    kind,
                    points[index],
                    points[(index + 1) % points.len()],
                    &sources.source_edges,
                    &sources.direct_vertex_sources,
                    &sources.direct_vertex_source_candidates,
                    &sources.direct_vertex_source_conflicts,
                    &sources.explicit_vertical_step_segments,
                    &mut segments,
                )?;
            }
            if segments.len() >= 3 {
                loops.push(segments);
            }
        }
    }

    (!loops.is_empty())
        .then_some(loops)
        .ok_or(NodeBoundaryExportError::MissingEarthworkBoundarySource)
}

pub(in crate::simulation::network::surface::node) fn same_winding_boundary_point_loops_from_loop(
    points: &[NodeFootprintBoundaryPoint],
) -> Vec<Vec<NodeFootprintBoundaryPoint>> {
    if !boundary_point_loop_has_repeated_xz(points) {
        return vec![points.to_vec()];
    }

    let source_area_m2 = signed_boundary_point_loop_area_xz(points);
    split_boundary_point_loop_at_repeated_xz(points.to_vec())
        .into_iter()
        .filter_map(|points| {
            let points = canonicalize_boundary_point_loop(points);
            if points.len() < 3 {
                return None;
            }
            let split_area_m2 = signed_boundary_point_loop_area_xz(&points);
            if split_area_m2.abs() <= boundary_point_loop_numeric_area_budget_m2(&points) {
                return None;
            }
            (source_area_m2.signum() == split_area_m2.signum()).then_some(points)
        })
        .collect()
}

fn split_boundary_point_loop_at_repeated_xz(
    points: Vec<NodeFootprintBoundaryPoint>,
) -> Vec<Vec<NodeFootprintBoundaryPoint>> {
    let points = canonicalize_boundary_point_loop(points);
    if points.len() < 3 {
        return Vec::new();
    }

    let mut loops = Vec::new();
    let mut stack = vec![points[0]];
    let mut seen = BTreeMap::<arrangement::NodeArrangementKey, usize>::new();
    seen.insert(points[0].xz_key(), 0);
    for index in 1..=points.len() {
        let current = points[index % points.len()];
        let current_key = current.xz_key();
        if let Some(start_index) = seen.get(&current_key).copied() {
            let mut cycle = stack[start_index..].to_vec();
            cycle.push(current);
            let cycle = canonicalize_boundary_point_loop(cycle);
            if cycle.len() >= 3 {
                loops.push(cycle);
            }
            stack.truncate(start_index + 1);
            if let Some(last) = stack.last_mut() {
                *last = current;
            }
            seen.clear();
            for (stack_index, point) in stack.iter().enumerate() {
                seen.insert(point.xz_key(), stack_index);
            }
        } else {
            stack.push(current);
            seen.insert(current_key, stack.len() - 1);
        }
    }

    if loops.is_empty() {
        vec![points]
    } else {
        loops
    }
}

fn boundary_point_loop_has_repeated_xz(points: &[NodeFootprintBoundaryPoint]) -> bool {
    let mut seen = BTreeSet::new();
    for point in canonicalize_boundary_point_loop(points.to_vec()) {
        if !seen.insert(point.xz_key()) {
            return true;
        }
    }
    false
}

fn canonicalize_boundary_point_loop(
    mut points: Vec<NodeFootprintBoundaryPoint>,
) -> Vec<NodeFootprintBoundaryPoint> {
    points.dedup_by(|a, b| a.point_key == b.point_key);
    if points.len() >= 2
        && points.first().map(|point| point.point_key) == points.last().map(|point| point.point_key)
    {
        points.pop();
    }
    points
}

fn boundary_point_loop_world_points(points: &[NodeFootprintBoundaryPoint]) -> Vec<RoadVec3> {
    points.iter().map(|point| point.point_world()).collect()
}

fn signed_boundary_point_loop_area_xz(points: &[NodeFootprintBoundaryPoint]) -> f32 {
    RoadSurfaceSystem::signed_polygon_area_xz(&boundary_point_loop_world_points(points))
}

fn boundary_point_loop_numeric_area_budget_m2(points: &[NodeFootprintBoundaryPoint]) -> f32 {
    boundary_points_numeric_area_budget_m2(&boundary_point_loop_world_points(points))
}
