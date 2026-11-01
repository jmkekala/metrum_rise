// SPDX-License-Identifier: GPL-2.0-only

//! Walkers follow their route's lane witness at junctions (`ROAD-74`).

use super::support::*;
use super::*;
use crate::config::AGENT_WALK_SPEED_MS;
use crate::simulation::network::lanes::LaneType;

// The foot lanes of `edge` leaving `node`, by lane index.
fn foot_lanes_from(
    network: &TransitNetwork,
    graph: &RegionGraph,
    edge: usize,
    node: u32,
) -> Vec<usize> {
    let start = graph.edge(edge).start_node;
    let mut lanes: Vec<usize> = network.lane_system.edge_lanes[&edge]
        .iter()
        .copied()
        .filter(|&lane| {
            let value = &network.lane_system.lanes[lane];
            value.lane_type == LaneType::Foot && value.is_fwd == (start == node)
        })
        .collect();
    lanes.sort_by_key(|&lane| network.lane_system.lanes[lane].lane_idx);
    lanes
}

// The foot connectors from road lane `from` into road lane `to`, shortest first.
fn connectors(network: &TransitNetwork, from: usize, to: usize) -> Vec<f32> {
    let lanes = &network.lane_system.lanes;
    let mut found: Vec<f32> = lanes[from]
        .next_lanes
        .iter()
        .map(|&conn| &lanes[conn])
        .filter(|conn| conn.edge_id == usize::MAX && conn.next_lanes.first() == Some(&to))
        .map(|conn| conn.length)
        .collect();
    found.sort_by(f32::total_cmp);
    found
}

fn turn_crossings(graph: &mut RegionGraph, network: &mut TransitNetwork, node: u32, on: bool) {
    for edge_id in 0..4 {
        graph.set_crosswalk_override(node, edge_id, on);
    }
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(graph);
    network.cch_graph = CchGraph::build(graph);
}

/// The traced case (round R6b): with the crossings of the junction off, only one sidewalk of the
/// road into it reaches the walker's exit lane. A walker at the far end of that road took the
/// shortest connector onto it, kept its sidewalk and stood at the junction for ever. Following
/// its route's witness it takes the longer connector across the road end and reaches the exit.
struct Traced {
    network: TransitNetwork,
    graph: RegionGraph,
    allocator: BuildingAllocator,
    start: usize,
    exit: usize,
}

fn traced_case() -> Traced {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    let (center, west) = (0_u32, 1_u32);
    let (west_edge, north_edge) = (0_usize, 2_usize);
    turn_crossings(&mut graph, &mut network, center, false);
    let into_center = foot_lanes_from(&network, &graph, west_edge, west);
    let exits = foot_lanes_from(&network, &graph, north_edge, center);
    // The north exit lane that exactly one west sidewalk reaches across the crossing-less node.
    let (exit, good) = exits
        .iter()
        .find_map(|&exit| {
            let reach: Vec<usize> = into_center
                .iter()
                .copied()
                .filter(|&lane| !connectors(&network, lane, exit).is_empty())
                .collect();
            (reach.len() == 1).then(|| (exit, reach[0]))
        })
        .expect("a crossing-less corner joins one sidewalk to one exit");
    let bad = into_center
        .iter()
        .copied()
        .find(|&lane| lane != good)
        .unwrap();
    // The walker arrives at the west road end on the sidewalk the shortest connector keeps: the
    // one that cannot reach the exit (the movement rule before `ROAD-74` took it).
    let bad_idx = network.lane_system.lanes[bad].lane_idx;
    let start = foot_lanes_from(&network, &graph, west_edge, center)
        .into_iter()
        .find(|&lane| network.lane_system.lanes[lane].lane_idx == bad_idx)
        .unwrap();
    let (keep, cross) = (
        connectors(&network, start, bad),
        connectors(&network, start, good),
    );
    assert!(
        keep[0] < cross[0],
        "the shortest connector keeps the sidewalk: {keep:?} against {cross:?}"
    );
    // The walker's destination: a building on the north road whose door the exit lane serves.
    let exit_idx = network.lane_system.lanes[exit].lane_idx;
    let mut allocator = BuildingAllocator::new();
    allocator
        .buildings
        .push(create_test_building(north_edge, exit_idx.signum()));
    allocator.entrances.push(BuildingEntrance {
        edge_idx: north_edge,
        side: exit_idx.signum(),
        entrance_s_m: 50.0,
        foot_lane_fwd: foot_lanes_from(&network, &graph, north_edge, 3)
            .into_iter()
            .find(|&lane| network.lane_system.lanes[lane].lane_idx == exit_idx)
            .unwrap(),
        foot_lane_bkw: exit,
        ..BuildingEntrance::default()
    });
    Traced {
        network,
        graph,
        allocator,
        start,
        exit,
    }
}

