// SPDX-License-Identifier: GPL-2.0-only

//! Watchdog probe (`TRAFFIC-19`): every trip-failure record and every replan-watchdog verdict of
//! a populated city's days after the build, by agent kind (walker, car, freight), lane state,
//! site and outcome. Ignored; run with `TRAFFIC19_RESIDENTS` and `TRAFFIC19_DAYS`.
//! `TRAFFIC19_TOGGLES=1` plays the player who turns crossings off around walkers: every 30 game
//! minutes, through the editor, a sixth of the four-way junctions lose every crossing, and get
//! them back 30 minutes later, the sixths in turn. `TRAFFIC19_BULLDOZE=1` plays the player who
//! removes roads under traffic: every 2 game hours one live road edge goes, through the bulldoze
//! tool's path, chosen by a fixed stride over the edge ids.

use super::super::frame::FRAME_DT_S;
use super::PopulatedCity;
use crate::simulation::economy::agents::tick::probe_tally;

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "probe: TRAFFIC19_RESIDENTS, TRAFFIC19_DAYS"]
fn watchdog_firings_by_kind_and_cause() {
    let residents = env_u32("TRAFFIC19_RESIDENTS", 10_000);
    let days = env_u32("TRAFFIC19_DAYS", 3);
    probe_tally::enable();
    let mut city = PopulatedCity::build(residents);
    probe_tally::take();
    let toggles = std::env::var_os("TRAFFIC19_TOGGLES").is_some();
    let bulldoze = std::env::var_os("TRAFFIC19_BULLDOZE").is_some();
    let mut bulldozed = 0;
    let start_day = city.core.time.day_index;
    let mut half_hour = u32::MAX;
    let mut toggled = 0;
    while city.core.time.day_index < start_day + days {
        city.core.step_frame(FRAME_DT_S);
        let now = city.core.time.day_index * 48 + u32::from(city.core.time.minute_of_day) / 30;
        if now != half_hour {
            half_hour = now;
            if toggles {
                toggled += toggle_crossings(&mut city, now);
            }
            if bulldoze && now.is_multiple_of(4) && bulldoze_road(&mut city, now / 4) {
                bulldozed += 1;
            }
        }
    }
    println!(
        "PROBE residents {} days {days} crossings {} toggles {toggled} bulldozed {bulldozed}",
        city.record().residents,
        if std::env::var_os("METRUM_R6_CROSSINGS_OFF").is_some() {
            "off"
        } else {
            "default"
        }
    );
    end_state(&mut city);
    let (tally, samples) = probe_tally::take();
    for ((section, kind, lane, site, outcome), count) in tally {
        println!("TALLY {count:>8} {section} | {kind} | {lane} | {site} | {outcome}");
    }
    for line in samples {
        println!("SAMPLE {line}");
    }
    // Walker records with their lines, for tracing.
    for line in city
        .core
        .agents
        .trip_failures
        .lines
        .iter()
        .filter(|line| line.contains("on foot"))
        .take(40)
    {
        println!("WALKLINE {line}");
    }
}

// The walkers' state at the end (round T19b): walkers with no lane stranded by their node (within
// a road edit's 30 m reattach reach of it) or off the network (beyond it, so beyond the 200 m walk
// bound of every sidewalk), with each off-network one's distance from its node; walkers still
// walking to a sidewalk; walkers waiting for a home.
fn end_state(city: &mut PopulatedCity) {
    use crate::simulation::economy::agents::{
        ACCESS_WALK_STRANDED, ACCESS_WALK_TO_SIDEWALK, MODE_WALK, TRANSIT_NETWORK,
    };
    let core = &mut city.core;
    core.agents
        .refresh_stranded_report(&core.transit_network, &core.region_graph);
    let agents = &core.agents;
    let (mut at_node, mut off, mut walking) = (0, Vec::new(), 0);
    for i in 0..agents.len() {
        if agents.transit_mode[i] != MODE_WALK
            || agents.transit[i] != TRANSIT_NETWORK
            || agents.current_lane_id[i] != usize::MAX
        {
            continue;
        }
        if agents.access_flags[i] & ACCESS_WALK_TO_SIDEWALK != 0 {
            walking += 1;
        }
        if agents.access_flags[i] & ACCESS_WALK_STRANDED == 0 {
            continue;
        }
        let node = agents.current_node[i];
        let metres = if (node as usize) < core.region_graph.node_count() {
            let at = core
                .region_graph
                .node(core.region_graph.get_valid_node(node))
                .pos;
            (at.x - agents.pos_x[i]).hypot(at.z - agents.pos_y[i])
        } else {
            f32::INFINITY
        };
        if metres <= 30.0 {
            at_node += 1;
        } else {
            off.push(format!("{i}@{metres:.0}m"));
        }
    }
    println!(
        "END walkers lane-less stranded at a node {at_node} off the network {} {off:?} walking to \
         a sidewalk {walking} waiting for a home {}",
        off.len(),
        agents.stranded_report().waiting_for_home
    );
}

