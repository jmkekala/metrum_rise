// SPDX-License-Identifier: GPL-2.0-only

//! Network-lane detach into local access ingress movement.

use super::super::super::super::{ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_ACCESS_INGRESS};
use super::super::super::access::{local_access_point, local_access_side_label};
use super::super::super::claims::LaneClaimContext;
use super::super::super::lane_nav::lane_terminal_node;
use super::super::super::slices::MovementSlices;
use super::super::super::traffic::claim_lane_entry;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;

/// Attempts to detach from the current network lane into the target building's access path.
///
/// The detach rule (`TRAFFIC-15`, `TRAFFIC-13`): a car detaches once its movement step has
/// reached `reached_d` at or past the planned detach distance on the planned detach lane, and
/// only on the route's last visit to that lane, when no route node is left beyond the lane's
/// terminal node. A step that ends inside the lane reaches its `lane_d`; a step that reaches the
/// lane's end reaches `length + ACCESS_LANE_D_TOLERANCE_M`, the planner's legality bound, so every
/// legal detach point is reachable. O(1), no allocation.
///
/// Returns `true` when movement should stop for this tick.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn try_network_detach(
    i: usize,
    lane_id: usize,
    reached_d: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_tgt_b = &slices.tgt_b;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_pos_x = &slices.pos_x;
        let s_pos_y = &slices.pos_y;
        let s_cur_e = &slices.cur_e;

        let access_plan_valid = (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
        let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
        if !access_plan_valid
            || planned_detach_lane_id == usize::MAX
            || lane_id != planned_detach_lane_id
            || reached_d < *s_plan_detach_lane_d.get(i)
            || route_continues_past_lane(
                s_path.get(i),
                *s_path_idx.get(i),
                lane_terminal_node(lane_id, transit_network, graph),
            )
        {
            return false;
        }

        let detach_d = *s_plan_detach_lane_d.get(i);
        let detach_allowed = if *s_tmode.get(i) == MODE_CAR {
            claim_lane_entry(i, planned_detach_lane_id, lane_claims)
        } else {
            true
        };
        if detach_allowed {
            let t_bldg_idx = *s_tgt_b.get(i);
            if t_bldg_idx < allocator.entrances.len()
                && let Some(ingress_origin) = local_access_point(
                    *s_tmode.get(i),
                    &allocator.entrances[t_bldg_idx],
                    planned_detach_lane_id,
                    detach_d,
                    transit_network,
                )
            {
                *s_pos_x.get_mut(i) = ingress_origin.x;
                *s_pos_y.get_mut(i) = ingress_origin.y;
                s_path.get_mut(i).clear();
                *s_path_idx.get_mut(i) = 0;
                *s_cur_n.get_mut(i) = u32::MAX;
                *s_cur_e.get_mut(i) = usize::MAX;
                *s_lane_id.get_mut(i) = usize::MAX;
                *s_lane_d.get_mut(i) = 0.0;
                *s_speed.get_mut(i) = 0.0;
                *s_transit.get_mut(i) = TRANSIT_ACCESS_INGRESS;
                if crate::debug::Channel::Traffic.enabled() {
                    let entrance = &allocator.entrances[t_bldg_idx];
                    traffic_log!(
                        "[ACCESS_INGRESS_DETACH] agent={} target_bldg={} lane={}({}) lane_d={:.2} ingress_origin=({:.2},{:.2}) detach_node={} path_count={} flags=0x{:02x}",
                        i,
                        t_bldg_idx,
                        planned_detach_lane_id,
                        local_access_side_label(*s_tmode.get(i), entrance, planned_detach_lane_id,),
                        detach_d,
                        ingress_origin.x,
                        ingress_origin.y,
                        *s_plan_detach_n.get(i),
                        s_path.get(i).len(),
                        *s_access_flags.get(i),
                    );
                }
                return true;
            }
            return false;
        }

        if crate::debug::Channel::Traffic.enabled() {
            let target_entrance = allocator.entrances.get(*s_tgt_b.get(i));
            let side = target_entrance
                .map(|entrance| {
                    local_access_side_label(*s_tmode.get(i), entrance, planned_detach_lane_id)
                })
                .unwrap_or("unknown-target");
            traffic_log!(
                "[ACCESS_INGRESS_WAIT] agent={} target_bldg={} lane={}({}) lane_d={:.2} reason=detach-slot-busy",
                i,
                *s_tgt_b.get(i),
                planned_detach_lane_id,
                side,
                detach_d,
            );
        }
        // Wait at the detach point; a lane-end point within the tolerance waits at the end.
        *s_lane_d.get_mut(i) = detach_d.min(transit_network.lane_system.lanes[lane_id].length);
        *s_speed.get_mut(i) = 0.0;
        true
    }
}

// Whether the route still has a node to reach after this lane's terminal node: `path_idx` names
// the next node, which is the current lane's terminal while the car drives toward it (the same
// reading as `planned_next_connector`). A route that leaves the detach lane and comes back to a
// detach point behind the car is not finished on its first visit (`TRAFFIC-13`).
fn route_continues_past_lane(path: &[u32], path_idx: usize, terminal: Option<u32>) -> bool {
    let next_idx = if terminal.is_some() && path.get(path_idx).copied() == terminal {
        path_idx + 1
    } else {
        path_idx
    };
    next_idx < path.len()
}
