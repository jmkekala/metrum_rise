// SPDX-License-Identifier: GPL-2.0-only

//! Retained lattice constraints along geometrically continuous straight road frontages.

use super::*;
use crate::simulation::zoning::cells::CELL_FRONTAGE_UNITS;
use crate::simulation::zoning::cells::geometry::road_contact_precision;
use std::collections::{BTreeMap, BTreeSet};

struct Interval {
    min: f64,
    max: f64,
    edge: usize,
    side: i8,
}

/// Proves that a cell crossing an edge endpoint has continuous, eligible straight frontage.
/// Endpoint columns query k intersecting roads and P points in O(log R + P + k log k).
pub(super) fn column_is_covered(
    graph: &RegionGraph,
    width: f32,
    start: DVec2,
    tangent: DVec2,
    length: f64,
) -> bool {
    let bounds = CellBounds::from_points([start, start + tangent * length]);
    // Current graph points are f32. This bound covers their collinear subdivision error;
    // final whole-cell road/cell predicates remain unchanged.
    let epsilon = super::super::geometry::road_source_precision(bounds);
    let query = bounds.expanded(epsilon);
    let mut intervals = Vec::new();
    graph.visit_edges_near_aabb(
        Vector3::new(query.min.x as f32, 0.0, query.min.y as f32),
        Vector3::new(query.max.x as f32, 0.0, query.max.y as f32),
        |id| {
            let edge = graph.edge(id);
            if edge.deleted
                || edge.no_building_spawn
                || edge.primary_type != TransitType::Road
                || edge.class != EdgeClass::Standard
                || edge.width != width
                || edge.physical_geometry.len() < 2
            {
                return;
            }
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for point in &edge.physical_geometry {
                let point = DVec2::new(f64::from(point.x), f64::from(point.z)) - start;
                if point.perp_dot(tangent).abs() > epsilon {
                    return;
                }
                min = min.min(point.dot(tangent));
                max = max.max(point.dot(tangent));
            }
            intervals.push((min, max));
        },
    );
    intervals.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut covered = 0.0;
    for (min, max) in intervals {
        if min > covered + epsilon {
            return false;
        }
        covered = covered.max(max);
        if covered >= length - epsilon {
            return true;
        }
    }
    false
}

/// Reuses nearby reserved frames when the current curb lies on their exact lattice boundary.
/// Full frontage coverage is the union of local collinear intervals, independent of edge splits.
pub(super) fn pinned_candidates(
    store: &CellStore,
    graph: &RegionGraph,
    config: &WorldConfig,
    roads: &[usize],
    bounds: CellBounds,
) -> Vec<Candidate> {
    if !store.has_reservations() {
        return Vec::new();
    }
    let mut frames = BTreeSet::new();
    // A retained four-column/six-row strip can be constrained by paint outside the requested
    // payload. Its influence is defined in lattice coordinates, never by camera/chunk bounds.
    let cell_m = f64::from(config.zone_cell_m);
    store.visit_reserved_in_bounds(
        bounds.expanded((CELL_DEPTH as f64 + GROUP_COLUMNS) * cell_m),
        |key| {
            if let Some(frame) = store.frame(key.grid) {
                frames.insert((frame, key.grid));
            }
        },
    );
    let world = DVec2::new(f64::from(config.width_m), f64::from(config.height_m)) * 0.5;
    let epsilon = road_contact_precision(CellBounds {
        min: -world,
        max: world,
    }) / cell_m;
    let frames: Vec<_> = frames.into_iter().collect();
    frames
        .par_iter()
        .flat_map_iter(|&(frame, grid)| {
            frame_candidates(store, frame, grid, graph, roads, bounds, epsilon)
        })
        .collect()
}

