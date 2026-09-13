// SPDX-License-Identifier: GPL-2.0-only

//! IDM following and overlap correction; congestion lifecycle is covered in lane_dynamics.

use super::support::*;
use super::*;

#[test]
fn test_idm_free_road_accelerates() {
    // A stopped car on an empty road should accelerate after one tick.
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let allocator = BuildingAllocator::new();
    let i = place_on_lane(&mut agents, edge_idx, fwd_lane, 10.0, 0.0);
    agents.tick(&allocator, &mut network, &mut graph, 1.0, &test_clock(0, 0));
    assert!(
        agents.speed[i] > 0.0,
        "stopped car should accelerate on free road"
    );
}

#[test]
fn test_idm_following_car_slower_than_free_car() {
    // Car A ahead, car B close behind — B must finish with lower speed than a lone free car.
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let allocator = BuildingAllocator::new();
    // Place a fast leader at dist=60 and a follower at dist=50.
    let _leader = place_on_lane(&mut agents, edge_idx, fwd_lane, 60.0, 40.0);
    let follower = place_on_lane(&mut agents, edge_idx, fwd_lane, 50.0, 40.0);

    // Reference: a lone car at the same position as the follower.
    let (mut network2, mut graph2, edge2, fwd2) = setup_straight_road();
    let mut agents2 = AgentSystem::new();
    let alloc2 = BuildingAllocator::new();
    let free_car = place_on_lane(&mut agents2, edge2, fwd2, 50.0, 40.0);

    agents.tick(&allocator, &mut network, &mut graph, 0.5, &test_clock(0, 0));
    agents2.tick(&alloc2, &mut network2, &mut graph2, 0.5, &test_clock(0, 0));

    assert!(
        agents.speed[follower] <= agents2.speed[free_car] + 0.01,
        "follower speed {} should not exceed free car speed {}",
        agents.speed[follower],
        agents2.speed[free_car]
    );
}

#[test]
fn test_overlap_correction_separates_cars() {
    use crate::config::{CAR_LENGTH, IDM_S_MIN};
    // Place two cars so they overlap: front car at 20 m, rear car at 20 m + gap < CAR_LENGTH.
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let allocator = BuildingAllocator::new();
    let front = place_on_lane(&mut agents, edge_idx, fwd_lane, 20.0, 10.0);
    let rear = place_on_lane(&mut agents, edge_idx, fwd_lane, 19.5, 10.0); // 0.5 m apart < CAR_LENGTH
    agents.tick(&allocator, &mut network, &mut graph, 0.1, &test_clock(0, 0));
    let gap = agents.lane_distance[front] - agents.lane_distance[rear];
    assert!(
        gap >= CAR_LENGTH + IDM_S_MIN - 0.01,
        "gap {gap:.3} m after tick must be >= CAR_LENGTH + IDM_S_MIN = {:.3}",
        CAR_LENGTH + IDM_S_MIN
    );
}
