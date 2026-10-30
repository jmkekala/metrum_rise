// SPDX-License-Identifier: GPL-2.0-only

//! `TRAFFIC-07` audit: runs a prepared snapshot's day and counts, every few game hours, the
//! lanes holding more cars than fit and the cars stacked at one point of a lane, plus the cars
//! waiting at a node for their first lane and at an attach point for a gap (`TRAFFIC-11`). A
//! diagnostic of lane entry, not acceptance timing.

use super::{FRAME_DT_S, MINUTES_PER_DAY, absolute_minute};
use super::{cache_dir, residents, snapshot, timing_mode};
use crate::config::{CAR_LENGTH, IDM_S_MIN};
use crate::nodes::sim::core::state::SimCore;
use crate::simulation::economy::agents::{
    ACCESS_LANE_ATTACH_PENDING, MODE_WALK, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};

// Cars closer than this to a lane's start count as one stack.
const STACK_RADIUS_M: f32 = 0.5;

// Counts vehicles by lane from the agent columns. O(agents + lanes), serial: a test diagnostic
// run a few times per game day.
fn audit(core: &SimCore) -> serde_json::Value {
    let agents = &core.agents.agents;
    let lanes = &core.transit_network.lane_system.lanes;
    let mut count = vec![0_u32; lanes.len()];
    let mut at_start = vec![0_u32; lanes.len()];
    let (mut held, mut held_freight, mut held_at_attach) = (0_u32, 0_u32, 0_u32);
    let mut attach_lanes = Vec::new();
    for i in 0..agents.len() {
        let on_road =
            agents.transit[i] == TRANSIT_NETWORK || agents.transit[i] == TRANSIT_INTERSECTION;
        if !on_road || agents.transit_mode[i] == MODE_WALK {
            continue;
        }
        let lane = agents.current_lane_id[i];
        if lane >= lanes.len() {
            if lane == usize::MAX && (agents.access_flags[i] & ACCESS_LANE_ATTACH_PENDING) != 0 {
                held_at_attach += 1;
                attach_lanes.push(agents.planned_attach_lane_id[i] as usize);
            } else if lane == usize::MAX
                && agents.transit[i] == TRANSIT_NETWORK
                && !agents.current_path[i].is_empty()
            {
                held += 1;
                held_freight += u32::from(agents.freight_shipment_id[i] != 0);
            }
            continue;
        }
        count[lane] += 1;
        at_start[lane] += u32::from(agents.lane_distance[i] < STACK_RADIUS_M);
    }
    let spacing = CAR_LENGTH + IDM_S_MIN;
    let (mut over_fit, mut over_slots, mut worst_ratio, mut worst_lane) =
        (0_u32, 0_u32, 0.0_f32, 0);
    let (mut stacked_lanes, mut stacked_cars, mut max_stack) = (0_u32, 0_u32, 0_u32);
    for (lane, &cars) in count.iter().enumerate() {
        // A lane of length L holds floor(L / spacing) cars by length, one more at both ends.
        let fit = ((lanes[lane].length / spacing).floor() as u32).max(1);
        over_fit += u32::from(cars > fit);
        over_slots += u32::from(cars > fit + 1);
        let ratio = cars as f32 / fit as f32;
        if ratio > worst_ratio {
            (worst_ratio, worst_lane) = (ratio, lane);
        }
        if at_start[lane] >= 2 {
            stacked_lanes += 1;
            stacked_cars += at_start[lane];
            max_stack = max_stack.max(at_start[lane]);
        }
    }
    // The lanes cars wait to attach to, and how many of them hold at least as many cars as fit.
    attach_lanes.sort_unstable();
    attach_lanes.dedup();
    let attach_lanes_full = attach_lanes
        .iter()
        .filter(|&&lane| {
            lane < lanes.len()
                && count[lane] >= ((lanes[lane].length / spacing).floor() as u32).max(1)
        })
        .count();
    // The lanes the audit flags, with who is on them (diagnostic detail).
    let mut flagged = Vec::new();
    for (lane, &cars) in count.iter().enumerate() {
        let fit = ((lanes[lane].length / spacing).floor() as u32).max(1);
        if cars <= fit + 1 && at_start[lane] < 2 {
            continue;
        }
        let mut kinds = std::collections::BTreeMap::<String, u32>::new();
        let mut sample = Vec::new();
        for i in 0..agents.len() {
            if agents.current_lane_id[i] != lane
                || agents.transit_mode[i] == MODE_WALK
                || !(agents.transit[i] == TRANSIT_NETWORK
                    || agents.transit[i] == TRANSIT_INTERSECTION)
            {
                continue;
            }
            let near_start = agents.lane_distance[i] < STACK_RADIUS_M;
            let key = format!(
                "transit={} mode={} freight={} path_left={} start={} detach_lane_here={}",
                agents.transit[i],
                agents.transit_mode[i],
                agents.freight_shipment_id[i] != 0,
                agents.current_path[i]
                    .len()
                    .saturating_sub(agents.current_path_index[i]),
                near_start,
                agents.planned_detach_lane_id[i] as usize == lane,
            );
            *kinds.entry(key).or_default() += 1;
            if near_start && sample.len() < 3 {
                sample.push(serde_json::json!({
                    "agent": i, "node": agents.current_node[i], "edge": agents.current_edge[i],
                    "d": agents.lane_distance[i], "speed": agents.speed[i],
                    "lane_change_from": agents.lane_change_from_lane_id[i],
                    "building": agents.current_building[i], "target": agents.target_building[i],
                    "access": agents.access_flags[i],
                }));
            }
        }
        let l = &lanes[lane];
        flagged.push(serde_json::json!({
            "lane": lane, "edge": l.edge_id, "node": l.node_id, "fwd": l.is_fwd,
            "length": l.length, "cars": cars, "fit": fit, "at_start": at_start[lane],
            "kinds": kinds, "sample": sample,
        }));
        if flagged.len() >= 6 {
            break;
        }
    }
    serde_json::json!({
        "flagged": flagged,
        "vehicles_on_lanes": count.iter().sum::<u32>(),
        "lanes_over_fit": over_fit, "lanes_over_fit_plus_one": over_slots,
        "worst_lane_cars": count.get(worst_lane).copied().unwrap_or(0),
        "worst_lane_fit": lanes.get(worst_lane).map_or(0, |lane| (lane.length / spacing).floor() as u32),
        "stacked_lanes": stacked_lanes, "stacked_cars": stacked_cars, "max_stack": max_stack,
        "held_at_node": held, "held_freight": held_freight, "held_at_attach": held_at_attach,
        "attach_lanes": attach_lanes.len(), "attach_lanes_full": attach_lanes_full,
    })
}

