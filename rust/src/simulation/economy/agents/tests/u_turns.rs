// SPDX-License-Identifier: GPL-2.0-only

//! `NET-07`: cars never follow a plan that needs a U-turn the lanes cannot drive.

use super::support::*;
use super::*;

// Junctions A (0) and B (1) joined by the frontage road, edge 0. Every other arm is a dead-end
// stub, where the lane builder creates U-turn connectors; A and B have three arms and none.
pub(super) fn frontage_town() -> (TransitNetwork, RegionGraph) {
    let mut graph = RegionGraph::new();
    for (x, z) in [
        (0.0, 0.0),
        (100.0, 0.0),
        (-150.0, 0.0),
        (0.0, -150.0),
        (250.0, 0.0),
        (100.0, 150.0),
    ] {
        graph.add_node(Vector3::new(x, 0.0, z), NodeType::Junction);
    }
    for (a, b) in [(0, 1), (2, 0), (3, 0), (1, 4), (1, 5)] {
        let (from, to) = (graph.node(a).pos, graph.node(b).pos);
        let length = from.distance_to(to);
        graph.add_edge(Edge {
            physical_length: length,
            base_cost: length / 14.0,
            geometry: vec![from, to],
            physical_geometry: vec![from, to],
            ..create_test_edge(a, b)
        });
    }
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    (network, graph)
}

#[test]
fn car_facing_away_from_its_destination_turns_back_legally() {
    // The car has driven past its home's door towards junction B. Turning straight back at B
    // is a U-turn the lanes do not build; the replan must loop through a dead end instead of
    // failing with "no turn ... into its exit lane" on every arrival at B.
    for side in [1, -1] {
        let (mut network, mut graph) = frontage_town();
        let mut allocator = BuildingAllocator::new();
        let asset_id =
            register_test_asset(&mut allocator, "test", "net07_home", ZoneClass::Residential);
        let mut home = create_test_building(0, side);
        home.asset_id = asset_id;
        // Mid-road, so the door is well inside both lanes' clipped extents.
        home.center_x = 50.0;
        home.center_y = f32::from(side) * 12.0;
        allocator.buildings.push(home);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);

        let lane = fwd_vehicle_lanes(&network, 0)[0];
        let mut agents = AgentSystem::new();
        let idx = agents.spawn_border_arrival_agent(0, 0, 0.0, 0.0);
        agents.transit_mode[idx] = MODE_CAR;
        agents
            .agents
            .enter_lane(idx, &network.lane_system, lane, 90.0, 0);
        agents.target_building[idx] = 0;
        agents.speed[idx] = 10.0;

        let mut arrived = false;
        for _ in 0..3000 {
            agents.tick(&allocator, &mut network, &mut graph, 0.1, &test_clock(0, 0));
            if matches!(
                agents.transit[idx],
                TRANSIT_ACCESS_INGRESS | TRANSIT_IN_BUILDING
            ) {
                arrived = true;
                break;
            }
        }
        let failures = &agents.trip_failures.lines;
        assert!(failures.is_empty(), "side={side}: {failures:?}");
        assert!(arrived, "side={side}: the car never reached its home");
    }
}

#[test]
fn loaded_routes_needing_a_forbidden_turn_are_dropped_for_replanning() {
    // A save made before NET-07 may hold a zero hop that U-turns at junction B, or a route
    // that turns back at it. Loading drops exactly those routes; legal ones are kept.
    let (network, graph) = frontage_town();
    let eastbound = fwd_vehicle_lanes(&network, 0)[0];
    let westbound = bkw_vehicle_lanes(&network, 0)[0];
    let stub = fwd_vehicle_lanes(&network, 3)[0];
    let mut agents = AgentSystem::new();
    for (path, detach_node, detach_lane) in [
        (vec![1], 1, westbound),
        (vec![1, 0], 0, fwd_vehicle_lanes(&network, 1)[0]),
        (vec![1, 4, 1], 1, westbound),
        (vec![1], 1, stub),
    ] {
        let idx = agents.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0);
        agents.transit_mode[idx] = MODE_CAR;
        agents
            .agents
            .enter_lane(idx, &network.lane_system, eastbound, 90.0, 0);
        agents.target_building[idx] = 0;
        agents.current_path[idx] = path;
        agents.current_path_index[idx] = 0;
        agents.planned_detach_node[idx] = detach_node;
        agents.planned_detach_lane_id[idx] = detach_lane as u32;
        agents.access_flags[idx] = ACCESS_PLAN_VALID;
    }
    agents.clear_routes_with_illegal_turns(&network.lane_system, &graph);
    let kept: Vec<_> = (0..agents.len())
        .map(|i| agents.access_flags[i] & ACCESS_PLAN_VALID != 0)
        .collect();
    assert_eq!(kept, [false, false, true, true]);
    assert!(agents.current_path[0].is_empty() && agents.current_path[1].is_empty());
}
