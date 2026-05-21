//! Footprint boundary loop tracing from exposed arrangement edges.

use super::super::{
    arrangement::{NodeArrangement, NodeArrangementKey, NodeArrangementVertex},
    arrangement_faces::arrangement_key_boundary_point,
    boundary::{
        ArrangementBoundaryPointKey, NodeBoundaryExportError, NodeFootprintBoundaryExportSources,
        NodeFootprintBoundaryPoint, boundary_points_numeric_area_budget_m2,
        remove_subbudget_unsupported_numeric_boundary_vertices,
        same_winding_boundary_point_loops_from_loop,
    },
};
use crate::simulation::network::surface::RoadSurfaceSystem;
use godot::prelude::Vector3;
use std::collections::{BTreeMap, BTreeSet};

impl RoadSurfaceSystem {
    pub(super) fn footprint_boundary_point_loops_from_arrangement_edges(
        arrangement: &NodeArrangement,
        boundary_export_sources: &mut NodeFootprintBoundaryExportSources,
    ) -> Result<Vec<Vec<NodeFootprintBoundaryPoint>>, NodeBoundaryExportError> {
        let mut boundary_edges = Vec::<FootprintBoundaryDirectedEdge>::new();
        for edge in arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
        {
            let Some(start_vertex) = arrangement.vertices().get(edge.start().index()) else {
                return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
            };
            let Some(end_vertex) = arrangement.vertices().get(edge.end().index()) else {
                return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
            };
            if start_vertex.key() == end_vertex.key() {
                continue;
            }
            let directed_edge = FootprintBoundaryDirectedEdge {
                start: footprint_boundary_point_from_arrangement_vertex(start_vertex),
                end: footprint_boundary_point_from_arrangement_vertex(end_vertex),
            };
            boundary_edges.push(directed_edge);
        }
        boundary_edges.sort_by(footprint_boundary_directed_edge_ordering);
        let mut adjacency = BTreeMap::<NodeArrangementKey, Vec<usize>>::new();
        for (edge_index, edge) in boundary_edges.iter().enumerate() {
            adjacency
                .entry(edge.start.xz_key())
                .or_default()
                .push(edge_index);
            adjacency
                .entry(edge.end.xz_key())
                .or_default()
                .push(edge_index);
        }
        for edges in adjacency.values_mut() {
            edges.sort_unstable();
            edges.dedup();
        }

        let mut loops = Vec::new();
        let mut emitted_loop_identities = BTreeSet::<Vec<ArrangementBoundaryPointKey>>::new();
        let mut visited_half_edges = BTreeSet::<(usize, bool)>::new();
        for edge_index in 0..boundary_edges.len() {
            for reversed in [false, true] {
                if visited_half_edges.contains(&(edge_index, reversed)) {
                    continue;
                }
                let Some(mut points) = trace_footprint_boundary_face(
                    &boundary_edges,
                    &adjacency,
                    &mut visited_half_edges,
                    edge_index,
                    reversed,
                )?
                else {
                    continue;
                };
                remove_subbudget_unsupported_numeric_boundary_vertices(
                    &mut points,
                    |current_point_key, local_points| {
                        boundary_export_sources
                            .has_exact_final_owned_footprint_boundary_support_at_point(
                                current_point_key,
                            )
                            || RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                                > boundary_points_numeric_area_budget_m2(&local_points)
                    },
                );
                let points = canonicalize_footprint_boundary_point_loop(points);
                if points.len() < 3 {
                    continue;
                }
                if signed_footprint_boundary_point_loop_area_xz(&points).abs()
                    <= footprint_boundary_point_loop_numeric_area_budget_m2(&points)
                {
                    continue;
                }
                for split_points in same_winding_boundary_point_loops_from_loop(&points) {
                    if signed_footprint_boundary_point_loop_area_xz(&split_points).abs()
                        <= footprint_boundary_point_loop_numeric_area_budget_m2(&split_points)
                    {
                        continue;
                    }
                    if !emitted_loop_identities
                        .insert(footprint_boundary_point_loop_identity(&split_points))
                    {
                        continue;
                    }
                    for point in &split_points {
                        boundary_export_sources
                            .reject_boundary_vertex_height_conflict(point.xz_key())?;
                        if !boundary_export_sources
                            .has_exact_final_owned_footprint_boundary_support_at_point(
                                point.point_key,
                            )
                        {
                            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight {
                                x_key: point.point_key.x_key,
                                z_key: point.point_key.z_key,
                            });
                        }
                    }
                    loops.push(split_points);
                }
            }
        }
        (!loops.is_empty())
            .then_some(loops)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }
}

