// SPDX-License-Identifier: GPL-2.0-only

//! Lane geometry construction and authoritative agent position sampling.

use super::super::graph::Edge;
use super::super::roadbed::RoadVec2;
use super::{Lane, LaneSuccessors, LaneType};
use crate::config;
use godot::prelude::*;
use std::collections::HashMap;

/// Builds the cumulative-distance prefix sum for a lane's geometry.
pub fn build_cum_dist(geometry: &[Vector3]) -> Vec<f32> {
    let mut v = Vec::with_capacity(geometry.len());
    let mut acc = 0.0;
    for i in 0..geometry.len() {
        if i > 0 {
            acc += geometry[i - 1].distance_to(geometry[i]);
        }
        v.push(acc);
    }
    v
}

/// Trims `dist` metres from the front of `geom`, returning the truncated polyline starting at
/// the interpolated cut point. Returns a single-point slice at the far end if `dist` exceeds
/// the total length.
fn trim_from_front(geom: &[Vector3], dist: f32) -> Vec<Vector3> {
    if dist <= 0.0 || geom.len() < 2 {
        return geom.to_vec();
    }
    let mut acc = 0.0;
    for i in 0..geom.len() - 1 {
        let seg = geom[i].distance_to(geom[i + 1]);
        if acc + seg >= dist {
            let t = ((dist - acc) / seg.max(1e-6)).clamp(0.0, 1.0);
            let cut = geom[i].lerp(geom[i + 1], t);
            let mut out = Vec::with_capacity(geom.len() - i);
            out.push(cut);
            out.extend_from_slice(&geom[i + 1..]);
            return out;
        }
        acc += seg;
    }
    vec![*geom.last().unwrap()]
}

/// Where a lane stops at one end of its edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LaneEnd {
    /// This many metres beyond the edge's junction clip, measured along the lane. Footpaths use
    /// it to stop at the sidewalk of the roads they meet, and road sidewalks at nodes without a
    /// junction layout to keep room for their corner routes.
    Inset(f32),
    /// Exactly at `point`, which lies `station_m` horizontal metres along the edge's centreline
    /// from the node: a road sidewalk's end in its node's junction layout (`NodeWalkways`).
    Layout {
        /// Horizontal centreline distance of the end from the node.
        station_m: f64,
        /// The end point.
        point: Vector3,
    },
    /// Exactly at `point`, a vehicle lane's point on its arm's mouth frame at `station_m`
    /// (`MouthFrame`, `TRAFFIC-12`). As [`Self::Layout`], but lane points less than
    /// `MOUTH_KNOT_MERGE_M` beyond the station are dropped too: the roadbed profile puts a knot at
    /// every mouth station, which would otherwise stay as a zero-length end piece whenever its
    /// station rounds beyond the frame's, so the lane's end piece runs along the arm.
    Mouth {
        /// Horizontal centreline distance of the end from the node.
        station_m: f64,
        /// The end point.
        point: Vector3,
    },
}

// Lane points closer than this beyond a mouth station are replaced by the mouth point.
const MOUTH_KNOT_MERGE_M: f64 = 1.0e-3;

impl LaneEnd {
    // The station beyond which lane points are kept, and the end point, of a layout or mouth
    // end.
    fn cut(self) -> Option<(f64, Vector3)> {
        match self {
            Self::Inset(_) => None,
            Self::Layout { station_m, point } => Some((station_m, point)),
            Self::Mouth { station_m, point } => Some((station_m + MOUTH_KNOT_MERGE_M, point)),
        }
    }
}

/// Builds geometry and appends one straight lane to `lanes`, updating `lane_map` and
/// `edge_lane_indices`. `ends` says where the lane stops at the edge's start and end.
#[allow(clippy::too_many_arguments)]
pub fn build_one_lane(
    lanes: &mut Vec<Lane>,
    lane_map: &mut HashMap<(usize, bool, i8), usize>,
    edge_lane_indices: &mut Vec<usize>,
    edge_idx: usize,
    edge: &Edge,
    is_fwd: bool,
    lane_idx: i8,
    lane_type: LaneType,
    lane_offset: f32,
    ends: [LaneEnd; 2],
) {
    let pts = &edge.physical_geometry;
    let mut geometry = Vec::with_capacity(pts.len());

    let mut dir0 = pts[1] - pts[0];
    dir0.y = 0.0;
    let t0 = if dir0.length() > 1e-5 {
        dir0.normalized()
    } else {
        Vector3::new(1.0, 0.0, 0.0)
    };
    let n0 = Vector3::new(-t0.z, 0.0, t0.x);
    geometry.push(pts[0] + n0 * lane_offset);

    for j in 1..pts.len() - 1 {
        let mut d1 = pts[j] - pts[j - 1];
        let mut d2 = pts[j + 1] - pts[j];
        d1.y = 0.0;
        d2.y = 0.0;
        let t1 = if d1.length() > 1e-5 {
            d1.normalized()
        } else {
            t0
        };
        let t2 = if d2.length() > 1e-5 {
            d2.normalized()
        } else {
            t1
        };
        let n1 = Vector3::new(-t1.z, 0.0, t1.x);
        let n2 = Vector3::new(-t2.z, 0.0, t2.x);
        let bisect = (n1 + n2).normalized();
        let dot = n1.dot(bisect).max(0.1);
        geometry.push(pts[j] + bisect * (lane_offset / dot));
    }

    let mut d_last = pts[pts.len() - 1] - pts[pts.len() - 2];
    d_last.y = 0.0;
    let t_last = if d_last.length() > 1e-5 {
        d_last.normalized()
    } else {
        t0
    };
    let n_last = Vector3::new(-t_last.z, 0.0, t_last.x);
    geometry.push(pts[pts.len() - 1] + n_last * lane_offset);

    // A layout end replaces every lane point at or short of its station with the layout's own
    // point, so the lane ends exactly where the node's pedestrian connectors start. The points
    // are still index-aligned with `pts` here.
    if ends.iter().any(|end| end.cut().is_some()) {
        geometry = cut_at_layout_ends(&geometry, pts, ends);
    }
    if let LaneEnd::Inset(inset) = ends[0]
        && edge.start_clip + inset > 0.0
    {
        geometry = trim_from_front(&geometry, edge.start_clip + inset);
    }
    if let LaneEnd::Inset(inset) = ends[1]
        && edge.end_clip + inset > 0.0
    {
        geometry.reverse();
        geometry = trim_from_front(&geometry, edge.end_clip + inset);
        geometry.reverse();
    }

    if geometry.len() < 2 {
        return; // Edge too short after clipping — skip.
    }

    if !is_fwd {
        geometry.reverse();
    }

    let mut length = 0.0;
    for j in 0..geometry.len() - 1 {
        length += geometry[j].distance_to(geometry[j + 1]);
    }

    let cum_dist = build_cum_dist(&geometry);
    let new_lane_id = lanes.len();
    lanes.push(Lane {
        edge_id: edge_idx,
        is_fwd,
        lane_idx,
        geometry,
        length,
        frontage_delay_penalty_s: 0.0,
        live_frontage_delay_s: 0.0,
        cum_dist,
        lane_type,
        next_lanes: LaneSuccessors::default(),
        node_id: usize::MAX,
    });
    lane_map.insert((edge_idx, is_fwd, lane_idx), new_lane_id);
    edge_lane_indices.push(new_lane_id);
}

