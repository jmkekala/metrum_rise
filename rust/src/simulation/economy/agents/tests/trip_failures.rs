// SPDX-License-Identifier: GPL-2.0-only

//! Trip failure lines on `events` (`DEBUG-06`), from failures in the movement tick.

use super::support::*;
use super::*;

// Nodes of `build_4way_junction`: centre, then the W and E arm ends.
const CENTER: u32 = 0;
const WEST: u32 = 1;
const EAST: u32 = 2;

fn car_on_lane(agents: &mut AgentSystem, home: usize, lane_id: usize, lane_d: f32) -> usize {
    let idx = agents.spawn_border_arrival_agent(home, WEST, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.target_building[idx] = usize::MAX;
    agents.current_edge[idx] = 0;
    agents.current_lane_id[idx] = lane_id;
    agents.lane_distance[idx] = lane_d;
    agents.speed[idx] = 10.0;
    agents.current_path_index[idx] = 1;
    agents.overtake_cooldown_s[idx] = f32::MAX;
    idx
}

fn tick_for(
    agents: &mut AgentSystem,
    allocator: &BuildingAllocator,
    network: &mut TransitNetwork,
    graph: &mut RegionGraph,
    seconds: f32,
) {
    for _ in 0..(seconds / 0.1).round() as usize {
        agents.tick(allocator, network, graph, 0.1, &test_clock(0, 0));
    }
}

#[test]
fn a_missing_turn_and_a_missing_road_name_the_agent_and_the_junction() {
    let allocator = BuildingAllocator::new();
    let (mut network, mut graph, arms) = build_4way_junction(1, 1);
    let lane_id = arms[0][0];
    let to_edge = graph.get_edge_between_nodes(CENTER, EAST).unwrap();
    // Cut every turn from the west lane onto the east arm.
    let lanes = &mut network.lane_system.lanes;
    let kept: Vec<usize> = lanes[lane_id]
        .next_lanes
        .iter()
        .copied()
        .filter(|&connector| {
            lanes[connector].edge_id != usize::MAX
                || lanes[lanes[connector].next_lanes[0]].edge_id != to_edge
        })
        .collect();
    assert!(kept.len() < lanes[lane_id].next_lanes.len());
    lanes[lane_id].next_lanes.clear();
    for connector in kept {
        lanes[lane_id].next_lanes.push(connector);
    }
    let lane_d = lanes[lane_id].length - 1.0;
    let mut agents = AgentSystem::new();
    agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
    let idx = car_on_lane(&mut agents, usize::MAX, lane_id, lane_d);
    agents.current_path[idx] = vec![WEST, CENTER, EAST];
    let render_id = agents.render_id[idx];

    tick_for(&mut agents, &allocator, &mut network, &mut graph, 1.0);
    let lines = &agents.trip_failures.lines;
    assert_eq!(lines.len(), 1, "{lines:?}");
    let prefix = format!("trip failed: agent {render_id} [row {idx}], resident, car, ");
    assert!(lines[0].starts_with(&prefix), "{}", lines[0]);
    assert!(
        lines[0].ends_with(&format!(
            ", with no destination: no turn from lane {lane_id} (edge 0) onto edge {to_edge} at \
             node {CENTER}; drops its route and replans"
        )),
        "{}",
        lines[0]
    );

    // The same car on a route whose next node has no road from the junction.
    let (mut network, mut graph, arms) = build_4way_junction(1, 1);
    let lane_id = arms[0][0];
    let lane_d = network.lane_system.lanes[lane_id].length - 1.0;
    let mut agents = AgentSystem::new();
    let idx = car_on_lane(&mut agents, usize::MAX, lane_id, lane_d);
    agents.current_path[idx] = vec![WEST, CENTER, 99];
    tick_for(&mut agents, &allocator, &mut network, &mut graph, 1.0);
    let lines = &agents.trip_failures.lines;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].ends_with(&format!(
            ": no road from node {CENTER} to node 99, the next node on its route; drops its \
             route and replans"
        )),
        "{}",
        lines[0]
    );
}

