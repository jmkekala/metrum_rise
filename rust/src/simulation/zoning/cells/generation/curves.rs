// SPDX-License-Identifier: GPL-2.0-only

//! Reattaches pinned curved groups by exact local centreline coverage after graph edits.

use super::super::CELL_FRONTAGE_UNITS;
use super::super::geometry::road_contact_precision;
use super::*;

struct Interval {
    min: f64,
    max: f64,
    edge: usize,
    side: i8,
}

/// Rebuilds local reserved-group suppliers without changing their original lattice geometry.
pub(super) fn pinned_candidates(
    store: &CellStore,
    graph: &RegionGraph,
    roads: &[usize],
    bounds: CellBounds,
) -> Vec<Candidate> {
    if !store.has_reservations() {
        return Vec::new();
    }
    store
        .curve_sources_in_bounds(bounds)
        .par_iter()
        .filter(|source| store.curve_source_reserved(source))
        .flat_map_iter(|source| source_candidates(store, source, graph, roads, bounds))
        .collect()
}

fn source_candidates(
    store: &CellStore,
    source: &Arc<CellCurveSource>,
    graph: &RegionGraph,
    roads: &[usize],
    bounds: CellBounds,
) -> Vec<Candidate> {
    let key = source.key;
    let (along, depth, sign) = key.axes();
    let frame = key.frame;
    let source_bounds =
        CellBounds::from_points(source.points.iter().copied().map(DVec2::from_array));
    // Road subdivision evaluates interpolation in f32. Two rounded arithmetic stages can
    // move a new point by up to 2 * machine epsilon * coordinate magnitude. Apply that
    // representation bound only to guide identity/intervals; footprint predicates stay exact.
    let scale = source_bounds
        .min
        .abs()
        .max(source_bounds.max.abs())
        .max_element()
        .max((source_bounds.max - source_bounds.min).length());
    let epsilon = road_contact_precision(key.bounds()) + 2.0 * f64::from(f32::EPSILON) * scale;
    let mut intervals = Vec::new();
    // Every original segment must remain covered. A nearby substitute road cannot supply a
    // floating lot merely because its projection happens to cross the same frontage.
    for pair in source.points.windows(2) {
        let a = DVec2::from_array(pair[0]);
        let b = DVec2::from_array(pair[1]);
        let length = a.distance(b);
        let tangent = (b - a) / length;
        let mut coverage = Vec::new();
        for &edge_id in roads {
            let edge = graph.edge(edge_id);
            if edge.deleted
                || edge.primary_type != TransitType::Road
                || edge.class != EdgeClass::Standard
                || f64::from(edge.width) * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH)
                    != source.half_width
            {
                continue;
            }
            for segment in edge.physical_geometry.windows(2) {
                let c = DVec2::new(f64::from(segment[0].x), f64::from(segment[0].z));
                let d = DVec2::new(f64::from(segment[1].x), f64::from(segment[1].z));
                if (c - a).perp_dot(tangent).abs() > epsilon
                    || (d - a).perp_dot(tangent).abs() > epsilon
                {
                    continue;
                }
                let c_t = (c - a).dot(tangent);
                let d_t = (d - a).dot(tangent);
                let min = c_t.min(d_t).max(0.0);
                let max = c_t.max(d_t).min(length);
                if max <= min {
                    continue;
                }
                coverage.push((min, max));
                let u = frame.local(a + tangent * min)[along];
                let v = frame.local(a + tangent * max)[along];
                intervals.push(Interval {
                    min: u.min(v),
                    max: u.max(v),
                    edge: edge_id,
                    side: source.side * if d_t > c_t { 1 } else { -1 },
                });
            }
        }
        coverage.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut end = 0.0_f64;
        for (min, max) in coverage {
            if min > end + epsilon {
                break;
            }
            end = end.max(max);
        }
        if end < length - epsilon {
            return Vec::new();
        }
    }
    // Nonterminal rigid groups intentionally complete their final square beyond the chord.
    // Keep that original contract while assigning the extension to the endpoint supplier.
    let first = frame.local(DVec2::from_array(source.points[0]))[along];
    let last = frame.local(DVec2::from_array(source.points[source.points.len() - 1]))[along];
    let low = first.min(last);
    let high = first.max(last);
    let epsilon = epsilon / frame.cell_m();
    for interval in &mut intervals {
        if interval.min <= low + epsilon {
            interval.min = interval.min.min(f64::from(key.min[along]));
        }
        if interval.max >= high - epsilon {
            interval.max = interval.max.max(f64::from(key.max[along]));
        }
    }
    // One edge can contribute many sampled segments. Merge its contiguous intervals before
    // producing cells, so terrain sampling density does not multiply candidate publication.
    intervals.sort_unstable_by(|a, b| {
        (a.edge, a.side)
            .cmp(&(b.edge, b.side))
            .then(a.min.total_cmp(&b.min))
            .then(a.max.total_cmp(&b.max))
    });
    let mut merged: Vec<Interval> = Vec::with_capacity(intervals.len());
    for interval in intervals {
        if let Some(last) = merged.last_mut()
            && last.edge == interval.edge
            && last.side == interval.side
            && interval.min <= last.max + epsilon
        {
            last.max = last.max.max(interval.max);
        } else {
            merged.push(interval);
        }
    }
    let mut intervals = merged;
    intervals.sort_unstable_by(|a, b| {
        a.min
            .total_cmp(&b.min)
            .then(a.max.total_cmp(&b.max))
            .then(a.edge.cmp(&b.edge))
    });
    let mut output = Vec::new();
    // Resolve the existing grid in O(1), without registering or changing it during Rayon work.
    let Some(grid) = store.grid_for_frame(frame) else {
        return output;
    };
    for column in key.min[along]..key.max[along] {
        let begin = f64::from(column);
        let end = begin + 1.0;
        let mut eligible_end = begin;
        for interval in &intervals {
            if graph.edge(interval.edge).no_building_spawn || interval.max <= eligible_end {
                continue;
            }
            if interval.min > eligible_end + epsilon {
                break;
            }
            eligible_end = interval.max;
            if eligible_end >= end - epsilon {
                break;
            }
        }
        for interval in &intervals {
            if interval.max <= begin + epsilon || interval.min >= end - epsilon {
                continue;
            }
            let fraction = |value: f64| {
                if value <= begin + epsilon {
                    0
                } else if value >= end - epsilon {
                    CELL_FRONTAGE_UNITS
                } else {
                    ((value - begin) * f64::from(CELL_FRONTAGE_UNITS)).round() as u32
                }
            };
            for row in 0..CELL_DEPTH {
                let mut xy = key.min;
                xy[along] = column;
                xy[depth] = if sign > 0 {
                    key.min[depth] + row as i32
                } else {
                    key.max[depth] - 1 - row as i32
                };
                let cell = CellKey {
                    grid,
                    x: xy[0],
                    y: xy[1],
                };
                let claimed = store.lot(cell).is_some_and(|lot| lot != 0);
                if (graph.edge(interval.edge).no_building_spawn && !claimed)
                    || (eligible_end < end - epsilon
                        && !claimed
                        && !store.profile(cell).is_some_and(|p| p != 0))
                {
                    continue;
                }
                let candidate = Candidate {
                    pinned: true,
                    frame,
                    x: xy[0],
                    y: xy[1],
                    row: row as u8,
                    source: Some(Arc::clone(source)),
                    frontage: CellRoadFrontage {
                        edge: interval.edge,
                        side: interval.side,
                        boundary: key.frontage,
                        start: fraction(interval.min),
                        end: fraction(interval.max),
                    },
                };
                if CellBounds::from_points(candidate.corners()).intersects(bounds) {
                    output.push(candidate);
                }
            }
        }
    }
    output
}
