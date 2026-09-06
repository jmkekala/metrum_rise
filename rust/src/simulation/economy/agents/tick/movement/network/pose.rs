// SPDX-License-Identifier: GPL-2.0-only

//! Simulation-side lane-distance to position sampling.

use super::super::super::super::MODE_WALK;
use super::super::super::slices::MovementSlices;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::lanes::geometry::agent_lane_position;

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
        if let Some(pos) = agent_lane_position(
            lane,
            *s_lane_d.get(i),
            (*s_tmode.get(i) == MODE_WALK).then_some(i),
        ) {
            *s_pos_x.get_mut(i) = pos.x;
            *s_pos_y.get_mut(i) = pos.z;
        }
    }
}
