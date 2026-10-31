// SPDX-License-Identifier: GPL-2.0-only

//! The detach rule: a car detaches at its door's lane end (`TRAFFIC-15`) and only on its route's
//! last visit to the detach lane (`TRAFFIC-13`).

use super::support::*;
use super::*;

// Builds `streets` copies, 40 m apart, of a corner: junction Q joins a 30 m two-way street to
// dead-end corner P (edge 3s), a 600 m approach from dead end R (edge 3s + 1) and a 100 m stub
// (edge 3s + 2), so Q has connectors. Returns the network and graph.
fn corner_streets(streets: usize) -> (TransitNetwork, RegionGraph) {
    let mut graph = RegionGraph::new();
    for s in 0..streets {
        let z = s as f32 * 40.0;
        for (x, dz) in [(0.0, 0.0), (30.0, 0.0), (630.0, 0.0), (30.0, 100.0)] {
            graph.add_node(Vector3::new(x, 0.0, z + dz), NodeType::Junction);
        }
        let n = 4 * s as u32;
        // P = n, Q = n + 1, R = n + 2, stub end = n + 3.
        for (a, b) in [(n, n + 1), (n + 1, n + 2), (n + 1, n + 3)] {
            let (from, to) = (graph.node(a).pos, graph.node(b).pos);
            let length = from.distance_to(to);
            graph.add_edge(Edge {
                physical_length: length,
                base_cost: length / 14.0,
                speed_limit: 14.0,
                geometry: vec![from, to],
                physical_geometry: vec![from, to],
                ..create_test_edge(a, b)
            });
        }
    }
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    (network, graph)
}

// `TRAFFIC-15` fixture run: on every street a corner store at `frontage_t` 0 of the 30 m street
// (corner P), so the westbound lane Q to P sees its door at the lane's end, and `cars` shoppers
// on the approach heading for it. Returns, per car, whether it detached into the store from the
// westbound lane and the final state as bits; panics if a car leaves the westbound lane without
// detaching (the missed door that sends it round the block on `dev`).
fn run_corner_store(streets: usize, cars: usize, ticks: usize, workers: usize) -> Vec<(bool, u64)> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let (mut network, mut graph) = corner_streets(streets);
            let mut allocator = BuildingAllocator::new();
            let asset_id = register_test_asset(
                &mut allocator,
                "test",
                "corner_store",
                ZoneClass::Commercial,
            );
            let mut doors = Vec::new();
            for s in 0..streets {
                let (street, approach) = (3 * s, 3 * s + 1);
                let eastbound = fwd_vehicle_lanes(&network, street)[0];
                let westbound = bkw_vehicle_lanes(&network, street)[0];
                let mut store = create_test_building(street, 1);
                store.asset_id = asset_id.clone();
                store.frontage_t = 0.0;
                allocator.buildings.push(store);
                let door = Vector2::new(0.0, s as f32 * 40.0 + 8.0);
                allocator.entrances.push(BuildingEntrance {
                    edge_idx: street,
                    side: 1,
                    door_pos: door,
                    curb_pos: door,
                    car_lane_fwd: eastbound,
                    car_lane_bkw: westbound,
                    ..BuildingEntrance::default()
                });
                doors.push((westbound, approach));
            }
            let mut agents = AgentSystem::new();
            for (s, &(westbound, approach)) in doors.iter().enumerate() {
                let lane = bkw_vehicle_lanes(&network, approach)[0];
                let length = network.lane_system.lanes[lane].length;
                let (q, r) = (4 * s as u32 + 1, 4 * s as u32 + 2);
                for car in 0..cars {
                    let idx = agents.spawn_border_arrival_agent(usize::MAX, r, 0.0, 0.0);
                    agents.transit_mode[idx] = MODE_CAR;
                    let d = length - 200.0 + car as f32 * 10.0;
                    agents
                        .agents
                        .enter_lane(idx, &network.lane_system, lane, d, r);
                    agents.speed[idx] = 10.0;
                    agents.target_building[idx] = s;
                    agents.current_path[idx] = vec![r, q];
                    agents.current_path_index[idx] = 1;
                    agents.access_flags[idx] = ACCESS_PLAN_VALID;
                    agents.planned_detach_node[idx] = q;
                    agents.planned_detach_lane_id[idx] = westbound as u32;
                    // The city's case: the door projects exactly onto the lane's end.
                    agents.planned_detach_lane_d[idx] = network.lane_system.lanes[westbound].length;
                }
            }
            let n = agents.len();
            let westbound_of = |i: usize| doors[i / cars].0;
            let mut detached = vec![false; n];
            let mut previous: Vec<usize> = agents.current_lane_id.clone();
            for tick in 0..ticks {
                agents.tick(
                    &allocator,
                    &mut network,
                    &mut graph,
                    0.25,
                    &test_clock(0, 0),
                );
                for i in 0..n {
                    if detached[i] {
                        continue;
                    }
                    if agents.transit[i] == TRANSIT_ACCESS_INGRESS {
                        assert_eq!(previous[i], westbound_of(i), "car {i} tick {tick}");
                        detached[i] = true;
                    } else {
                        assert!(
                            previous[i] != westbound_of(i)
                                || agents.current_lane_id[i] == westbound_of(i),
                            "car {i} tick {tick}: left its door's lane without detaching"
                        );
                    }
                    previous[i] = agents.current_lane_id[i];
                }
            }
            (0..n)
                .map(|i| {
                    let state = (u64::from(agents.transit[i]) << 48)
                        ^ (u64::from(agents.pos_x[i].to_bits()) << 16)
                        ^ u64::from(agents.pos_y[i].to_bits());
                    (detached[i], state)
                })
                .collect()
        })
}

