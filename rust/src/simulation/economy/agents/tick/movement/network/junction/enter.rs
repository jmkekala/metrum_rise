// SPDX-License-Identifier: GPL-2.0-only

//! Connector entry, wait, and claim handling.

use super::super::super::super::super::MODE_CAR;
use super::super::super::super::claims::LaneClaimContext;
use super::super::super::super::lane_nav::{
    collect_connector_lanes_to_edge, collect_connector_lanes_to_lane, connector_choice_seed,
    direct_vehicle_lane_to_edge, direct_vehicle_lane_to_lane, sibling_lane_reaches_edge,
};
use super::super::super::super::slices::{MovementSlices, enter_lane};
use super::super::super::super::traffic::{
    ConnectorEntry, claim_connector_entry, claim_lane_entry, junction_car_speed,
    junction_entry_speed, lane_entry_slot_clear,
};
use super::super::super::super::trip_failures::{FailureCause, FailureOutcome, record_failure};
use super::{LaneEndAction, WalkerStranded};
use crate::simulation::network::TransitNetwork;
use crate::traffic_log;
use std::cell::RefCell;

thread_local! {
    static VALID_CONNS: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
}

const CONNECTOR_ENTRY_RETAIN_EPS_M: f32 = 0.05;

/// Enters the connector from road lane `lane_id` onto `to_edge` at `node_id`: into `walk_lane`,
/// the sidewalk a walker's route witness names, or any connector for a car. A walker with no
/// such connector is handed back to the lane end (`WalkerStranded`).
#[allow(clippy::too_many_arguments)]
// Compiled in its caller's codegen unit: its inlining does not follow the crate's partition
// (see `handle_network_movement`).
#[inline]
pub(super) unsafe fn enter_next_edge_connector(
    i: usize,
    lane_id: usize,
    from_edge: usize,
    to_edge: usize,
    walk_lane: Option<usize>,
    node_id: u32,
    path_idx_before_lane_end: usize,
    path_len: usize,
    speed: f32,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) -> Result<LaneEndAction, WalkerStranded> {
    unsafe {
        if let Some(action) = try_enter_edge(
            i,
            lane_id,
            from_edge,
            to_edge,
            walk_lane,
            node_id,
            path_idx_before_lane_end,
            path_len,
            speed,
            remaining_dist,
            transit_network,
            lane_buckets,
            lane_claims,
            slices,
        ) {
            return Ok(action);
        }
        if let Some(action) = follow_missed_turn(
            i,
            lane_id,
            from_edge,
            to_edge,
            node_id,
            path_idx_before_lane_end,
            path_len,
            speed,
            remaining_dist,
            transit_network,
            lane_buckets,
            lane_claims,
            slices,
        ) {
            return Ok(action);
        }
        traffic_log!(
            "[JUNCTION_MISSING_CONN] agent={} node={} from_lane={} from_edge={} to_edge={} path_idx={}/{} reason=no-connection-lane",
            i,
            node_id,
            lane_id,
            from_edge,
            to_edge,
            *slices.path_idx.get(i),
            path_len,
        );
        let cause = FailureCause::NoTurnToEdge {
            node: node_id,
            from_lane: lane_id,
            from_edge,
            to_edge,
        };
        if *slices.tmode.get(i) != MODE_CAR {
            return Err(WalkerStranded(cause));
        }
        record_failure(i, cause, FailureOutcome::Replans, slices);
        slices.path.get_mut(i).clear();
        *slices.lane_id.get_mut(i) = usize::MAX;
        Ok(LaneEndAction::Break)
    }
}