// Diagnostic: reports agents that moved onto a long lane that was already full in the previous
// frame, with where they came from. Returns the number reported.
fn trace_entries(
    core: &SimCore,
    previous_lane: &mut Vec<usize>,
    previous_count: &mut Vec<u32>,
    previous_state: &mut Vec<(u8, usize, f32, u32, usize)>,
    previous_removed: &mut u64,
) -> u32 {
    let agents = &core.agents.agents;
    let lanes = &core.transit_network.lane_system.lanes;
    let spacing = CAR_LENGTH + IDM_S_MIN;
    let mut count = vec![0_u32; lanes.len()];
    let mut reported = 0;
    previous_lane.resize(agents.len(), usize::MAX);
    previous_state.resize(agents.len(), (0, usize::MAX, 0.0, u32::MAX, usize::MAX));
    let removed =
        crate::debug::work::snapshot()[crate::debug::work::Work::BuildingsRemoved as usize];
    previous_count.resize(lanes.len(), 0);
    for i in 0..agents.len() {
        let on_road =
            agents.transit[i] == TRANSIT_NETWORK || agents.transit[i] == TRANSIT_INTERSECTION;
        if on_road && agents.transit_mode[i] != MODE_WALK && agents.current_lane_id[i] < lanes.len()
        {
            count[agents.current_lane_id[i]] += 1;
        }
    }
    for i in 0..agents.len() {
        let lane = agents.current_lane_id[i];
        let on_road =
            agents.transit[i] == TRANSIT_NETWORK || agents.transit[i] == TRANSIT_INTERSECTION;
        if on_road
            && agents.transit_mode[i] != MODE_WALK
            && lane < lanes.len()
            && previous_lane[i] != lane
        {
            let fit = (lanes[lane].length / spacing).floor() as u32;
            if previous_count[lane] > fit {
                reported += 1;
                eprintln!(
                    "TRAFFIC07_ENTRY {}",
                    serde_json::json!({
                        "minute": absolute_minute(core), "buildings_removed_total": removed,
                        "buildings_removed_in_frame": removed - *previous_removed, "agent": i, "lane": lane, "fit": fit,
                        "count_before": previous_count[lane], "count_now": count[lane],
                        "from_lane": previous_lane[i], "d": agents.lane_distance[i],
                        "transit": agents.transit[i], "freight": agents.freight_shipment_id[i] != 0,
                        "access": agents.access_flags[i], "node": agents.current_node[i],
                        "lane_change_from": agents.lane_change_from_lane_id[i],
                        "path_left": agents.current_path[i].len().saturating_sub(agents.current_path_index[i]),
                        "detach_lane": agents.planned_detach_lane_id[i], "detach_d": agents.planned_detach_lane_d[i],
                        "lane_edge": lanes[lane].edge_id, "lane_node": lanes[lane].node_id,
                        "lane_len": lanes[lane].length,
                        "prev_transit": previous_state[i].0, "prev_raw_lane": previous_state[i].1,
                        "prev_d": previous_state[i].2, "prev_node": previous_state[i].3,
                        "prev_building": previous_state[i].4, "home": agents.home_building[i],
                        "target": agents.target_building[i], "speed": agents.speed[i],
                    })
                );
            }
        }
        previous_lane[i] = if on_road { lane } else { usize::MAX };
        previous_state[i] = (
            agents.transit[i],
            lane,
            agents.lane_distance[i],
            agents.current_node[i],
            agents.current_building[i],
        );
    }
    previous_count.copy_from_slice(&count);
    *previous_removed = removed;
    reported
}