// Keeps the lane points strictly beyond each layout end's station and puts the layout point in
// front of them. `geometry` is index-aligned with the centreline `pts`, whose horizontal
// distance from each end is the layout's station measure. O(points).
fn cut_at_layout_ends(geometry: &[Vector3], pts: &[Vector3], ends: [LaneEnd; 2]) -> Vec<Vector3> {
    // The arms' station measure (`roadbed::NodeArms`).
    let xz = |point: Vector3| RoadVec2::new(f64::from(point.x), f64::from(point.z));
    let horizontal = |a: Vector3, b: Vector3| xz(a).distance(xz(b));
    // First index, in walking order, whose station from the walk's first point exceeds
    // `station_m`.
    let beyond = |indices: &mut dyn Iterator<Item = usize>, station_m: f64| {
        let mut walked_m = 0.0;
        let mut previous: Option<usize> = None;
        for index in indices {
            if let Some(previous) = previous {
                walked_m += horizontal(pts[previous], pts[index]);
            }
            if walked_m > station_m {
                return Some(index);
            }
            previous = Some(index);
        }
        None
    };
    let count = geometry.len();
    let (first, start_point) = match ends[0].cut() {
        Some((station_m, point)) => (
            beyond(&mut (0..count), station_m).unwrap_or(count),
            Some(point),
        ),
        None => (0, None),
    };
    let (end, end_point) = match ends[1].cut() {
        Some((station_m, point)) => (
            beyond(&mut (0..count).rev(), station_m).map_or(0, |index| index + 1),
            Some(point),
        ),
        None => (count, None),
    };
    let kept = geometry.get(first..end).unwrap_or_default();
    let mut cut = Vec::with_capacity(kept.len() + 2);
    cut.extend(start_point);
    cut.extend_from_slice(kept);
    cut.extend(end_point);
    cut
}

/// Returns the half-width of the road asphalt based on the number of lanes.
pub fn road_half_width(edge: &Edge) -> f32 {
    ((edge.fwd_lanes + edge.bkw_lanes) as f32) * config::LANE_WIDTH * 0.5
}

/// Samples the authoritative agent position, including the deterministic pedestrian offset.
/// Movement and snapshot recovery share the same arithmetic.
pub(crate) fn agent_lane_position(
    lane: &Lane,
    dist: f32,
    pedestrian_index: Option<usize>,
) -> Option<Vector3> {
    if dist <= 0.0 {
        return lane.geometry.first().copied();
    }
    if dist >= lane.length {
        return lane.geometry.last().copied();
    }
    if lane.geometry.len() < 2 || lane.cum_dist.is_empty() {
        return None;
    }
    let seg = lane
        .cum_dist
        .partition_point(|&d| d <= dist)
        .saturating_sub(1)
        .min(lane.geometry.len() - 2);
    let p0 = lane.geometry[seg];
    let p1 = lane.geometry[seg + 1];
    let seg_len = lane.cum_dist[seg + 1] - lane.cum_dist[seg];
    let t = if seg_len > 1e-5 {
        (dist - lane.cum_dist[seg]) / seg_len
    } else {
        0.0
    };
    let mut out = p0.lerp(p1, t.clamp(0.0, 1.0));
    if let Some(i) = pedestrian_index.filter(|_| seg_len > 1e-5) {
        let tangent = (p1 - p0) / seg_len;
        let normal = Vector3::new(-tangent.z, 0.0, tangent.x);
        let jitter = (f32::sin(i as f32 * 4.0) + f32::cos(i as f32 * 7.0)) * 0.7;
        out += normal * jitter;
    }
    Some(out)
}
