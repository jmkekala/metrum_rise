// SPDX-License-Identifier: GPL-2.0-only

//! Lane-transition arbitration regressions and isolated movement measurements.

use super::*;
use crate::config::{CAR_LENGTH, IDM_S_MIN};
use crate::simulation::economy::agents::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, MODE_WALK,
    TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use crate::simulation::network::graph::Edge;
use crate::simulation::network::lanes::{Lane, LaneSuccessors, LaneType};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use godot::prelude::Vector3;

// Isolate the movement boundary: two connectors merge into each independent exit lane.
// No road-end or route query occurs; the generated-road regression covers that integration.
fn connector_exit_fixture(count: usize, blocked: bool, mode: u8) -> (AgentSystem, TransitNetwork) {
    let mut agents = AgentSystem::new();
    let mut network = TransitNetwork::new();
    for group in 0..count {
        let base = network.lane_system.lanes.len();
        for connector in [true, true, false] {
            let (start, end) = if connector {
                (0.0, 10.0)
            } else {
                (10.0, 110.0)
            };
            network.lane_system.lanes.push(Lane {
                edge_id: if connector { usize::MAX } else { group },
                node_id: if connector { group } else { usize::MAX },
                geometry: vec![Vector3::new(start, 0.0, 0.0), Vector3::new(end, 0.0, 0.0)],
                length: end - start,
                cum_dist: vec![0.0, end - start],
                lane_type: if mode == MODE_CAR {
                    LaneType::Vehicle
                } else {
                    LaneType::Foot
                },
                next_lanes: if connector {
                    LaneSuccessors::one(base + 2)
                } else {
                    LaneSuccessors::default()
                },
                ..Default::default()
            });
        }
        for offset in 0..if blocked { 3 } else { 2 } {
            let i = agents.spawn_housed_agent(usize::MAX, 9.5, 0.0);
            agents.transit_mode[i] = mode;
            agents.current_lane_id[i] = base + offset;
            agents.transit[i] = if offset < 2 {
                TRANSIT_INTERSECTION
            } else {
                TRANSIT_NETWORK
            };
            agents.lane_distance[i] = if offset < 2 { 9.5 } else { 1.0 };
            agents.speed[i] = if offset < 2 { 4.0 } else { 0.0 };
            agents.current_path[i] = vec![0, 1];
            agents.current_path_index[i] = 1;
        }
    }
    (agents, network)
}

#[test]
fn pedestrian_connector_cannot_publish_a_vehicle_lane_exit() {
    let (mut agents, mut network) = connector_exit_fixture(1, false, MODE_WALK);
    // A malformed connection must fail at the transition, before the walker occupies asphalt.
    network.lane_system.lanes[2].lane_type = LaneType::Vehicle;
    let allocator = BuildingAllocator::new();
    let graph = RegionGraph::new();
    let clock = TimeSystem::new();
    let n = agents.len();
    agents.prepare_lane_buckets_for_tick(&network, n);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        agents.dispatch_movement_pass(&allocator, &network, &graph, 0.25, &clock, n);
    }));
    let error = result.expect_err("cross-mode connector exit must be rejected");
    let message = error.downcast_ref::<String>().expect("contract diagnostic");
    assert!(message.contains("lane transition rejected: agent=0 mode=0"));
    assert!(message.contains("to_lane=2"));
    assert_eq!(agents.current_lane_id[0], 0);
    assert_eq!(agents.transit[0], TRANSIT_INTERSECTION);
}

fn lane_change_fixture(count: usize, cruise: bool) -> (AgentSystem, TransitNetwork) {
    let mut agents = AgentSystem::new();
    let mut network = TransitNetwork::new();
    for group in 0..count {
        let base = network.lane_system.lanes.len();
        for lane_idx in 0..3 {
            network.lane_system.lanes.push(Lane {
                edge_id: group,
                lane_idx,
                geometry: vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)],
                cum_dist: vec![0.0, 100.0],
                length: 100.0,
                ..Default::default()
            });
        }
        network
            .lane_system
            .edge_lanes
            .insert(group, vec![base, base + 1, base + 2]);
        // The inner car returns outward while the outer car overtakes a stopped leader.
        for (offset, lane) in [0, 2, 2].into_iter().enumerate() {
            let i = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
            agents.transit_mode[i] = MODE_CAR;
            agents.transit[i] = TRANSIT_NETWORK;
            agents.current_lane_id[i] = base + if cruise { 2 } else { lane };
            agents.lane_distance[i] = if cruise {
                offset as f32 * 20.0
            } else if offset == 2 {
                10.0
            } else {
                0.0
            };
            agents.speed[i] = if !cruise && offset == 2 { 0.0 } else { 4.0 };
            agents.overtake_blocked_time_s[i] = if !cruise && offset == 1 { 3.0 } else { 0.0 };
        }
    }
    (agents, network)
}