fn frame_candidates(
    store: &CellStore,
    frame: GridFrame,
    grid: u64,
    graph: &RegionGraph,
    roads: &[usize],
    bounds: CellBounds,
    epsilon: f64,
) -> Vec<Candidate> {
    // The line key is (along axis, inward depth sign, integer curb boundary).
    let mut lines: BTreeMap<(usize, i32, i32), Vec<Interval>> = BTreeMap::new();
    let (u, v) = frame.axes();
    let axes = [u, v];
    let cell_m = frame.cell_m();
    for &edge_id in roads {
        let edge = graph.edge(edge_id);
        if edge.deleted
            || edge.primary_type != TransitType::Road
            || edge.class != EdgeClass::Standard
            || edge.physical_geometry.len() < 2
        {
            continue;
        }
        let first = edge.physical_geometry[0];
        let last = edge.physical_geometry[edge.physical_geometry.len() - 1];
        let first = frame.local(DVec2::new(f64::from(first.x), f64::from(first.z)));
        let last = frame.local(DVec2::new(f64::from(last.x), f64::from(last.z)));
        let delta = last - first;
        let along = usize::from(delta.y.abs() > delta.x.abs());
        let depth = 1 - along;
        if delta[along].abs() <= epsilon {
            continue;
        }
        let mut min = first[along].min(last[along]);
        let mut max = first[along].max(last[along]);
        let straight = edge.physical_geometry.iter().all(|point| {
            let point = frame.local(DVec2::new(f64::from(point.x), f64::from(point.z)));
            min = min.min(point[along]);
            max = max.max(point[along]);
            (point[depth] - first[depth]).abs() <= epsilon
        });
        if !straight {
            continue;
        }
        let tangent = axes[along] * delta[along].signum();
        let right = DVec2::new(tangent.y, -tangent.x);
        let offset =
            (f64::from(edge.width) * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH)) / cell_m;
        for sign in [-1, 1] {
            let boundary = first[depth] + f64::from(sign) * offset;
            if (boundary - boundary.round()).abs() > epsilon {
                continue;
            }
            let side = if right.dot(axes[depth]) * f64::from(sign) > 0.0 {
                1
            } else {
                -1
            };
            lines
                .entry((along, sign, boundary.round() as i32))
                .or_default()
                .push(Interval {
                    min,
                    max,
                    edge: edge_id,
                    side,
                });
        }
    }
    let corners = [
        bounds.min,
        DVec2::new(bounds.max.x, bounds.min.y),
        bounds.max,
        DVec2::new(bounds.min.x, bounds.max.y),
    ]
    .map(|point| frame.local(point));
    let local_bounds = CellBounds::from_points(corners);
    let mut output = Vec::new();
    for ((along, sign, boundary), mut intervals) in lines {
        intervals.sort_unstable_by(|a, b| {
            a.min
                .total_cmp(&b.min)
                .then(a.max.total_cmp(&b.max))
                .then(a.edge.cmp(&b.edge))
        });
        let mut begin = 0;
        while begin < intervals.len() {
            let mut end = begin + 1;
            let mut limit = intervals[begin].max;
            while end < intervals.len() && intervals[end].min <= limit + epsilon {
                limit = limit.max(intervals[end].max);
                end += 1;
            }
            let first_column = (intervals[begin].min - epsilon)
                .ceil()
                .max(local_bounds.min[along].floor() - 1.0) as i32;
            let end_column = (limit + epsilon)
                .floor()
                .min(local_bounds.max[along].ceil() + 1.0) as i32;
            for column in first_column..end_column {
                let group_start = column.div_euclid(GROUP_COLUMNS as i32) * GROUP_COLUMNS as i32;
                let pinned = (group_start..group_start + GROUP_COLUMNS as i32).any(|column| {
                    (0..CELL_DEPTH).any(|row| {
                        let depth = boundary + sign * row as i32 - i32::from(sign < 0);
                        let (x, y) = if along == 0 {
                            (column, depth)
                        } else {
                            (depth, column)
                        };
                        let key = CellKey { grid, x, y };
                        store.profile(key).is_some_and(|profile| profile != 0)
                            || store.lot(key).is_some_and(|lot| lot != 0)
                    })
                });
                if !pinned {
                    continue;
                }
                let mut eligible_end = f64::from(column);
                for interval in &intervals[begin..end] {
                    if graph.edge(interval.edge).no_building_spawn || interval.max <= eligible_end {
                        continue;
                    }
                    if interval.min > eligible_end + epsilon {
                        break;
                    }
                    eligible_end = interval.max;
                    if eligible_end >= f64::from(column) + 1.0 - epsilon {
                        break;
                    }
                }
                let full_eligible_column = eligible_end >= f64::from(column) + 1.0 - epsilon;
                let suppliers = intervals[begin..end].iter().filter(|interval| {
                    interval.min < f64::from(column) + 1.0 - epsilon
                        && interval.max > f64::from(column) + epsilon
                });
                for supplier in suppliers {
                    // Full-column acceptance already includes the road-contact precision bound.
                    // Use it at the cell endpoints as well, so a rounded road terminal does not
                    // leave an artificial fractional gap in an otherwise complete frontage.
                    let start = if supplier.min <= f64::from(column) + epsilon {
                        0
                    } else {
                        ((supplier.min - f64::from(column)).clamp(0.0, 1.0)
                            * f64::from(CELL_FRONTAGE_UNITS))
                        .round() as u32
                    };
                    let end = if supplier.max >= f64::from(column) + 1.0 - epsilon {
                        CELL_FRONTAGE_UNITS
                    } else {
                        ((supplier.max - f64::from(column)).clamp(0.0, 1.0)
                            * f64::from(CELL_FRONTAGE_UNITS))
                        .round() as u32
                    };
                    for row in 0..CELL_DEPTH {
                        let depth = boundary + sign * row as i32 - i32::from(sign < 0);
                        let (x, y) = if along == 0 {
                            (column, depth)
                        } else {
                            (depth, column)
                        };
                        let key = CellKey { grid, x, y };
                        // A no-build road can supply metadata for retained lot claims until
                        // allocator policy processes them. It grants no new paintable frontage.
                        let claimed = store.lot(key).is_some_and(|lot| lot != 0);
                        if graph.edge(supplier.edge).no_building_spawn && !claimed {
                            continue;
                        }
                        if !full_eligible_column
                            && !claimed
                            && !store.profile(key).is_some_and(|profile| profile != 0)
                        {
                            continue;
                        }
                        let candidate = Candidate {
                            source: None,
                            pinned: true,
                            frame,
                            x,
                            y,
                            row: row as u8,
                            frontage: CellRoadFrontage {
                                start,
                                end,
                                edge: supplier.edge,
                                side: supplier.side,
                                boundary: match (along, sign) {
                                    (0, 1) => CellFrontage::MinY,
                                    (0, _) => CellFrontage::MaxY,
                                    (_, 1) => CellFrontage::MinX,
                                    _ => CellFrontage::MaxX,
                                },
                            },
                        };
                        if CellBounds::from_points(candidate.corners()).intersects(bounds) {
                            output.push(candidate);
                        }
                    }
                }
            }
            begin = end;
        }
    }
    output
}
