// SPDX-License-Identifier: GPL-2.0-only

//! Bounded road-side candidate generation and direct, non-cascading conflict ownership.

mod continuity;
mod curves;

use super::geometry::interiors_overlap;
use super::sources::{CellCurveSource, CurveSourceKey};
use super::{
    CELL_DEPTH, CellBounds, CellFrontage, CellKey, CellRoadFrontage, CellStore, GridFrame,
};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use glam::DVec2;
use godot::prelude::Vector3;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

// Four frontage cells per rigid curved group. Extra polyline height samples do not change
// its XZ stations. The last group uses the remaining horizontal arc length.
const GROUP_COLUMNS: f64 = super::CELL_GROUP_COLUMNS as f64;
// Existing parcel/road-query contract bounds the maximum corridor half-width by 128 m.
const ROAD_QUERY_PAD_M: f64 = 128.0;

/// Work counters for local generation acceptance; setup and publication remain distinguishable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellGeneration {
    /// Roads returned by the spatial query, independent of distant city population.
    pub(crate) roads: usize,
    /// Unique geometrically legal candidates before competing-frame suppression.
    pub(crate) candidates: usize,
    /// Exact cell/cell conflict comparisons, including direct suppressed-candidate dependencies.
    pub(crate) comparisons: usize,
    /// Accepted candidates in the requested publication area.
    pub(crate) accepted: usize,
}

#[derive(Clone)]
struct Candidate {
    pinned: bool,
    frame: GridFrame,
    x: i32,
    y: i32,
    row: u8,
    frontage: CellRoadFrontage,
    source: Option<Arc<CellCurveSource>>,
}

impl Candidate {
    fn priority(&self) -> (bool, u8, GridFrame, i32, i32) {
        (!self.pinned, self.row, self.frame, self.x, self.y)
    }

    fn corners(&self) -> [DVec2; 4] {
        self.frame.corners(self.x, self.y)
    }
}