// Half hour `k`: the four-way junctions `n` with `n % 6 == (k / 2) % 6` lose every crossing on
// even `k` and get them back on odd `k`. Returns the arms toggled.
fn toggle_crossings(city: &mut PopulatedCity, k: u32) -> u32 {
    let (phase, enabled) = ((k / 2) % 6, k % 2 == 1);
    let core = &mut city.core;
    let mut toggled = 0;
    for node in 0..core.region_graph.node_count() as u32 {
        if core.region_graph.get_valid_node(node) != node || node % 6 != phase {
            continue;
        }
        let arms: Vec<usize> = core
            .region_graph
            .node_adjacency(node)
            .iter()
            .copied()
            .filter(|&edge| !core.region_graph.edge(edge).deleted)
            .collect();
        if arms.len() != 4 {
            continue;
        }
        for edge in arms {
            if core.set_crosswalk_override_internal(node, edge as i32, enabled) {
                toggled += 1;
            }
        }
    }
    toggled
}

// One agent row's movement state, printed whenever it changes, until a game time: the history
// of a probe sample (`TRAFFIC19_TRACE_AGENT`, `TRAFFIC19_TRACE_UNTIL` in simulated seconds).
#[test]
#[ignore = "probe: TRAFFIC19_RESIDENTS, TRAFFIC19_TRACE_AGENT, TRAFFIC19_TRACE_UNTIL"]
fn trace_agent_row() {
    let residents = env_u32("TRAFFIC19_RESIDENTS", 10_000);
    let row = env_u32("TRAFFIC19_TRACE_AGENT", 0) as usize;
    let until = env_u32("TRAFFIC19_TRACE_UNTIL", 0) as f32;
    let mut city = PopulatedCity::build(residents);
    let mut last = String::new();
    while city.core.agents.sim_time <= until {
        city.core.step_frame(FRAME_DT_S);
        let a = &city.core.agents.agents;
        if row >= a.len() {
            continue;
        }
        let state = format!(
            "render {} transit {} mode {} activity {} planned_activity {} building {} target {} \
             home {} node {} edge {} lane {} path {:?} idx {} flags {:#x} detach {} {} attach {} \
             next_replan {:.1} failures {}",
            a.render_id[row],
            a.transit[row],
            a.transit_mode[row],
            a.activity[row],
            a.planned_activity[row],
            a.current_building[row],
            a.target_building[row],
            a.home_building[row],
            a.current_node[row],
            a.current_edge[row],
            a.current_lane_id[row],
            a.current_path[row],
            a.current_path_index[row],
            a.access_flags[row],
            a.planned_detach_node[row],
            a.planned_detach_lane_id[row],
            a.planned_attach_node[row],
            a.next_replan_time[row],
            a.network_replan_failures[row],
        );
        if state != last {
            println!("TRACE t {:.1} {state}", city.core.agents.sim_time);
            last = state;
        }
    }
}

// Edit `k`: bulldozes the first live edge at or after `k * 7919` modulo the edge count. Returns
// whether one went.
fn bulldoze_road(city: &mut PopulatedCity, k: u32) -> bool {
    let core = &mut city.core;
    let edges = core.region_graph.edge_count();
    (0..edges)
        .map(|offset| (k as usize * 7919 + offset) % edges)
        .find(|&edge| !core.region_graph.edge(edge).deleted)
        .is_some_and(|edge| core.bulldoze_road_edge_for_test(edge))
}
