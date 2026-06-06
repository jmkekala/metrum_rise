//! Connector-lane exit handling.

use super::super::super::super::slices::MovementSlices;
use super::LaneEndAction;
use crate::simulation::network::TransitNetwork;
use crate::traffic_log;

pub(super) unsafe fn exit_connector_lane(
    i: usize,
    lane_id: usize,
    remaining_dist: f32,
    transit_network: &TransitNetwork,
    slices: &MovementSlices,
) -> LaneEndAction {
    unsafe {
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_cur_e = &slices.cur_e;

        let lane = &transit_network.lane_system.lanes[lane_id];
        if !lane.next_lanes.is_empty() {
            let tgt_road_lane = lane.next_lanes[0];
            if tgt_road_lane < transit_network.lane_system.lanes.len() {
                let target_edge = transit_network.lane_system.lanes[tgt_road_lane].edge_id;
                traffic_log!(
                    "[JUNCTION_EXIT] agent={} node={} conn_lane={} conn_len={:.2} to_lane={} to_edge={} speed={:.2} remaining_dist={:.2} path_idx={}/{}",
                    i,
                    lane.node_id,
                    lane_id,
                    lane.length,
                    tgt_road_lane,
                    target_edge,
                    *s_speed.get(i),
                    remaining_dist,
                    *s_path_idx.get(i),
                    s_path.get(i).len(),
                );
                *s_lane_id.get_mut(i) = tgt_road_lane;
                *s_lane_d.get_mut(i) = 0.0;
                *s_transit.get_mut(i) = super::super::super::super::super::TRANSIT_NETWORK;
                *s_cur_e.get_mut(i) = target_edge;
                return LaneEndAction::KeepMoving;
            }
            traffic_log!(
                "[JUNCTION_MISSING_EXIT] agent={} node={} conn_lane={} conn_len={:.2} next_lane={} reason=invalid-target-lane",
                i,
                lane.node_id,
                lane_id,
                lane.length,
                tgt_road_lane,
            );
        } else {
            traffic_log!(
                "[JUNCTION_MISSING_EXIT] agent={} node={} conn_lane={} conn_len={:.2} reason=no-next-lane",
                i,
                lane.node_id,
                lane_id,
                lane.length,
            );
        }
        s_path.get_mut(i).clear();
        *s_lane_id.get_mut(i) = usize::MAX;
        LaneEndAction::Break
    }
}