#[test]
fn corner_store_door_at_the_lane_end_is_reached() {
    // 64 streets x 8 shoppers pass the parallel dispatch threshold, so 1 and 8 workers
    // schedule the movement pass differently; the results must not differ.
    let one = run_corner_store(64, 8, 320, 1);
    let eight = run_corner_store(64, 8, 320, 8);
    let missed = one.iter().filter(|(detached, _)| !detached).count();
    assert_eq!(missed, 0, "{missed} shoppers never reached the store");
    assert!(one == eight, "worker count changed the outcome");
}

#[test]
fn looping_route_detaches_only_on_its_last_visit() {
    // One 1 km two-way road between dead ends 0 and 1 (U-turn connectors at both). The car
    // attached at 950 m of the eastbound lane; its door is at 50 m of the same lane, so its
    // route [1, 0] runs to node 1, back to node 0 and onto the eastbound lane again.
    let mut graph = RegionGraph::new();
    graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    graph.add_node(Vector3::new(1_000.0, 0.0, 0.0), NodeType::Junction);
    let road = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(1_000.0, 0.0, 0.0)];
    graph.add_edge(Edge {
        physical_length: 1_000.0,
        base_cost: 1_000.0 / 14.0,
        speed_limit: 14.0,
        geometry: road.to_vec(),
        physical_geometry: road.to_vec(),
        ..create_test_edge(0, 1)
    });
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let eastbound = fwd_vehicle_lanes(&network, 0)[0];
    let westbound = bkw_vehicle_lanes(&network, 0)[0];

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(&mut allocator, "test", "t13_home", ZoneClass::Residential);
    let mut home = create_test_building(0, 1);
    home.asset_id = asset_id;
    allocator.buildings.push(home);
    let door = Vector2::new(50.0, 8.0);
    allocator.entrances.push(BuildingEntrance {
        edge_idx: 0,
        side: 1,
        door_pos: door,
        curb_pos: door,
        car_lane_fwd: eastbound,
        car_lane_bkw: westbound,
        ..BuildingEntrance::default()
    });

    let mut agents = AgentSystem::new();
    let idx = agents.spawn_border_arrival_agent(0, 0, 0.0, 0.0);
    agents.transit_mode[idx] = MODE_CAR;
    agents
        .agents
        .enter_lane(idx, &network.lane_system, eastbound, 950.0, 0);
    agents.speed[idx] = 10.0;
    agents.target_building[idx] = 0;
    // As a building-origin plan installs it: the attach lane's terminal first, index 1.
    agents.current_path[idx] = vec![1, 0];
    agents.current_path_index[idx] = 1;
    agents.access_flags[idx] = ACCESS_PLAN_VALID;
    agents.planned_detach_node[idx] = 0;
    agents.planned_detach_lane_id[idx] = eastbound as u32;
    agents.planned_detach_lane_d[idx] = 50.0;

    let mut came_back = false;
    for tick in 0..2_400 {
        agents.tick(
            &allocator,
            &mut network,
            &mut graph,
            0.25,
            &test_clock(0, 0),
        );
        came_back |= agents.current_lane_id[idx] == westbound;
        if agents.transit[idx] == TRANSIT_ACCESS_INGRESS {
            assert!(came_back, "tick {tick}: detached before driving the loop");
            return;
        }
    }
    panic!(
        "the car never detached: lane {} d {} came_back {came_back}",
        agents.current_lane_id[idx], agents.lane_distance[idx]
    );
}