fn walk_traced(case: &mut Traced, agents: &mut AgentSystem, walkers: usize) -> Vec<usize> {
    let (center, west) = (0_u32, 1_u32);
    let start_len = case.network.lane_system.lanes[case.start].length;
    let mut ids = Vec::new();
    for k in 0..walkers {
        let idx = agents.spawn_border_arrival_agent(usize::MAX, west, 0.0, 0.0);
        agents.transit[idx] = TRANSIT_NETWORK;
        agents.transit_mode[idx] = MODE_WALK;
        agents.speed[idx] = AGENT_WALK_SPEED_MS;
        agents.current_node[idx] = west;
        agents.current_edge[idx] = 0;
        agents.current_lane_id[idx] = case.start;
        agents.lane_distance[idx] = (start_len - 1.0 - 3.0 * k as f32).max(0.0);
        agents.current_path[idx] = vec![west, center];
        agents.current_path_index[idx] = 1;
        agents.access_flags[idx] = ACCESS_PLAN_VALID;
        agents.planned_detach_node[idx] = center;
        agents.planned_detach_lane_id[idx] = case.exit as u32;
        agents.planned_detach_lane_d[idx] = 30.0;
        agents.target_building[idx] = 0;
        ids.push(idx);
    }
    ids
}

// Whether the walker is in the connector into the exit lane or on it.
fn reached_exit(agents: &AgentSystem, network: &TransitNetwork, idx: usize, exit: usize) -> bool {
    let lane = agents.current_lane_id[idx];
    lane == exit
        || network.lane_system.lanes.get(lane).is_some_and(|value| {
            value.edge_id == usize::MAX && value.next_lanes.first() == Some(&exit)
        })
}

#[test]
fn a_walker_takes_the_crossing_its_exit_needs() {
    let mut case = traced_case();
    let allocator = std::mem::take(&mut case.allocator);
    let mut agents = AgentSystem::new();
    let idx = walk_traced(&mut case, &mut agents, 1)[0];
    let mut reached = false;
    for _ in 0..1200 {
        agents.tick(
            &allocator,
            &mut case.network,
            &mut case.graph,
            0.1,
            &test_clock(0, 0),
        );
        if reached_exit(&agents, &case.network, idx, case.exit) {
            reached = true;
            break;
        }
    }
    assert!(
        reached,
        "walker stands on lane {} at d {:.2}",
        agents.current_lane_id[idx], agents.lane_distance[idx]
    );
    assert!(
        agents.trip_failures.lines.is_empty(),
        "{:#?}",
        agents.trip_failures.lines
    );
}

#[test]
fn walks_by_witness_are_identical_for_1_and_8_workers() {
    let run = |workers: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let mut case = traced_case();
                let allocator = std::mem::take(&mut case.allocator);
                let mut agents = AgentSystem::new();
                let ids = walk_traced(&mut case, &mut agents, 24);
                // More than `PAR_THRESHOLD` agents, so the movement pass runs on the pool.
                for _ in 0..500 {
                    agents.spawn_housed_agent(0, 0.0, 0.0);
                }
                let mut reached = vec![false; ids.len()];
                for _ in 0..1500 {
                    agents.tick(
                        &allocator,
                        &mut case.network,
                        &mut case.graph,
                        0.1,
                        &test_clock(0, 0),
                    );
                    for (seen, &idx) in reached.iter_mut().zip(&ids) {
                        *seen |= reached_exit(&agents, &case.network, idx, case.exit);
                    }
                }
                let reached = reached.iter().filter(|&&seen| seen).count();
                let state: Vec<_> = ids
                    .iter()
                    .map(|&idx| {
                        (
                            agents.current_lane_id[idx],
                            agents.lane_distance[idx].to_bits(),
                            agents.pos_x[idx].to_bits(),
                            agents.pos_y[idx].to_bits(),
                            agents.walk_lanes[idx].clone(),
                        )
                    })
                    .collect();
                (reached, state)
            })
    };
    let serial = run(1);
    assert_eq!(serial.0, 24, "every walker reaches its exit");
    assert_eq!(serial, run(8));
}