#[test]
fn a_stale_exit_logs_its_first_failure_and_the_recovery_home() {
    let (mut network, mut graph, fwd_lanes) = build_two_edge_road(1, 1);
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(create_test_building(1, 1));
    let door = Vector2::new(150.0, 12.0);
    // An entrance with no lanes: the planned exit is no longer legal and no new one exists.
    allocator.entrances.push(BuildingEntrance {
        edge_idx: 1,
        side: 1,
        entrance_s_m: 40.0,
        door_pos: door,
        curb_pos: Vector2::new(150.0, 5.0),
        ..BuildingEntrance::default()
    });
    let detach_lane = fwd_vehicle_lanes(&network, 1)[0];
    let mut agents = AgentSystem::new();
    let idx = car_on_lane(&mut agents, 0, fwd_lanes[0], 30.0);
    agents.current_node[idx] = 0;
    agents.target_building[idx] = 0;
    agents.current_path[idx] = vec![0, 1];
    agents.access_flags[idx] = ACCESS_PLAN_VALID;
    agents.planned_detach_node[idx] = 1;
    agents.planned_detach_lane_id[idx] = detach_lane as u32;
    agents.planned_detach_lane_d[idx] = 40.0;
    let render_id = agents.render_id[idx];

    // Failures at 0.1 s, 5.1 s and 10.1 s; the second repeats the first and is not logged.
    tick_for(&mut agents, &allocator, &mut network, &mut graph, 11.0);
    let lines = &agents.trip_failures.lines;
    let at = format!(
        "trip failed: agent {render_id} [row {idx}], resident, car, at node 0, lane {} \
         on edge 0 at 30.0 m, to building 0 (residential): planned exit from the road no longer \
         legal (building 0, lane {detach_lane} at 40.0 m), then no route from node 1 to building \
         0: no lane by the door from node 1; no lane by the door from node 2; ",
        fwd_lanes[0]
    );
    assert_eq!(
        lines,
        &vec![
            format!("{at}retry in 5 s (failure 1 of 3)"),
            format!(
                "{at}sent home to building 0 at ({:.1}, {:.1}) after 3 failures",
                door.x, door.y
            ),
        ]
    );
    assert_eq!(agents.transit[idx], TRANSIT_IN_BUILDING);
}

#[test]
fn departure_failures_log_in_agent_order_and_fold_past_the_place_limit() {
    let run = || {
        let (mut network, mut graph, _) = build_two_edge_road(1, 1);
        let mut allocator = BuildingAllocator::new();
        for edge in [0, 1] {
            allocator.buildings.push(create_test_building(edge, 1));
            allocator.entrances.push(BuildingEntrance {
                edge_idx: edge,
                ..BuildingEntrance::default()
            });
        }
        let mut agents = AgentSystem::new();
        // More than `PAR_THRESHOLD`, so the movement pass runs on the Rayon pool.
        for _ in 0..600 {
            let idx = agents.spawn_housed_agent(0, 0.0, 0.0);
            agents.current_building[idx] = 0;
            agents.planned_target_building[idx] = 1;
        }
        tick_for(&mut agents, &allocator, &mut network, &mut graph, 61.0);
        agents.trip_failures.lines.clone()
    };
    let lines = run();
    assert_eq!(
        lines,
        run(),
        "the log must not depend on the worker schedule"
    );
    // Twenty lines for the first twenty agents, then one summary when the minute ends. The
    // retries at 30 s repeat the first failures and are not logged.
    assert_eq!(lines.len(), 21, "{lines:#?}");
    for (row, line) in lines[..20].iter().enumerate() {
        assert_eq!(
            line,
            &format!(
                "trip failed: agent {row} [row {row}], resident, in building 0, to building 1 \
                 (residential): no route from building 0 to building 1 by car or on foot; tries \
                 to leave again in 30 s"
            )
        );
    }
    assert_eq!(
        lines[20],
        "trip failures: 580 more in the last simulated minute: no route at departure, 580 at \
         building 0 (first agent 20)"
    );
}