// Enters `to_edge` from road lane `lane_id` straight on or through a connector (into `walk_lane`
// for a walker), or waits for the entry at the lane end. `None` when this lane has no way onto
// `to_edge` at all. Not recursive, so the lane-end handler stays one inlinable call tree.
#[allow(clippy::too_many_arguments)]
unsafe fn try_enter_edge(
    i: usize,
    lane_id: usize,
    from_edge: usize,
    to_edge: usize,
    walk_lane: Option<usize>,
    node_id: u32,
    path_idx_before_lane_end: usize,
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
        if let Some(direct_lane_id) = direct_vehicle_lane_to_edge(lane_id, to_edge, transit_network)
        {
            return Some(enter_direct_vehicle_lane(
                i,
                lane_id,
                direct_lane_id,
                from_edge,
                to_edge,
                node_id,
                path_idx_before_lane_end,
                path_len,
                remaining_dist,
                transit_network,
                lane_buckets,
                lane_claims,
                slices,
            ));
        }

        let connector_entry = match walk_lane {
            Some(target_lane_id) => claim_connector_to_lane(
                i,
                lane_id,
                target_lane_id,
                connector_choice_seed(i, lane_id, target_lane_id, node_id, *s_path_idx.get(i)),
                transit_network,
                lane_buckets,
                lane_claims,
            ),
            None => claim_connector_to_edge(
                i,
                lane_id,
                to_edge,
                connector_choice_seed(i, lane_id, to_edge, node_id, *s_path_idx.get(i)),
                transit_network,
                lane_buckets,
                lane_claims,
            ),
        };

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
                if crate::debug::Channel::Traffic.enabled() {
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
                Some(LaneEndAction::KeepMoving)
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
                Some(LaneEndAction::Break)
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
                Some(LaneEndAction::Break)
            }
            ConnectorEntry::MissingConnection => None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
// Compiled in its caller's codegen unit: its inlining does not follow the crate's partition
// (see `handle_network_movement`).
#[inline]
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
) -> Option<Result<LaneEndAction, WalkerStranded>> {
    unsafe {
        let s_path_idx = &slices.path_idx;
        let s_lane_d = &slices.lane_d;
        let s_speed = &slices.speed;
        let car = *slices.tmode.get(i) == MODE_CAR;
        let detach_edge = target_edge_for_lane(detach_lane_id, transit_network);
        // A car whose lane does not run into the detach lane itself enters the detach road on
        // the lane it does run into; the frontage change then reaches the detach lane.
        let direct = if !car {
            None
        } else if direct_vehicle_lane_to_lane(lane_id, detach_lane_id, transit_network) {
            Some(detach_lane_id)
        } else {
            direct_vehicle_lane_to_edge(lane_id, detach_edge, transit_network)
        };
        if let Some(direct) = direct {
            return Some(Ok(enter_direct_vehicle_lane(
                i,
                lane_id,
                direct,
                from_edge,
                detach_edge,
                node_id,
                path_idx.min(path_len),
                path_len,
                remaining_dist,
                transit_network,
                lane_buckets,
                lane_claims,
                slices,
            )));
        }

        let mut connector_entry = claim_connector_to_lane(
            i,
            lane_id,
            detach_lane_id,
            connector_choice_seed(i, lane_id, detach_lane_id, node_id, *s_path_idx.get(i)),
            transit_network,
            lane_buckets,
            lane_claims,
        );
        // Under the default lane use a connector may reach the detach road on another lane only.
        if car && matches!(connector_entry, ConnectorEntry::MissingConnection) {
            connector_entry = claim_connector_to_edge(
                i,
                lane_id,
                detach_edge,
                connector_choice_seed(i, lane_id, detach_edge, node_id, *s_path_idx.get(i)),
                transit_network,
                lane_buckets,
                lane_claims,
            );
        }

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
                if crate::debug::Channel::Traffic.enabled() {
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
                Some(Ok(LaneEndAction::Continue))
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
                Some(Ok(LaneEndAction::Break))
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
                Some(Ok(LaneEndAction::Break))
            }
            ConnectorEntry::MissingConnection => {
                if let Some(action) = follow_missed_turn(
                    i,
                    lane_id,
                    from_edge,
                    detach_edge,
                    node_id,
                    path_idx.min(path_len),
                    path_len,
                    speed,
                    remaining_dist,
                    transit_network,
                    lane_buckets,
                    lane_claims,
                    slices,
                ) {
                    return Some(Ok(action));
                }
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
                let cause = FailureCause::NoTurnToLane {
                    node: node_id,
                    from_lane: lane_id,
                    to_lane: detach_lane_id,
                };
                if *slices.tmode.get(i) != MODE_CAR {
                    return Some(Err(WalkerStranded(cause)));
                }
                record_failure(i, cause, FailureOutcome::Replans, slices);
                None
            }
        }
    }
}