fn move_fixture(agents: &mut AgentSystem, network: &TransitNetwork) {
    let n = agents.len();
    agents.invalidate_lane_bucket_snapshot();
    agents.prepare_lane_buckets_for_tick(network, n);
    agents.dispatch_movement_pass(
        &BuildingAllocator::new(),
        network,
        &RegionGraph::new(),
        0.25,
        &TimeSystem::new(),
        n,
    );
}

#[test]
fn opposing_lane_changes_reserve_the_shared_target() {
    for workers in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                // Competing pairs straddle classification chunk boundaries.
                let groups = 5_462;
                let (mut agents, network) = lane_change_fixture(groups, false);
                move_fixture(&mut agents, &network);
                for group in 0..groups {
                    let base = group * 3;
                    assert_eq!(agents.current_lane_id[base], base + 1);
                    assert_eq!(agents.lane_change_from_lane_id[base], base as u32);
                    assert_eq!(agents.current_lane_id[base + 1], base + 2);
                    assert_eq!(agents.lane_change_from_lane_id[base + 1], u32::MAX);
                }
            });
    }
}

#[test]
fn connector_exits_wait_for_space_and_serialize_merges() {
    let groups = 256;
    for workers in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                for blocked in [false, true] {
                    let (mut agents, network) = connector_exit_fixture(groups, blocked, MODE_CAR);
                    move_fixture(&mut agents, &network);
                    let stride = if blocked { 3 } else { 2 };
                    for group in 0..groups {
                        let a = group * stride;
                        assert_eq!(
                            agents.current_lane_id[a],
                            group * 3 + if blocked { 0 } else { 2 }
                        );
                        assert_eq!(agents.current_lane_id[a + 1], group * 3 + 1);
                        assert_eq!(agents.lane_distance[a + 1], 10.0);
                        assert_eq!(agents.current_path[a + 1], [0, 1]);
                        assert_eq!(agents.current_path_index[a + 1], 1);
                        if blocked {
                            assert_eq!(agents.lane_distance[a], 10.0);
                            agents.lane_distance[a + 2] = 20.0;
                        } else {
                            assert_eq!(agents.lane_distance[a], 0.5);
                            agents.lane_distance[a] = 20.0;
                        }
                    }
                    move_fixture(&mut agents, &network);
                    for group in 0..groups {
                        let winner = group * stride + usize::from(!blocked);
                        assert_eq!(agents.current_lane_id[winner], group * 3 + 2);
                        assert_eq!(agents.transit[winner], TRANSIT_NETWORK);
                    }
                }
                let (mut walkers, network) = connector_exit_fixture(1, false, MODE_WALK);
                move_fixture(&mut walkers, &network);
                assert_eq!(walkers.current_lane_id, [2, 2]);
            });
    }
}