#[test]
#[ignore = "TRAFFIC-07 lane-entry audit over one game day of a prepared snapshot; release"]
fn traffic07_lane_entry_audit() {
    timing_mode();
    let every_minutes: u64 = std::env::var("METRUM_AUDIT_EVERY_MINUTES")
        .map(|value| {
            value
                .parse()
                .expect("METRUM_AUDIT_EVERY_MINUTES must be an integer")
        })
        .unwrap_or(240);
    let speed: f32 = std::env::var("METRUM_PERF07_SPEED")
        .map(|value| value.parse().expect("METRUM_PERF07_SPEED must be a number"))
        .unwrap_or(32.0);
    let mut city = snapshot::load_snapshot(residents(), &cache_dir());
    let core = &mut city.core;
    core.time.speed_multiplier = speed;
    let start = absolute_minute(core);
    let end = start + MINUTES_PER_DAY;
    let mut next = start;
    let trace = std::env::var("METRUM_AUDIT_TRACE").is_ok();
    let mut previous_lane: Vec<usize> = Vec::new();
    let mut previous_count: Vec<u32> = Vec::new();
    let mut previous_state: Vec<(u8, usize, f32, u32, usize)> = Vec::new();
    let mut previous_removed = 0;
    let mut traced = 0;
    loop {
        if trace && traced < 400 {
            traced += trace_entries(
                core,
                &mut previous_lane,
                &mut previous_count,
                &mut previous_state,
                &mut previous_removed,
            );
        }
        let now = absolute_minute(core);
        if now >= next {
            eprintln!(
                "TRAFFIC07_AUDIT {}",
                serde_json::json!({
                    "residents": residents(), "minute": now - start, "audit": audit(core),
                })
            );
            next += every_minutes;
        }
        if now >= end {
            break;
        }
        core.step_frame(FRAME_DT_S);
    }
}