/// A crossing removed under a walker in flight (the graph changed without the editor, which
/// would clear the route): its witness has no step at the junction any more, so it replans from
/// its own lane through the normal planner, here round the north road end, and walks on without
/// leaving its sidewalk.
#[test]
fn a_walker_whose_crossing_is_removed_replans_from_its_own_lane() {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    let (center, west, east) = (0_u32, 1_u32, 2_u32);
    let (west_edge, east_edge) = (0_usize, 1_usize);
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(create_test_building(east_edge, 1));
    let side_lane = |network: &TransitNetwork, graph: &RegionGraph, node: u32| {
        foot_lanes_from(network, graph, east_edge, node)
            .into_iter()
            .find(|&lane| network.lane_system.lanes[lane].lane_idx > 0)
            .unwrap()
    };
    let entrance = |network: &TransitNetwork, graph: &RegionGraph| BuildingEntrance {
        edge_idx: east_edge,
        side: 1,
        entrance_s_m: 50.0,
        foot_lane_fwd: side_lane(network, graph, east),
        foot_lane_bkw: side_lane(network, graph, center),
        ..BuildingEntrance::default()
    };
    allocator.entrances.push(entrance(&network, &graph));

    let start = foot_lanes_from(&network, &graph, west_edge, west)[0];
    let start_len = network.lane_system.lanes[start].length;
    let mut agents = AgentSystem::new();
    let idx = agents.spawn_border_arrival_agent(usize::MAX, west, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.transit_mode[idx] = MODE_WALK;
    agents.speed[idx] = AGENT_WALK_SPEED_MS;
    agents.current_node[idx] = west;
    agents.current_edge[idx] = west_edge;
    agents.current_lane_id[idx] = start;
    agents.lane_distance[idx] = start_len - 3.0;
    agents.target_building[idx] = 0;
    agents.current_path[idx] = vec![west, center, east];
    agents.current_path_index[idx] = 1;
    agents.access_flags[idx] = ACCESS_PLAN_VALID;
    agents.planned_detach_node[idx] = east;
    agents.planned_detach_lane_id[idx] = side_lane(&network, &graph, east) as u32;
    agents.planned_detach_lane_d[idx] = 40.0;

    // Every crossing at the junction goes while the walker approaches it; the toggle's lane
    // rebuild reattaches it to its rebuilt sidewalk, as `set_crosswalk_override_internal` does.
    let start_idx = network.lane_system.lanes[start].lane_idx;
    turn_crossings(&mut graph, &mut network, center, false);
    allocator.entrances[0] = entrance(&network, &graph);
    let rebuilt = foot_lanes_from(&network, &graph, west_edge, west)
        .into_iter()
        .find(|&lane| network.lane_system.lanes[lane].lane_idx == start_idx)
        .unwrap();
    agents.current_lane_id[idx] = rebuilt;
    agents.planned_detach_lane_id[idx] = side_lane(&network, &graph, east) as u32;

    let mut last = (agents.pos_x[idx], agents.pos_y[idx]);
    let mut replanned = false;
    let mut reached_east = false;
    for tick in 0..1500 {
        agents.tick(&allocator, &mut network, &mut graph, 0.1, &test_clock(0, 0));
        let pos = (agents.pos_x[idx], agents.pos_y[idx]);
        if tick > 0 {
            let step = (pos.0 - last.0).hypot(pos.1 - last.1);
            assert!(step < 2.0, "no teleport: {step} m in one tick {tick}");
        }
        last = pos;
        replanned |= agents.current_path[idx].len() > 3;
        let lane = agents.current_lane_id[idx];
        reached_east |=
            network.lane_system.lanes.get(lane).is_some_and(|value| {
                value.edge_id == east_edge && value.lane_type == LaneType::Foot
            });
        if reached_east {
            break;
        }
    }
    assert!(replanned, "the walker replans a detour round a road end");
    assert!(reached_east, "the walker reaches the east road on foot");
    let lines = &agents.trip_failures.lines;
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(
        lines[0].contains("drops its route and replans"),
        "{lines:#?}"
    );
}
