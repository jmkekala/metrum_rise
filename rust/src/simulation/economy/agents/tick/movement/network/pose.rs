// SPDX-License-Identifier: GPL-2.0-only

//! Simulation-side lane-distance to position sampling.

use super::super::super::super::MODE_WALK;
use super::super::super::slices::MovementSlices;
use crate::simulation::network::TransitNetwork;
use godot::prelude::Vector3;

/// Writes the current network lane position into the agent's simulation position fields.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn update_network_pose(
    i: usize,
    transit_network: &TransitNetwork,
    slices: &MovementSlices,
) {
    unsafe {
        let s_tmode = &slices.tmode;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_pos_x = &slices.pos_x;
        let s_pos_y = &slices.pos_y;

        let current_lane = *s_lane_id.get(i);
        if current_lane == usize::MAX || current_lane >= transit_network.lane_system.lanes.len() {
            return;
        }

        let lane = &transit_network.lane_system.lanes[current_lane];
        let dist = *s_lane_d.get(i);
        if dist <= 0.0 && !lane.geometry.is_empty() {
            *s_pos_x.get_mut(i) = lane.geometry[0].x;
            *s_pos_y.get_mut(i) = lane.geometry[0].z;
        } else if dist >= lane.length && !lane.geometry.is_empty() {
            let end = lane.geometry.last().unwrap();
            *s_pos_x.get_mut(i) = end.x;
            *s_pos_y.get_mut(i) = end.z;
        } else if lane.geometry.len() >= 2 && !lane.cum_dist.is_empty() {
            let seg = lane
                .cum_dist
                .partition_point(|&d| d <= dist)
                .saturating_sub(1);
            let seg = seg.min(lane.geometry.len() - 2);
            let p0 = lane.geometry[seg];
            let p1 = lane.geometry[seg + 1];
            let seg_len = lane.cum_dist[seg + 1] - lane.cum_dist[seg];
            let t = if seg_len > 1e-5 {
                (dist - lane.cum_dist[seg]) / seg_len
            } else {
                0.0
            };
            let mut out = p0.lerp(p1, t.clamp(0.0, 1.0));
            if *s_tmode.get(i) == MODE_WALK && seg_len > 1e-5 {
                let tangent = (p1 - p0) / seg_len;
                let normal = Vector3::new(-tangent.z, 0.0, tangent.x);
                let jitter = (f32::sin(i as f32 * 4.0) + f32::cos(i as f32 * 7.0)) * 0.7;
                out += normal * jitter;
            }
            *s_pos_x.get_mut(i) = out.x;
            *s_pos_y.get_mut(i) = out.z;
        }
    }
}
