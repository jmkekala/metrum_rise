// SPDX-License-Identifier: GPL-2.0-only

//! The agent inspector's planned route against the lanes cars actually drive.

use super::support::*;
use super::*;
use crate::simulation::economy::agents::tick::{RouteEnd, planned_route};

// Nodes of `build_4way_junction`: centre, then the W, E, N and S arm ends.
const CENTER: u32 = 0;
const WEST: u32 = 1;
const ARM_ENDS: [u32; 3] = [2, 3, 4];

fn car_on_lane(agents: &mut AgentSystem, lane_id: usize, lane_d: f32, path: Vec<u32>) -> usize {
    let idx = agents.spawn_border_arrival_agent(usize::MAX, WEST, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.transit_mode[idx] = MODE_CAR;
    agents.current_node[idx] = WEST;
    agents.current_edge[idx] = 0;
    agents.current_lane_id[idx] = lane_id;
    agents.lane_distance[idx] = lane_d;
    agents.speed[idx] = 10.0;
    agents.current_path[idx] = path;
    agents.current_path_index[idx] = 1;
    // Overtaking and returning to the outer lane are not part of the plan.
    agents.overtake_cooldown_s[idx] = f32::MAX;
    idx
}

fn planned_lanes(
    agents: &AgentSystem,
    idx: usize,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> (Vec<usize>, RouteEnd) {
    let route = planned_route(agents, idx, allocator, network, graph);
    (
        route.spans.iter().map(|span| span.lane_id).collect(),
        route.end,
    )
}

// Ticks the agent and returns the lanes it drives, from its current one, until it leaves the
// lanes or reaches `last`.
fn driven_lanes(
    agents: &mut AgentSystem,
    idx: usize,
    allocator: &BuildingAllocator,
    network: &mut TransitNetwork,
    graph: &mut RegionGraph,
    last: usize,
) -> Vec<usize> {
    let mut driven = vec![agents.current_lane_id[idx]];
    for _ in 0..2000 {
        agents.tick(allocator, network, graph, 0.1, &test_clock(0, 0));
        let now = agents.current_lane_id[idx];
        if now == usize::MAX {
            break;
        }
        if driven.last() != Some(&now) {
            driven.push(now);
        }
        if now == last {
            break;
        }
    }
    driven
}

// Gives every connector at `node` an identical twin, so each turn has two equally short
// connectors and the movement tick picks one by its seed.
fn twin_connectors(network: &mut TransitNetwork, node: u32) -> usize {
    let lanes = &mut network.lane_system.lanes;
    let originals = lanes.len();
    for from in 0..originals {
        let connectors: Vec<usize> = lanes[from]
            .next_lanes
            .iter()
            .copied()
            .filter(|&id| lanes[id].edge_id == usize::MAX && lanes[id].node_id == node as usize)
            .collect();
        for connector in connectors {
            let twin = lanes[connector].clone();
            lanes.push(twin);
            let twin_id = lanes.len() - 1;
            lanes[from].next_lanes.push(twin_id);
        }
    }
    originals
}

#[test]
fn planned_route_is_the_lanes_the_car_drives_through_a_junction() {
    let allocator = BuildingAllocator::new();
    for lane_slot in 0..2 {
        for arm_end in ARM_ENDS {
            let (mut network, mut graph, arms) = build_4way_junction(2, 2);
            let lane_id = arms[0][lane_slot];
            let lane_d = network.lane_system.lanes[lane_id].length - 5.0;
            let mut agents = AgentSystem::new();
            let idx = car_on_lane(&mut agents, lane_id, lane_d, vec![WEST, CENTER, arm_end]);
            let (planned, end) = planned_lanes(&agents, idx, &allocator, &network, &graph);
            assert_eq!(end, RouteEnd::PathEnd { node: arm_end });
            assert_eq!(planned.len(), 3, "lane, connector, exit lane: {planned:?}");

            let driven = driven_lanes(
                &mut agents,
                idx,
                &allocator,
                &mut network,
                &mut graph,
                usize::MAX,
            );
            assert_eq!(
                driven, planned,
                "lane slot {lane_slot} to node {arm_end}: driven vs planned"
            );
        }
    }
}

#[test]
fn planned_route_takes_the_connector_the_car_picks_among_equal_ones() {
    let allocator = BuildingAllocator::new();
    let mut twins_planned = 0;
    let mut cases = 0;
    for lane_slot in 0..2 {
        for arm_end in ARM_ENDS {
            let (mut network, mut graph, arms) = build_4way_junction(2, 2);
            let originals = twin_connectors(&mut network, CENTER);
            let lane_id = arms[0][lane_slot];
            let lane_d = network.lane_system.lanes[lane_id].length - 5.0;
            let mut agents = AgentSystem::new();
            let idx = car_on_lane(&mut agents, lane_id, lane_d, vec![WEST, CENTER, arm_end]);
            let (planned, _) = planned_lanes(&agents, idx, &allocator, &network, &graph);
            twins_planned += usize::from(planned[1] >= originals);
            cases += 1;
            let driven = driven_lanes(
                &mut agents,
                idx,
                &allocator,
                &mut network,
                &mut graph,
                usize::MAX,
            );
            assert_eq!(driven, planned, "lane slot {lane_slot} to node {arm_end}");
        }
    }
    // Both the original and the twin must be chosen somewhere, or the seed was never tested.
    assert!(
        twins_planned > 0 && twins_planned < cases,
        "{twins_planned} of {cases} twins"
    );
}

#[test]
fn planned_route_enters_the_detach_lane_the_car_enters_and_ends_at_the_door() {
    // The zero-hop access case: the path is used up at the centre, which is the detach node, so
    // the car crosses straight into the planned detach lane on the north arm. Twin connectors
    // make the crossing a seeded choice, which also depends on the held path index.
    let north_edge = 2;
    let door = Vector2::new(0.0, -105.0);
    for lane_slot in 0..2 {
        for detach_slot in 0..2 {
            let (mut network, mut graph, arms) = build_4way_junction(2, 2);
            twin_connectors(&mut network, CENTER);
            let lane_id = arms[0][lane_slot];
            let detach_lane = bkw_vehicle_lanes(&network, north_edge)[detach_slot];
            let mut allocator = BuildingAllocator::new();
            allocator
                .buildings
                .push(create_test_building(north_edge, -1));
            allocator.entrances.push(BuildingEntrance {
                edge_idx: north_edge,
                side: -1,
                entrance_s_m: 5.0,
                door_pos: door,
                curb_pos: Vector2::new(0.0, -100.0),
                car_lane_bkw: detach_lane,
                ..BuildingEntrance::default()
            });
            let lane_d = network.lane_system.lanes[lane_id].length - 5.0;
            let mut agents = AgentSystem::new();
            let idx = car_on_lane(&mut agents, lane_id, lane_d, Vec::new());
            agents.current_path_index[idx] = 0;
            agents.target_building[idx] = 0;
            agents.access_flags[idx] = ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH;
            agents.planned_detach_node[idx] = CENTER;
            agents.planned_detach_lane_id[idx] = detach_lane as u32;
            agents.planned_detach_lane_d[idx] = 5.0;

            let route = planned_route(&agents, idx, &allocator, &network, &graph);
            assert_eq!(route.end, RouteEnd::Destination);
            assert_eq!(
                route.ingress.last(),
                Some(&door),
                "the route must end at the door"
            );
            let planned: Vec<usize> = route.spans.iter().map(|span| span.lane_id).collect();
            assert_eq!(planned.last(), Some(&detach_lane));
            let driven = driven_lanes(
                &mut agents,
                idx,
                &allocator,
                &mut network,
                &mut graph,
                usize::MAX,
            );
            assert_eq!(
                driven, planned,
                "lane slot {lane_slot}, detach slot {detach_slot}"
            );
        }
    }
}

#[test]
fn planned_route_starts_at_the_car_and_ends_at_the_detach_point() {
    let (network, graph, fwd_lanes) = build_two_edge_road(1, 1);
    let allocator = BuildingAllocator::new();
    let detach_lane = fwd_vehicle_lanes(&network, 1)[0];
    let mut agents = AgentSystem::new();
    let idx = car_on_lane(&mut agents, fwd_lanes[0], 30.0, vec![0, 1]);
    agents.access_flags[idx] = ACCESS_PLAN_VALID;
    agents.planned_detach_node[idx] = 1;
    agents.planned_detach_lane_id[idx] = detach_lane as u32;
    agents.planned_detach_lane_d[idx] = 40.0;

    let route = planned_route(&agents, idx, &allocator, &network, &graph);
    assert_eq!(route.end, RouteEnd::Destination);
    let first = route.spans.first().unwrap();
    let last = route.spans.last().unwrap();
    assert_eq!((first.lane_id, first.from_d), (fwd_lanes[0], 30.0));
    assert_eq!((last.lane_id, last.to_d), (detach_lane, 40.0));

    // Already past the detach point on the detach lane: the car leaves the road at once.
    agents.current_lane_id[idx] = detach_lane;
    agents.lane_distance[idx] = 55.0;
    let route = planned_route(&agents, idx, &allocator, &network, &graph);
    assert_eq!(route.end, RouteEnd::Destination);
    assert_eq!(route.spans.len(), 1);
}

#[test]
fn replanning_changes_the_planned_route() {
    let (network, graph, arms) = build_4way_junction(1, 1);
    let allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();
    let idx = car_on_lane(
        &mut agents,
        arms[0][0],
        50.0,
        vec![WEST, CENTER, ARM_ENDS[0]],
    );
    let (east, _) = planned_lanes(&agents, idx, &allocator, &network, &graph);
    agents.current_path[idx] = vec![WEST, CENTER, ARM_ENDS[1]];
    let (north, end) = planned_lanes(&agents, idx, &allocator, &network, &graph);
    assert_eq!(east[0], north[0], "both routes start on the car's lane");
    assert_ne!(east[1..], north[1..]);
    assert_eq!(end, RouteEnd::PathEnd { node: ARM_ENDS[1] });

    // A path with no road onwards stops where the car would replan.
    agents.current_path[idx] = vec![WEST, CENTER, WEST + 100];
    let (_, end) = planned_lanes(&agents, idx, &allocator, &network, &graph);
    assert_eq!(end, RouteEnd::NoConnection { node: WEST + 100 });
}

#[test]
fn agents_off_the_road_have_no_planned_lanes() {
    let (network, graph, _) = build_4way_junction(1, 1);
    let allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();
    let idx = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
    let route = planned_route(&agents, idx, &allocator, &network, &graph);
    assert!(route.spans.is_empty() && route.egress.is_empty() && route.ingress.is_empty());
    assert_eq!(route.end, RouteEnd::None);
}
