//! Zero-hop access connector handling at the destination junction neck.

use super::super::super::super::super::ACCESS_PLAN_VALID;
use super::super::super::super::lane_nav::lane_origin_node;
use super::super::super::super::slices::MovementSlices;
use super::LaneEndAction;
use super::enter::enter_detach_lane_connector;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use std::sync::atomic::AtomicBool;

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn try_zero_hop_access_connector(
    i: usize,
    lane_id: usize,
    from_edge: usize,
    path_idx: usize,
    path_len: usize,
    speed: f32,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_attach_claimed: &[AtomicBool],
    slices: &MovementSlices,
) -> Option<LaneEndAction> {
    unsafe {
        let access_plan_valid = (*slices.access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
        if !access_plan_valid
            || *slices.cur_n.get(i) != *slices.planned_detach_n.get(i)
            || *slices.planned_detach_lane.get(i) == u32::MAX
        {
            return None;
        }

        let detach_lane_id = *slices.planned_detach_lane.get(i) as usize;
        let Some(detach_origin) = lane_origin_node(detach_lane_id, transit_network, graph) else {
            return None;
        };
        if detach_origin != *slices.planned_detach_n.get(i) {
            return None;
        }

        enter_detach_lane_connector(
            i,
            lane_id,
            from_edge,
            detach_lane_id,
            *slices.cur_n.get(i),
            path_idx,
            path_len,
            speed,
            remaining_dist,
            transit_network,
            lane_buckets,
            lane_attach_claimed,
            slices,
        )
    }
}
