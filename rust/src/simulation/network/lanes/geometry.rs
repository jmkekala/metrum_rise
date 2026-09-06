// SPDX-License-Identifier: GPL-2.0-only

//! Lane geometry construction and authoritative agent position sampling.

use super::super::graph::Edge;
use super::super::types::TransitType;
use super::{Lane, LaneType};
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

/// Builds geometry and appends one straight lane to `lanes`, updating `lane_map` and `edge_lane_indices`.
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

    // Road sidewalks stop at the crosswalk mouth, while vehicle lanes stop at the asphalt
    // junction throat. This keeps the physical sidewalk endpoint identical to the first/last
    // point of every pedestrian connector and prevents walkers from passing the zebra before
    // returning to it.
    let junction_inset = if lane_type == LaneType::Foot && edge.primary_type == TransitType::Road {
        config::CROSSWALK_INSET
    } else {
        0.0
    };
    let start_clip = edge.start_clip + junction_inset;
    let end_clip = edge.end_clip + junction_inset;
    if start_clip > 0.0 {
        geometry = trim_from_front(&geometry, start_clip);
    }
    if end_clip > 0.0 {
        geometry.reverse();
        geometry = trim_from_front(&geometry, end_clip);
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
        cum_dist,
        lane_type,
        crosswalk_edge_id: None,
        crosswalk_marking: None,
        next_lanes: Vec::new(),
        node_id: usize::MAX,
    });
    lane_map.insert((edge_idx, is_fwd, lane_idx), new_lane_id);
    edge_lane_indices.push(new_lane_id);
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