// A car at the end of road lane `lane_id` with no way onto `to_edge` while another lane of its
// carriageway has one missed a lane-use turn: it could not reach a lane that makes the turn
// before the junction. It takes this lane's first successor in stable order whose road is not
// `to_edge` (a real connector or pass-through lane, so it never leaves the lane graph) and drops
// its node path; the lane-end handler then plans on that road from its end
// (`replan_after_missed_turn`). A wait for that connector keeps the car and its path at the lane
// end. `None` when no sibling makes the turn (a missing road turn, not a lane-use miss) or the lane
// has no other way on: the caller's failure path runs. O(L x C + C) lookups, no allocation, no
// route query.
#[allow(clippy::too_many_arguments)]
unsafe fn follow_missed_turn(
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
) -> Option<LaneEndAction> {
    unsafe {
        if *slices.tmode.get(i) != MODE_CAR
            || !sibling_lane_reaches_edge(lane_id, to_edge, transit_network)
        {
            return None;
        }
        let lanes = &transit_network.lane_system.lanes;
        let onward_edge = lanes[lane_id].next_lanes.iter().find_map(|&next| {
            let lane = &lanes[next];
            let road = if lane.edge_id == usize::MAX {
                *lane.next_lanes.first()?
            } else {
                next
            };
            let edge = lanes[road].edge_id;
            (edge != to_edge && edge != usize::MAX).then_some(edge)
        })?;
        // The onward road is one this lane reaches, so the entry finds its connector or
        // pass-through lane (or waits for it).
        let action = try_enter_edge(
            i,
            lane_id,
            from_edge,
            onward_edge,
            None,
            node_id,
            path_idx_before_lane_end,
            path_len,
            speed,
            remaining_dist,
            transit_network,
            lane_buckets,
            lane_claims,
            slices,
        )?;
        if *slices.lane_id.get(i) != lane_id {
            slices.path.get_mut(i).clear();
            *slices.path_idx.get_mut(i) = 0;
            traffic_log!(
                "[MISSED_TURN] agent={} node={} lane={} wanted_edge={} onward_edge={}",
                i,
                node_id,
                lane_id,
                to_edge,
                onward_edge
            );
        }
        Some(action)
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn enter_direct_vehicle_lane(
    i: usize,
    lane_id: usize,
    target_lane_id: usize,
    from_edge: usize,
    target_edge: usize,
    node_id: u32,
    path_idx_before_lane_end: usize,
    path_len: usize,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) -> LaneEndAction {
    unsafe {
        if !lane_entry_slot_clear(target_lane_id, lane_buckets) {
            *slices.path_idx.get_mut(i) = path_idx_before_lane_end;
            *slices.lane_d.get_mut(i) = transit_network.lane_system.lanes[lane_id].length;
            traffic_log!(
                "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_lane={} to_edge={} path_idx={}/{} reason=direct-continuation-occupied",
                i,
                node_id,
                lane_id,
                from_edge,
                target_lane_id,
                target_edge,
                *slices.path_idx.get(i),
                path_len,
            );
            return LaneEndAction::Break;
        }

        if !claim_lane_entry(i, target_lane_id, lane_claims) {
            *slices.path_idx.get_mut(i) = path_idx_before_lane_end;
            *slices.lane_d.get_mut(i) = transit_network.lane_system.lanes[lane_id].length;
            traffic_log!(
                "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_lane={} to_edge={} path_idx={}/{} reason=direct-continuation-claimed",
                i,
                node_id,
                lane_id,
                from_edge,
                target_lane_id,
                target_edge,
                *slices.path_idx.get(i),
                path_len,
            );
            return LaneEndAction::Break;
        }

        enter_lane!(slices, i, transit_network, target_lane_id, 0.0, node_id);
        traffic_log!(
            "[JUNCTION_BYPASS] agent={} node={} from_lane={} from_edge={} to_lane={} to_edge={} remaining_dist={:.2} path_idx={}/{} reason=direct-pass-through",
            i,
            node_id,
            lane_id,
            from_edge,
            target_lane_id,
            target_edge,
            *remaining_dist,
            *slices.path_idx.get(i),
            path_len,
        );
        LaneEndAction::KeepMoving
    }
}

fn target_edge_for_lane(lane_id: usize, transit_network: &TransitNetwork) -> usize {
    transit_network
        .lane_system
        .lanes
        .get(lane_id)
        .map(|lane| lane.edge_id)
        .unwrap_or(usize::MAX)
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
        enter_lane!(
            slices,
            i,
            transit_network,
            conn_lane_id,
            0.0,
            *slices.cur_n.get(i)
        );
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