#[test]
#[ignore = "manual matched release timing of connector exits and lane changes"]
fn benchmark_lane_transitions() {
    use std::{hint::black_box, time::Instant};

    let allocator = BuildingAllocator::new();
    let graph = RegionGraph::new();
    let clock = TimeSystem::new();
    for count in [512, 8_192, 65_536] {
        for scenario in ["clear", "merge", "blocked", "lateral", "cruise"] {
            let blocked = scenario == "blocked";
            let cruise = scenario == "cruise";
            let lateral = scenario == "lateral" || cruise;
            let (mut agents, network) = if lateral {
                lane_change_fixture(count, cruise)
            } else {
                connector_exit_fixture(count, blocked, MODE_CAR)
            };
            let n = agents.len();
            let stride = if blocked || lateral { 3 } else { 2 };
            let mut samples = [0.0; 21];
            for sample in 0..24 {
                for group in 0..count {
                    for offset in 0..stride {
                        let i = group * stride + offset;
                        if cruise {
                            agents.lane_distance[i] = offset as f32 * 20.0;
                            continue;
                        }
                        if lateral {
                            agents.current_lane_id[i] = group * 3 + if offset == 0 { 0 } else { 2 };
                            agents.lane_distance[i] = if offset == 2 { 10.0 } else { 0.0 };
                            agents.lane_change_from_lane_id[i] = u32::MAX;
                            agents.lane_change_start_d[i] = 0.0;
                            agents.lane_change_length_m[i] = 0.0;
                            agents.overtake_cooldown_s[i] = 0.0;
                            agents.overtake_blocked_time_s[i] = if offset == 1 { 3.0 } else { 0.0 };
                            continue;
                        }
                        agents.current_lane_id[i] = group * 3 + offset;
                        agents.transit[i] = if offset < 2 {
                            TRANSIT_INTERSECTION
                        } else {
                            TRANSIT_NETWORK
                        };
                        agents.lane_distance[i] = if offset == 1 && scenario == "clear" {
                            0.0
                        } else if offset < 2 {
                            9.5
                        } else {
                            1.0
                        };
                        agents.current_edge[i] = usize::MAX;
                    }
                }
                agents.invalidate_lane_bucket_snapshot();
                agents.prepare_lane_buckets_for_tick(&network, n);
                let start = Instant::now();
                agents.dispatch_movement_pass(&allocator, &network, &graph, 0.25, &clock, n);
                if sample >= 3 {
                    samples[sample - 3] = start.elapsed().as_secs_f64() * 1_000.0;
                }
                black_box(&agents);
            }
            samples.sort_by(f64::total_cmp);
            let transitions = if lateral {
                agents
                    .lane_change_from_lane_id
                    .iter()
                    .filter(|&&lane| lane != u32::MAX)
                    .count()
            } else {
                agents
                    .transit
                    .iter()
                    .filter(|&&state| state == TRANSIT_NETWORK)
                    .count()
                    - if blocked { count } else { 0 }
            };
            eprintln!(
                "lane_transition groups={count} scenario={scenario} agents={n} median_ms={:.6} transitions={transitions}",
                samples[10]
            );
        }
    }
}

#[test]
fn lane_change_that_can_reach_a_shorter_lane_end_stays_in_dynamic_phase() {
    let (mut agents, mut network) = lane_change_fixture(1, false);
    network.lane_system.lanes[1].length = 0.5;
    network.lane_system.lanes[1].geometry[1].x = 0.5;
    network.lane_system.lanes[1].cum_dist[1] = 0.5;
    let n = agents.len();
    agents.prepare_lane_buckets_for_tick(&network, n);
    agents.prepare_lane_claims(
        &BuildingAllocator::new(),
        &network,
        &RegionGraph::new(),
        0.25,
        n,
    );
    assert_eq!(
        agents.movement_claim_modes,
        [
            MovementClaimMode::Dynamic,
            MovementClaimMode::Dynamic,
            MovementClaimMode::Parallel
        ]
    );
}

#[test]
#[ignore = "manual matched release timing of retained lane reservation reset"]
fn benchmark_empty_lane_claim_reset() {
    use std::{hint::black_box, time::Instant};

    for count in [1_024, 16_384, 131_072, 1_048_576] {
        let mut agents = AgentSystem::new();
        let mut network = TransitNetwork::new();
        network.lane_system.lanes.resize_with(count, Lane::default);
        for _ in 0..3 {
            agents.prepare_lane_buckets_for_tick(&network, 0);
        }
        let mut samples = [0.0; 21];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..8 {
                black_box(agents.prepare_lane_buckets_for_tick(black_box(&network), 0));
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 8.0;
        }
        assert_eq!(
            agents.prepare_lane_buckets_for_tick(&network, 0),
            (count, 0)
        );
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "lane_claim_reset lanes={count} median_ms={:.6}",
            samples[10]
        );
    }
}