fn canonicalize_footprint_boundary_point_loop(
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

fn footprint_boundary_point_loop_identity(
    points: &[NodeFootprintBoundaryPoint],
) -> Vec<ArrangementBoundaryPointKey> {
    let keys = points
        .iter()
        .map(|point| point.point_key)
        .collect::<Vec<_>>();
    let forward = canonical_footprint_boundary_loop_rotation(&keys);
    let mut reversed = keys;
    reversed.reverse();
    let reversed = canonical_footprint_boundary_loop_rotation(&reversed);
    forward.min(reversed)
}

fn canonical_footprint_boundary_loop_rotation(
    keys: &[ArrangementBoundaryPointKey],
) -> Vec<ArrangementBoundaryPointKey> {
    if keys.is_empty() {
        return Vec::new();
    }
    let start_index = keys
        .iter()
        .enumerate()
        .min_by_key(|(_, key)| **key)
        .map(|(index, _)| index)
        .unwrap_or(0);
    keys[start_index..]
        .iter()
        .chain(&keys[..start_index])
        .copied()
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct FootprintBoundaryDirectedEdge {
    start: NodeFootprintBoundaryPoint,
    end: NodeFootprintBoundaryPoint,
}

impl FootprintBoundaryDirectedEdge {
    fn reversed(self) -> Self {
        Self {
            start: self.end,
            end: self.start,
        }
    }
}

fn trace_footprint_boundary_face(
    edges: &[FootprintBoundaryDirectedEdge],
    adjacency: &BTreeMap<NodeArrangementKey, Vec<usize>>,
    visited_half_edges: &mut BTreeSet<(usize, bool)>,
    first_edge_index: usize,
    first_reversed: bool,
) -> Result<Option<Vec<NodeFootprintBoundaryPoint>>, NodeBoundaryExportError> {
    let first_edge = oriented_footprint_boundary_edge(edges[first_edge_index], first_reversed);
    let first_point_key = first_edge.start.point_key;
    let mut points = Vec::new();
    let mut local_visited_half_edges = BTreeSet::<(usize, bool)>::new();
    let mut current_edge_index = first_edge_index;
    let mut current_reversed = first_reversed;
    loop {
        if visited_half_edges.contains(&(current_edge_index, current_reversed)) {
            return Ok(None);
        }
        if !local_visited_half_edges.insert((current_edge_index, current_reversed)) {
            return Err(NodeBoundaryExportError::DegenerateOuterBoundaryLoop);
        }
        let current_edge =
            oriented_footprint_boundary_edge(edges[current_edge_index], current_reversed);
        points.push(current_edge.start);
        let Some((next_edge_index, next_reversed, next_edge)) = next_footprint_boundary_half_edge(
            edges,
            adjacency,
            visited_half_edges,
            &local_visited_half_edges,
            (first_edge_index, first_reversed),
            current_edge_index,
            current_edge,
        ) else {
            return Ok(None);
        };
        if (next_edge_index, next_reversed) == (first_edge_index, first_reversed) {
            if current_edge.end.point_key != first_point_key {
                points.push(current_edge.end);
            }
            visited_half_edges.extend(local_visited_half_edges);
            return Ok(Some(points));
        }
        if next_edge.start.point_key != current_edge.end.point_key {
            points.push(current_edge.end);
        }
        current_edge_index = next_edge_index;
        current_reversed = next_reversed;
    }
}

fn next_footprint_boundary_half_edge(
    edges: &[FootprintBoundaryDirectedEdge],
    adjacency: &BTreeMap<NodeArrangementKey, Vec<usize>>,
    visited_half_edges: &BTreeSet<(usize, bool)>,
    local_visited_half_edges: &BTreeSet<(usize, bool)>,
    first_half_edge: (usize, bool),
    current_edge_index: usize,
    current_edge: FootprintBoundaryDirectedEdge,
) -> Option<(usize, bool, FootprintBoundaryDirectedEdge)> {
    let current_xz = current_edge.end.xz_key();
    let incident_edges = adjacency.get(&current_xz)?;
    let mut candidates = incident_edges
        .iter()
        .copied()
        .filter(|edge_index| *edge_index != current_edge_index)
        .filter_map(|edge_index| {
            let (reversed, edge) =
                oriented_footprint_boundary_edge_from_xz(edges[edge_index], current_xz)?;
            let half_edge = (edge_index, reversed);
            (half_edge == first_half_edge
                || (!visited_half_edges.contains(&half_edge)
                    && !local_visited_half_edges.contains(&half_edge)))
            .then_some((edge_index, reversed, edge))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by(|a, b| {
        let a_exact = a.2.start.point_key == current_edge.end.point_key;
        let b_exact = b.2.start.point_key == current_edge.end.point_key;
        b_exact
            .cmp(&a_exact)
            .then(
                footprint_boundary_turn_ordering(current_edge, b.2)
                    .total_cmp(&footprint_boundary_turn_ordering(current_edge, a.2)),
            )
            .then(a.2.start.point_key.cmp(&b.2.start.point_key))
            .then(a.2.end.point_key.cmp(&b.2.end.point_key))
    });
    candidates.into_iter().next()
}

fn oriented_footprint_boundary_edge(
    edge: FootprintBoundaryDirectedEdge,
    reversed: bool,
) -> FootprintBoundaryDirectedEdge {
    if reversed { edge.reversed() } else { edge }
}

fn oriented_footprint_boundary_edge_from_xz(
    edge: FootprintBoundaryDirectedEdge,
    start_xz: NodeArrangementKey,
) -> Option<(bool, FootprintBoundaryDirectedEdge)> {
    if edge.start.xz_key() == start_xz {
        Some((false, edge))
    } else if edge.end.xz_key() == start_xz {
        Some((true, edge.reversed()))
    } else {
        None
    }
}

fn footprint_boundary_turn_ordering(
    current: FootprintBoundaryDirectedEdge,
    candidate: FootprintBoundaryDirectedEdge,
) -> f64 {
    let current_start = current.start.point_world();
    let current_end = current.end.point_world();
    let candidate_end = candidate.end.point_world();
    let back_x = f64::from(current_start.x - current_end.x);
    let back_z = f64::from(current_start.z - current_end.z);
    let out_x = f64::from(candidate_end.x - current_end.x);
    let out_z = f64::from(candidate_end.z - current_end.z);
    let back_angle = back_z.atan2(back_x);
    let out_angle = out_z.atan2(out_x);
    (back_angle - out_angle).rem_euclid(std::f64::consts::TAU)
}

fn footprint_boundary_directed_edge_ordering(
    a: &FootprintBoundaryDirectedEdge,
    b: &FootprintBoundaryDirectedEdge,
) -> std::cmp::Ordering {
    a.start
        .point_key
        .cmp(&b.start.point_key)
        .then(a.end.point_key.cmp(&b.end.point_key))
}

fn footprint_boundary_point_from_arrangement_vertex(
    vertex: &NodeArrangementVertex,
) -> NodeFootprintBoundaryPoint {
    NodeFootprintBoundaryPoint::new(arrangement_key_boundary_point(
        vertex.key(),
        vertex.height_mm(),
    ))
}

fn footprint_boundary_point_loop_world_points(
    points: &[NodeFootprintBoundaryPoint],
) -> Vec<Vector3> {
    points.iter().map(|point| point.point_world()).collect()
}

fn signed_footprint_boundary_point_loop_area_xz(points: &[NodeFootprintBoundaryPoint]) -> f32 {
    RoadSurfaceSystem::signed_polygon_area_xz(&footprint_boundary_point_loop_world_points(points))
}

fn footprint_boundary_point_loop_numeric_area_budget_m2(
    points: &[NodeFootprintBoundaryPoint],
) -> f32 {
    boundary_points_numeric_area_budget_m2(&footprint_boundary_point_loop_world_points(points))
}
