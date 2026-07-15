//! Connector entry, wait, and claim handling.

use super::super::super::super::super::{MODE_CAR, TRANSIT_INTERSECTION};
use super::super::super::super::claims::LaneClaimContext;
use super::super::super::super::lane_nav::{
    collect_connector_lanes_to_edge, collect_connector_lanes_to_lane,
};
use super::super::super::super::slices::MovementSlices;
use super::super::super::super::traffic::{
    ConnectorEntry, claim_connector_entry, junction_car_speed, junction_entry_speed,
};
use super::LaneEndAction;
use crate::simulation::network::TransitNetwork;
use crate::traffic_log;
use std::cell::RefCell;

thread_local! {
    static VALID_CONNS: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
}

const CONNECTOR_ENTRY_RETAIN_EPS_M: f32 = 0.05;

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn enter_next_edge_connector(
    i: usize,
    lane_id: usize,
    from_edge: usize,
    to_edge: usize,
    node_id: u32,
    path_idx_before_lane_end: usize,
    path_len: usize,
    speed: f32,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) -> LaneEndAction {
    unsafe {
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let connector_entry = claim_connector_to_edge(
            i,
            lane_id,
            to_edge,
            connector_choice_seed(i, lane_id, to_edge, node_id, *s_path_idx.get(i)),
            transit_network,
            lane_buckets,
            lane_claims,
        );

        match connector_entry {
            ConnectorEntry::Enter(chosen_conn) => {
                enter_connector_lane(
                    i,
                    chosen_conn,
                    speed,
                    remaining_dist,
                    transit_network,
                    slices,
                );
                if crate::debug::is_traffic_enabled() {
                    let conn_lane = &transit_network.lane_system.lanes[chosen_conn];
                    let target_lane = conn_lane.next_lanes.first().copied().unwrap_or(usize::MAX);
                    let target_edge = transit_network
                        .lane_system
                        .lanes
                        .get(target_lane)
                        .map(|lane| lane.edge_id)
                        .unwrap_or(usize::MAX);
                    traffic_log!(
                        "[JUNCTION_ENTER] agent={} node={} from_lane={} from_edge={} conn_lane={} conn_len={:.2} to_lane={} to_edge={} speed={:.2} remaining_dist={:.2} path_idx={}/{}",
                        i,
                        node_id,
                        lane_id,
                        from_edge,
                        chosen_conn,
                        conn_lane.length,
                        target_lane,
                        target_edge,
                        *slices.speed.get(i),
                        *remaining_dist,
                        *s_path_idx.get(i),
                        path_len,
                    );
                }
                LaneEndAction::KeepMoving
            }
            ConnectorEntry::ClaimedThisTick => {
                *s_path_idx.get_mut(i) = path_idx_before_lane_end;
                *s_lane_d.get_mut(i) = transit_network.lane_system.lanes[lane_id].length;
                traffic_log!(
                    "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_edge={} path_idx={}/{} reason=connector-entry-claimed",
                    i,
                    node_id,
                    lane_id,
                    from_edge,
                    to_edge,
                    *s_path_idx.get(i),
                    path_len,
                );
                LaneEndAction::Break
            }
            ConnectorEntry::Occupied => {
                *s_path_idx.get_mut(i) = path_idx_before_lane_end;
                *s_lane_d.get_mut(i) = transit_network.lane_system.lanes[lane_id].length;
                traffic_log!(
                    "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_edge={} path_idx={}/{} reason=connector-occupied",
                    i,
                    node_id,
                    lane_id,
                    from_edge,
                    to_edge,
                    *s_path_idx.get(i),
                    path_len,
                );
                LaneEndAction::Break
            }
            ConnectorEntry::MissingConnection => {
                traffic_log!(
                    "[JUNCTION_MISSING_CONN] agent={} node={} from_lane={} from_edge={} to_edge={} path_idx={}/{} reason=no-connection-lane",
                    i,
                    node_id,
                    lane_id,
                    from_edge,
                    to_edge,
                    *s_path_idx.get(i),
                    path_len,
                );
                s_path.get_mut(i).clear();
                *s_lane_id.get_mut(i) = usize::MAX;
                LaneEndAction::Break
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn enter_detach_lane_connector(
    i: usize,
    lane_id: usize,
    from_edge: usize,
    detach_lane_id: usize,
    node_id: u32,
    path_idx: usize,
    path_len: usize,
    speed: f32,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) -> Option<LaneEndAction> {
    unsafe {
        let s_path_idx = &slices.path_idx;
        let s_lane_d = &slices.lane_d;
        let s_speed = &slices.speed;
        let connector_entry = claim_connector_to_lane(
            i,
            lane_id,
            detach_lane_id,
            connector_choice_seed(i, lane_id, detach_lane_id, node_id, *s_path_idx.get(i)),
            transit_network,
            lane_buckets,
            lane_claims,
        );

        match connector_entry {
            ConnectorEntry::Enter(conn_lane_id) => {
                enter_connector_lane(
                    i,
                    conn_lane_id,
                    speed,
                    remaining_dist,
                    transit_network,
                    slices,
                );
                if crate::debug::is_traffic_enabled() {
                    let conn_lane = &transit_network.lane_system.lanes[conn_lane_id];
                    let target_edge = transit_network
                        .lane_system
                        .lanes
                        .get(detach_lane_id)
                        .map(|lane| lane.edge_id)
                        .unwrap_or(usize::MAX);
                    traffic_log!(
                        "[JUNCTION_ENTER] agent={} node={} from_lane={} from_edge={} conn_lane={} conn_len={:.2} to_lane={} to_edge={} speed={:.2} remaining_dist={:.2} path_idx={}/{} reason=zero-hop-access",
                        i,
                        node_id,
                        lane_id,
                        from_edge,
                        conn_lane_id,
                        conn_lane.length,
                        detach_lane_id,
                        target_edge,
                        *s_speed.get(i),
                        *remaining_dist,
                        *s_path_idx.get(i),
                        path_len,
                    );
                }
                Some(LaneEndAction::Continue)
            }
            ConnectorEntry::Occupied => {
                *s_path_idx.get_mut(i) = path_idx.min(path_len);
                *s_lane_d.get_mut(i) = transit_network.lane_system.lanes[lane_id].length;
                *s_speed.get_mut(i) = 0.0;
                traffic_log!(
                    "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_lane={} path_idx={}/{} reason=zero-hop-connector-occupied",
                    i,
                    node_id,
                    lane_id,
                    from_edge,
                    detach_lane_id,
                    *s_path_idx.get(i),
                    path_len,
                );
                Some(LaneEndAction::Break)
            }
            ConnectorEntry::ClaimedThisTick => {
                *s_path_idx.get_mut(i) = path_idx.min(path_len);
                *s_lane_d.get_mut(i) = transit_network.lane_system.lanes[lane_id].length;
                *s_speed.get_mut(i) = 0.0;
                traffic_log!(
                    "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_lane={} path_idx={}/{} reason=zero-hop-connector-entry-claimed",
                    i,
                    node_id,
                    lane_id,
                    from_edge,
                    detach_lane_id,
                    *s_path_idx.get(i),
                    path_len,
                );
                Some(LaneEndAction::Break)
            }
            ConnectorEntry::MissingConnection => {
                traffic_log!(
                    "[JUNCTION_MISSING_CONN] agent={} node={} from_lane={} from_edge={} to_lane={} path_idx={}/{} reason=zero-hop-no-connection-lane",
                    i,
                    node_id,
                    lane_id,
                    from_edge,
                    detach_lane_id,
                    *s_path_idx.get(i),
                    path_len,
                );
                None
            }
        }
    }
}

fn claim_connector_to_edge(
    agent_idx: usize,
    lane_id: usize,
    edge_id: usize,
    seed: u64,
    transit_network: &TransitNetwork,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_claims: &LaneClaimContext<'_>,
) -> ConnectorEntry {
    VALID_CONNS.with(|v| {
        let mut connector_candidates = v.borrow_mut();
        let any_routing_valid = collect_connector_lanes_to_edge(
            lane_id,
            edge_id,
            transit_network,
            &mut connector_candidates,
        );
        claim_connector_entry(
            agent_idx,
            &mut connector_candidates,
            any_routing_valid,
            seed,
            lane_buckets,
            lane_claims,
        )
    })
}

fn claim_connector_to_lane(
    agent_idx: usize,
    lane_id: usize,
    target_lane_id: usize,
    seed: u64,
    transit_network: &TransitNetwork,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_claims: &LaneClaimContext<'_>,
) -> ConnectorEntry {
    VALID_CONNS.with(|v| {
        let mut connector_candidates = v.borrow_mut();
        let any_routing_valid = collect_connector_lanes_to_lane(
            lane_id,
            target_lane_id,
            transit_network,
            &mut connector_candidates,
        );
        claim_connector_entry(
            agent_idx,
            &mut connector_candidates,
            any_routing_valid,
            seed,
            lane_buckets,
            lane_claims,
        )
    })
}

unsafe fn enter_connector_lane(
    i: usize,
    conn_lane_id: usize,
    speed: f32,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    slices: &MovementSlices,
) {
    unsafe {
        *slices.lane_id.get_mut(i) = conn_lane_id;
        *slices.lane_d.get_mut(i) = 0.0;
        *slices.transit.get_mut(i) = TRANSIT_INTERSECTION;
        *slices.cur_e.get_mut(i) = usize::MAX;
        apply_connector_entry_speed(
            i,
            conn_lane_id,
            speed,
            remaining_dist,
            transit_network,
            slices,
        );
        clamp_remaining_to_connector_sample(conn_lane_id, remaining_dist, transit_network);
    }
}

#[inline(always)]
fn connector_choice_seed(
    agent_idx: usize,
    from_lane_id: usize,
    target_id: usize,
    node_id: u32,
    path_idx: usize,
) -> u64 {
    (agent_idx as u64)
        ^ ((from_lane_id as u64) << 11)
        ^ ((target_id as u64) << 23)
        ^ ((node_id as u64) << 37)
        ^ ((path_idx as u64) << 3)
}

unsafe fn apply_connector_entry_speed(
    i: usize,
    conn_lane_id: usize,
    speed: f32,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    slices: &MovementSlices,
) {
    unsafe {
        if *slices.tmode.get(i) == MODE_CAR {
            let turn_speed = transit_network
                .lane_system
                .lanes
                .get(conn_lane_id)
                .map(|conn_lane| junction_entry_speed(*slices.speed.get(i), conn_lane))
                .unwrap_or_else(|| junction_car_speed(*slices.speed.get(i)));
            let time_left = if speed > 1.0e-5 {
                *remaining_dist / speed
            } else {
                0.0
            };
            *remaining_dist = turn_speed * time_left;
            *slices.speed.get_mut(i) = turn_speed;
        }
    }
}

fn clamp_remaining_to_connector_sample(
    conn_lane_id: usize,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
) {
    let Some(conn_lane) = transit_network.lane_system.lanes.get(conn_lane_id) else {
        *remaining_dist = 0.0;
        return;
    };

    let max_connector_step = (conn_lane.length - CONNECTOR_ENTRY_RETAIN_EPS_M).max(0.0);
    *remaining_dist = remaining_dist.min(max_connector_step);
}