// Border-origin freight: `groups` independent one-lane roads, each with `cars` trucks waiting at
// the start node with a valid route and no lane (`TRAFFIC-07`).
fn node_entry_fixture(groups: usize, cars: usize) -> (AgentSystem, TransitNetwork, RegionGraph) {
    let mut agents = AgentSystem::new();
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    for group in 0..groups {
        let z = group as f32 * 10.0;
        let start = graph.add_node(Vector3::new(0.0, 0.0, z), NodeType::Junction);
        let end = graph.add_node(Vector3::new(2_000.0, 0.0, z), NodeType::Junction);
        let geometry = vec![Vector3::new(0.0, 0.0, z), Vector3::new(2_000.0, 0.0, z)];
        let edge = graph.add_edge(Edge {
            start_node: start,
            end_node: end,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 0,
            speed_limit: 14.0,
            base_cost: 150.0,
            physical_length: 2_000.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: geometry.clone(),
            physical_geometry: geometry.clone(),
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
        let lane = network.lane_system.lanes.len();
        network.lane_system.lanes.push(Lane {
            edge_id: edge,
            geometry,
            length: 2_000.0,
            cum_dist: vec![0.0, 2_000.0],
            ..Default::default()
        });
        network.lane_system.edge_lanes.insert(edge, vec![lane]);
        for _ in 0..cars {
            let i = agents.spawn_housed_agent(usize::MAX, 0.0, z);
            agents.transit_mode[i] = MODE_CAR;
            agents.transit[i] = TRANSIT_NETWORK;
            agents.current_node[i] = start;
            agents.current_path[i] = vec![end];
            agents.current_path_index[i] = 0;
            agents.access_flags[i] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
            agents.freight_target_border_node[i] = end;
            agents.freight_shipment_id[i] = 1_000 + i as u64;
        }
    }
    (agents, network, graph)
}

// Runs the fixture for `ticks` and returns every agent's lane state as exact bits.
fn run_node_entry(
    groups: usize,
    cars: usize,
    ticks: usize,
    workers: usize,
) -> Vec<(usize, u32, u32)> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let (mut agents, mut network, mut graph) = node_entry_fixture(groups, cars);
            let allocator = BuildingAllocator::new();
            let clock = TimeSystem::new();
            for _ in 0..ticks {
                agents.tick(&allocator, &mut network, &mut graph, 0.25, &clock);
            }
            let min_gap = CAR_LENGTH + IDM_S_MIN - 1e-3;
            for group in 0..groups {
                let mut on_lane: Vec<f32> = Vec::new();
                for i in group * cars..(group + 1) * cars {
                    if agents.current_lane_id[i] == group {
                        on_lane.push(agents.lane_distance[i]);
                        continue;
                    }
                    // A car that could not enter waits at its node with its trip and cargo.
                    assert_eq!(agents.current_lane_id[i], usize::MAX, "agent {i}");
                    assert_eq!(agents.transit[i], TRANSIT_NETWORK);
                    assert_eq!(agents.current_node[i], group as u32 * 2);
                    assert_eq!(agents.current_path[i], [group as u32 * 2 + 1]);
                    assert_eq!(agents.current_path_index[i], 0);
                    assert_eq!(agents.speed[i], 0.0);
                    assert_eq!(agents.freight_shipment_id[i], 1_000 + i as u64);
                    assert_eq!(
                        agents.access_flags[i],
                        ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION
                    );
                }
                on_lane.sort_by(f32::total_cmp);
                for pair in on_lane.windows(2) {
                    assert!(
                        pair[1] - pair[0] >= min_gap,
                        "group {group}: cars stacked at {} and {} ({} on the lane)",
                        pair[0],
                        pair[1],
                        on_lane.len()
                    );
                }
                assert!(!on_lane.is_empty() && on_lane.len() < cars, "group {group}");
            }
            (0..agents.len())
                .map(|i| {
                    (
                        agents.current_lane_id[i],
                        agents.lane_distance[i].to_bits(),
                        agents.speed[i].to_bits(),
                    )
                })
                .collect()
        })
}

#[test]
fn node_origin_cars_wait_for_an_entry_gap() {
    // 80 roads x 64 trucks cross the parallel classification chunk, so 1 and 8 workers differ
    // in how the held cars are scheduled; the results must not.
    let one = run_node_entry(80, 64, 24, 1);
    let eight = run_node_entry(80, 64, 24, 8);
    assert!(one == eight, "worker count changed the lane state");
}