impl CellStore {
    /// Recomputes cells intersecting a bounded edit/view envelope. The caller supplies exact
    /// compiled-road, manual-parcel, building-site and field exclusion without transferring
    /// ownership to this module. That predicate must be read-only and safe to query in parallel.
    pub(crate) fn generate_in_bounds(
        &mut self,
        graph: &RegionGraph,
        config: &WorldConfig,
        bounds: CellBounds,
        blocked: impl Fn(&[DVec2; 4]) -> bool + Sync,
    ) -> CellGeneration {
        if !bounds.is_valid() {
            return CellGeneration::default();
        }
        let cell_m = f64::from(config.zone_cell_m);
        // Any competing square intersects the requested square's envelope within one
        // diagonal. Direct candidate ownership never depends on its winner being accepted,
        // so there is exactly one conflict ring, not a city-wide greedy repacking chain.
        let candidates_bounds = bounds.expanded(2.0 * cell_m);
        let query = candidates_bounds
            .expanded((CELL_DEPTH as f64 + GROUP_COLUMNS + 1.0) * cell_m + ROAD_QUERY_PAD_M);
        let mut roads = graph.get_edges_near_aabb(
            Vector3::new(query.min.x as f32, 0.0, query.min.y as f32),
            Vector3::new(query.max.x as f32, 0.0, query.max.y as f32),
        );
        roads.sort_unstable();
        let pinned = continuity::pinned_candidates(self, graph, config, &roads, candidates_bounds);
        let curved = curves::pinned_candidates(self, graph, &roads, candidates_bounds);
        let candidates: Vec<_> = roads
            .par_iter()
            .flat_map_iter(|&edge_id| {
                edge_candidates(
                    graph,
                    edge_id,
                    cell_m,
                    candidates_bounds,
                    self.road_alignment(graph, edge_id),
                )
            })
            .chain(pinned.into_par_iter())
            .chain(curved.into_par_iter())
            .filter(|candidate| {
                let corners = candidate.corners();
                corners.iter().all(|point| {
                    point.x.abs() <= f64::from(config.width_m) * 0.5
                        && point.y.abs() <= f64::from(config.height_m) * 0.5
                }) && !overlaps_road_corridors(graph, &corners)
                    && !blocked(&corners)
            })
            .collect();

        // Reuse the cell block spatial structure as a temporary candidate index. It is local
        // to this operation and does not clone or scan the city's persistent frame registry.
        let mut raw = CellStore::default();
        let mut priorities = HashMap::with_capacity(candidates.len());
        let mut frontages: HashMap<CellKey, Vec<CellRoadFrontage>> = HashMap::new();
        let mut sources = HashMap::new();
        let mut candidates = candidates;
        candidates.sort_unstable_by_key(|candidate| candidate.priority());
        for candidate in candidates {
            let grid = raw.register_frame(candidate.frame);
            let key = CellKey {
                grid,
                x: candidate.x,
                y: candidate.y,
            };
            if raw.insert(key) {
                priorities.insert(key, candidate.priority());
            }
            if candidate.row == 0 {
                frontages.entry(key).or_default().push(candidate.frontage);
            }
            if let Some(source) = candidate.source {
                sources.entry(source.key).or_insert(source);
            }
        }
        let mut requested = Vec::new();
        raw.visit_in_bounds(bounds, |key| requested.push(key));
        requested.sort_unstable();
        // Distinct addresses on one canonical lattice cannot overlap. The temporary
        // registry contains only this request's candidates, not historical city frames.
        let competing_frames = raw.saved_frames().nth(1).is_some();
        let accepted: Vec<_> = requested
            .par_iter()
            .map(|&key| {
                let frame = raw.frame(key.grid).expect("indexed candidate frame");
                let corners = frame.corners(key.x, key.y);
                let candidate_bounds = CellBounds::from_points(corners);
                let mut valid = true;
                let mut comparisons = 0;
                // Painted land is pinned. Geometrically identical candidates retain paint; a new
                // incompatible frame may not displace even one previously painted square.
                let mut check_existing = |existing: CellKey| {
                    if valid
                        && let Some(existing_frame) = self.frame(existing.grid)
                        && (self.profile(existing).is_some_and(|profile| profile != 0)
                            || self.lot(existing).is_some_and(|lot| lot != 0)
                            || !CellBounds::from_points(
                                existing_frame.corners(existing.x, existing.y),
                            )
                            .intersects(bounds))
                        && (existing_frame != frame || existing.x != key.x || existing.y != key.y)
                    {
                        comparisons += 1;
                        valid = !interiors_overlap(
                            &corners,
                            &existing_frame.corners(existing.x, existing.y),
                        );
                    }
                };
                if candidate_bounds.min.cmpge(bounds.min).all()
                    && candidate_bounds.max.cmple(bounds.max).all()
                {
                    // An untouched empty cell lies outside publication bounds, so it cannot
                    // overlap a candidate wholly inside them. Reuse the reservation mask.
                    self.visit_reserved_in_bounds(candidate_bounds, &mut check_existing);
                } else {
                    self.visit_in_bounds(candidate_bounds, &mut check_existing);
                }
                if competing_frames {
                    raw.visit_in_bounds(candidate_bounds, |other| {
                        if valid
                            && other != key
                            && priorities[&other] < priorities[&key]
                            && let Some(other_frame) = raw.frame(other.grid)
                            && other_frame != frame
                        {
                            comparisons += 1;
                            valid = !interiors_overlap(
                                &corners,
                                &other_frame.corners(other.x, other.y),
                            );
                        }
                    });
                }
                (key, valid, comparisons)
            })
            .collect();

        let mut old = Vec::new();
        self.visit_in_bounds(bounds, |key| old.push(key));
        // Keep unchanged cells in place, avoiding revision churn and invalidating only actual
        // geometry differences. Conversion allocates ids for new frames in stable order.
        let mut output = Vec::with_capacity(accepted.len());
        let mut output_frontages = HashMap::new();
        let mut comparisons = 0;
        for (key, valid, count) in accepted {
            comparisons += count;
            if valid {
                let frame = raw.frame(key.grid).expect("indexed candidate frame");
                let published = CellKey {
                    grid: self.register_frame(frame),
                    x: key.x,
                    y: key.y,
                };
                if let Some(links) = frontages.remove(&key) {
                    output_frontages.insert(published, links);
                }
                output.push(published);
            }
        }
        output.sort_unstable();
        for key in old {
            if output.binary_search(&key).is_err() {
                // Pinned paint may survive without current road support. It cannot create
                // new lots using a deleted or geometrically incompatible former frontage.
                self.set_frontages(key, Vec::new());
                self.remove_unreserved(key);
            }
        }
        for &key in &output {
            self.insert(key);
            self.set_frontages(key, output_frontages.remove(&key).unwrap_or_default());
        }
        self.prune_curve_sources(bounds);
        let mut sources: Vec<_> = sources.into_values().collect();
        sources.sort_unstable_by_key(|source| source.key);
        for source in sources {
            self.publish_curve_source(source);
        }
        CellGeneration {
            roads: roads.len(),
            candidates: priorities.len(),
            comparisons,
            accepted: output.len(),
        }
    }
}

