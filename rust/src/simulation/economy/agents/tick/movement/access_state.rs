//! Access-egress and access-ingress movement state handling.

mod egress;
mod ingress;

use super::super::slices::MovementSlices;
use crate::simulation::economy::agents::TRANSIT_IN_BUILDING;
use godot::prelude::Vector2;

pub(super) use egress::handle_access_egress;
pub(super) use ingress::handle_access_ingress;

/// Clears the authoritative access plan scalars for one agent.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn clear_access_plan(i: usize, slices: &MovementSlices) {
    unsafe {
        *slices.planned_attach_n.get_mut(i) = u32::MAX;
        *slices.planned_detach_n.get_mut(i) = u32::MAX;
        *slices.planned_attach_lane.get_mut(i) = u32::MAX;
        *slices.planned_detach_lane.get_mut(i) = u32::MAX;
        *slices.planned_attach_lane_d.get_mut(i) = 0.0;
        *slices.planned_detach_lane_d.get_mut(i) = 0.0;
        *slices.access_flags.get_mut(i) = 0;
    }
}

/// Resets an invalid access plan back to the specified building-local position.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn reset_invalid_access_plan(
    i: usize,
    current_building: usize,
    pos: Vector2,
    next_replan_time: f32,
    slices: &MovementSlices,
) {
    unsafe {
        *slices.pos_x.get_mut(i) = pos.x;
        *slices.pos_y.get_mut(i) = pos.y;
        *slices.cur_b.get_mut(i) = current_building;
        *slices.tgt_b.get_mut(i) = usize::MAX;
        clear_access_plan(i, slices);
        clear_network_state(i, slices);
        *slices.transit.get_mut(i) = TRANSIT_IN_BUILDING;
        *slices.next_replan_time.get_mut(i) = next_replan_time;
        *slices.network_replan_failures.get_mut(i) = 0;
    }
}

/// Completes an access-ingress trip and places the agent inside the destination building.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn arrive_in_building(
    i: usize,
    building_id: usize,
    pos: Vector2,
    sim_time: f32,
    slices: &MovementSlices,
) {
    unsafe {
        *slices.pos_x.get_mut(i) = pos.x;
        *slices.pos_y.get_mut(i) = pos.y;
        *slices.cur_b.get_mut(i) = building_id;
        *slices.tgt_b.get_mut(i) = usize::MAX;
        *slices.transit.get_mut(i) = TRANSIT_IN_BUILDING;

        let home = *slices.home.get(i);
        let work = *slices.work.get(i);
        *slices.activity.get_mut(i) = if building_id == home {
            0
        } else if building_id == work {
            1
        } else {
            2
        };

        clear_access_plan(i, slices);
        clear_network_state(i, slices);
        *slices.next_replan_time.get_mut(i) = 0.0;
        *slices.network_replan_failures.get_mut(i) = 0;

        let commute_time = sim_time - *slices.jstart.get(i);
        *slices.happiness.get_mut(i) =
            (*slices.happiness.get(i) - commute_time / 60.0).clamp(0.0, 100.0);
    }
}

unsafe fn clear_network_state(i: usize, slices: &MovementSlices) {
    unsafe {
        *slices.cur_n.get_mut(i) = u32::MAX;
        *slices.cur_e.get_mut(i) = usize::MAX;
        *slices.lane_id.get_mut(i) = usize::MAX;
        *slices.lane_d.get_mut(i) = 0.0;
        *slices.lane_change_from_lane.get_mut(i) = u32::MAX;
        *slices.lane_change_start_d.get_mut(i) = 0.0;
        *slices.lane_change_length.get_mut(i) = 0.0;
        *slices.overtake_blocked_time.get_mut(i) = 0.0;
        *slices.overtake_cooldown.get_mut(i) = 0.0;
        *slices.speed.get_mut(i) = 0.0;
        slices.path.get_mut(i).clear();
        *slices.path_idx.get_mut(i) = 0;
    }
}