fn edge_candidates(
    graph: &RegionGraph,
    edge_id: usize,
    cell_m: f64,
    bounds: CellBounds,
    alignment: Option<[GridFrame; 2]>,
) -> Vec<Candidate> {
    let edge = graph.edge(edge_id);
    if edge.deleted
        || edge.no_building_spawn
        || edge.primary_type != TransitType::Road
        || edge.class != EdgeClass::Standard
        || edge.physical_geometry.len() < 2
    {
        return Vec::new();
    }
    let points: Vec<_> = edge
        .physical_geometry
        .iter()
        .map(|point| DVec2::new(f64::from(point.x), f64::from(point.z)))
        .collect();
    let mut stations = Vec::with_capacity(points.len());
    stations.push(0.0);
    for segment in points.windows(2) {
        stations.push(stations[stations.len() - 1] + segment[0].distance(segment[1]));
    }
    let length = stations[stations.len() - 1];
    let source_bounds = CellBounds::from_points(points.iter().copied());
    let station_precision = super::geometry::road_contact_precision(source_bounds);
    let straight_precision = super::geometry::road_source_precision(source_bounds).max(1e-4);
    let road_direction = (points[points.len() - 1] - points[0]).normalize_or_zero();
    let straight = road_direction.length_squared() > 0.0
        && points
            .iter()
            .all(|point| (point - points[0]).perp_dot(road_direction).abs() <= straight_precision);
    let half_width = f64::from(edge.width) * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH);
    let group_length = GROUP_COLUMNS * cell_m;
    let mut candidates = Vec::new();
    let mut station = 0.0;
    while station + cell_m <= length + station_precision || (straight && station < length) {
        let end_station = (station + group_length).min(length);
        let start = sample(&points, &stations, station);
        let end = sample(&points, &stations, end_station);
        let chord = end - start;
        if chord.length_squared() < 1e-12 {
            station = end_station;
            continue;
        }
        if !CellBounds::from_points([start, end])
            .expanded(CELL_DEPTH as f64 * cell_m + half_width + group_length)
            .intersects(bounds)
        {
            station = end_station;
            continue;
        }
        let direction = if straight {
            road_direction
        } else {
            chord.normalize()
        };
        let Some(basis) = alignment
            .filter(|_| straight)
            .map(|frames| frames[0])
            .or_else(|| GridFrame::new(DVec2::ZERO, direction, cell_m))
        else {
            break;
        };
        let (u, v) = basis.axes();
        let tangent = if direction.dot(u).abs() > direction.dot(v).abs() {
            u * direction.dot(u).signum()
        } else {
            v * direction.dot(v).signum()
        };
        let mut guide = vec![start];
        let first = stations.partition_point(|&s| s <= station);
        let last = stations.partition_point(|&s| s < end_station);
        guide.extend_from_slice(&points[first..last]);
        guide.push(end);
        guide.dedup();
        for side in [-1.0, 1.0] {
            let mut group = Vec::with_capacity(super::CELL_GROUP_COLUMNS * CELL_DEPTH);
            let normal = DVec2::new(tangent.y, -tangent.x) * side;
            let mut support = normal.dot(start).max(normal.dot(end)) + half_width;
            for &point in &guide {
                support = support.max(normal.dot(point) + half_width);
            }
            let mut origin = start + normal * (support - normal.dot(start));
            // A straight side inherits its endpoint junction's perpendicular curb phase.
            // The same corner construction on the neighboring road gives the same lattice.
            if straight {
                // All groups on a straight edge share the endpoint's exact frontage phase.
                // Recomputing it from each sampled chord accumulates independent rounding.
                origin = points[0] + normal * half_width;
                if alignment.is_none() {
                    if let Some(anchor) =
                        junction_anchor(graph, edge_id, tangent, normal, half_width)
                    {
                        origin += tangent * tangent.dot(anchor - origin);
                    } else {
                        origin += tangent * tangent.dot(points[0] - origin);
                    }
                }
            }
            let Some(frame) = alignment
                .filter(|_| straight)
                .map(|frames| frames[usize::from(side > 0.0)])
                .or_else(|| GridFrame::new(origin, tangent, cell_m))
            else {
                continue;
            };
            let local_start = frame.local(start);
            let along_axis = if tangent.dot(u).abs() > 0.5 { 0 } else { 1 };
            let along_sign = if tangent.dot(if along_axis == 0 { u } else { v }) > 0.0 {
                1
            } else {
                -1
            };
            let depth_axis = 1 - along_axis;
            let depth_sign = if normal.dot(if depth_axis == 0 { u } else { v }) > 0.0 {
                1
            } else {
                -1
            };
            let first_boundary = f64::from(along_sign) * local_start[along_axis];
            let mut boundary = if straight && station == 0.0 {
                (first_boundary + 1e-7).floor() as i32
            } else {
                (first_boundary - 1e-7).ceil() as i32
            };
            for _ in 0..=GROUP_COLUMNS as usize {
                let mut local = frame.local(origin).round();
                local[along_axis] = f64::from(along_sign * boundary);
                let front = frame.world(local.x, local.y);
                let along = tangent.dot(front - start);
                if along >= tangent.dot(end - start) - 1e-7 {
                    break;
                }
                if along < -1e-6 && !(straight && station == 0.0) {
                    boundary += 1;
                    continue;
                }
                let partial_start = station == 0.0 && along < -station_precision;
                let partial_end = end_station == length
                    && along + cell_m > tangent.dot(end - start) + station_precision;
                let mut coverage = [0.0, 1.0];
                if partial_start || partial_end {
                    let a = start + tangent * along;
                    if !straight
                        || !continuity::column_is_covered(graph, edge.width, a, tangent, cell_m)
                    {
                        boundary += 1;
                        continue;
                    }
                    coverage = [
                        (-along / cell_m).clamp(0.0, 1.0),
                        ((length - station - along) / cell_m).clamp(0.0, 1.0),
                    ];
                    if along_sign < 0 {
                        coverage = [1.0 - coverage[1], 1.0 - coverage[0]];
                    }
                }
                for row in 0..CELL_DEPTH {
                    let mut cell = local;
                    if along_sign < 0 {
                        cell[along_axis] -= 1.0;
                    }
                    cell[depth_axis] += f64::from(depth_sign) * row as f64;
                    if depth_sign < 0 {
                        cell[depth_axis] -= 1.0;
                    }
                    let candidate = Candidate {
                        source: None,
                        pinned: false,
                        frame,
                        x: cell.x as i32,
                        y: cell.y as i32,
                        row: row as u8,
                        frontage: CellRoadFrontage {
                            start: (coverage[0] * f64::from(super::CELL_FRONTAGE_UNITS)).round()
                                as u32,
                            end: (coverage[1] * f64::from(super::CELL_FRONTAGE_UNITS)).round()
                                as u32,
                            edge: edge_id,
                            side: side as i8,
                            boundary: match (depth_axis, depth_sign) {
                                (0, 1) => CellFrontage::MinX,
                                (0, _) => CellFrontage::MaxX,
                                (_, 1) => CellFrontage::MinY,
                                _ => CellFrontage::MaxY,
                            },
                        },
                    };
                    group.push(candidate);
                }
                boundary += 1;
            }
            if !straight && !group.is_empty() {
                let source = Arc::new(CellCurveSource {
                    key: CurveSourceKey {
                        frame,
                        min: [
                            group.iter().map(|c| c.x).min().unwrap(),
                            group.iter().map(|c| c.y).min().unwrap(),
                        ],
                        max: [
                            group.iter().map(|c| c.x + 1).max().unwrap(),
                            group.iter().map(|c| c.y + 1).max().unwrap(),
                        ],
                        frontage: group[0].frontage.boundary,
                    },
                    points: guide.iter().map(|p| p.to_array()).collect(),
                    half_width,
                    side: side as i8,
                    terminal: end_station == length,
                });
                debug_assert!(
                    source.is_valid(),
                    "invalid generated curve source: {source:?}"
                );
                for candidate in &mut group {
                    candidate.source = Some(Arc::clone(&source));
                }
            }
            candidates.extend(group.into_iter().filter(|candidate| {
                CellBounds::from_points(candidate.corners()).intersects(bounds)
            }));
        }
        station = end_station;
    }
    candidates
}

fn sample(points: &[DVec2], stations: &[f64], station: f64) -> DVec2 {
    let index = stations
        .partition_point(|&s| s <= station)
        .saturating_sub(1)
        .min(points.len() - 2);
    let length = stations[index + 1] - stations[index];
    if length <= 0.0 {
        return points[index];
    }
    points[index].lerp(
        points[index + 1],
        ((station - stations[index]) / length).clamp(0.0, 1.0),
    )
}

fn junction_anchor(
    graph: &RegionGraph,
    edge_id: usize,
    tangent: DVec2,
    normal: DVec2,
    half_width: f64,
) -> Option<DVec2> {
    let edge = graph.edge(edge_id);
    for (node_id, inward) in [(edge.start_node, tangent), (edge.end_node, -tangent)] {
        let mut neighbors: Vec<_> = graph
            .node_adjacency(graph.get_valid_node(node_id))
            .iter()
            .copied()
            .filter(|&id| id != edge_id)
            .collect();
        neighbors.sort_unstable();
        for id in neighbors {
            let neighbor = graph.edge(id);
            if neighbor.deleted
                || neighbor.physical_geometry.len() < 2
                || neighbor.primary_type != TransitType::Road
            {
                continue;
            }
            let other_node =
                if graph.get_valid_node(neighbor.start_node) == graph.get_valid_node(node_id) {
                    neighbor.end_node
                } else {
                    neighbor.start_node
                };
            let node = graph.node(graph.get_valid_node(node_id)).pos;
            let other = graph.node(graph.get_valid_node(other_node)).pos;
            let node = DVec2::new(f64::from(node.x), f64::from(node.z));
            let other = DVec2::new(f64::from(other.x), f64::from(other.z));
            let direction = other - node;
            let precision =
                super::geometry::road_source_precision(CellBounds::from_points([node, other]))
                    .max(1e-6 * direction.length());
            if direction.length_squared() == 0.0 || direction.dot(tangent).abs() > precision {
                continue;
            }
            let neighbor_width =
                f64::from(neighbor.width) * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH);
            return Some(node + normal * half_width + inward * neighbor_width);
        }
    }
    None
}

/// Computes a straight curb's junction phase in an already selected common basis.
pub(super) fn straight_frame(
    graph: &RegionGraph,
    edge_id: usize,
    basis: GridFrame,
    side: f64,
) -> GridFrame {
    let edge = graph.edge(edge_id);
    let a = edge.physical_geometry[0];
    let b = edge.physical_geometry[edge.physical_geometry.len() - 1];
    let start = DVec2::new(f64::from(a.x), f64::from(a.z));
    let direction = DVec2::new(f64::from(b.x) - start.x, f64::from(b.z) - start.y);
    let (u, v) = basis.axes();
    let tangent = if direction.dot(u).abs() > direction.dot(v).abs() {
        u * direction.dot(u).signum()
    } else {
        v * direction.dot(v).signum()
    };
    let normal = DVec2::new(tangent.y, -tangent.x) * side;
    let half_width = f64::from(edge.width) * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH);
    let mut origin = start + normal * half_width;
    if let Some(anchor) = junction_anchor(graph, edge_id, tangent, normal, half_width) {
        origin += tangent * tangent.dot(anchor - origin);
    }
    basis.at_origin(origin)
}

/// Tests canonical rectangles against every local standard road, including the supplying edge.
pub(crate) fn overlaps_road_corridors(graph: &RegionGraph, corners: &[DVec2; 4]) -> bool {
    let cell_bounds = CellBounds::from_points(*corners);
    let bounds = cell_bounds.expanded(ROAD_QUERY_PAD_M);
    let mut overlaps = false;
    graph.visit_edges_near_aabb(
        Vector3::new(bounds.min.x as f32, 0.0, bounds.min.y as f32),
        Vector3::new(bounds.max.x as f32, 0.0, bounds.max.y as f32),
        |id| {
            let edge: &Edge = graph.edge(id);
            if overlaps || edge.deleted || edge.class != EdgeClass::Standard {
                return;
            }
            let half_width = f64::from(edge.width) * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH);
            for segment in edge.physical_geometry.windows(2) {
                let a = DVec2::new(f64::from(segment[0].x), f64::from(segment[0].z));
                let b = DVec2::new(f64::from(segment[1].x), f64::from(segment[1].z));
                // The contracted corridor lies inside this full-width envelope. Rounding
                // disjoint coordinate intervals cannot create positive-area intersection.
                if !CellBounds::from_points([a, b])
                    .expanded(half_width)
                    .intersects(cell_bounds)
                {
                    continue;
                }
                let precision = super::geometry::road_source_precision(CellBounds::from_points(
                    corners.iter().copied().chain([a, b]),
                ));
                let normal = (b - a).normalize_or_zero().perp() * (half_width - precision).max(0.0);
                if normal.length_squared() > 0.0
                    && interiors_overlap(corners, &[a - normal, b - normal, b + normal, a + normal])
                {
                    overlaps = true;
                    break;
                }
            }
        },
    );
    overlaps
}
