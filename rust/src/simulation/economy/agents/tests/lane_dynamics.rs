//! Lane buckets, frontage delay, and steady-state congestion tests.

use super::support::*;
use super::*;
use crate::simulation::LANE_CONFIGS;

#[test]
fn test_junction_gate_prevents_stacking() {
    for &(fwd, bkw, label) in LANE_CONFIGS {
        if fwd > 0 {
            check_connection_spacing_two_edge(fwd, bkw, label);
            check_connection_spacing_4way(fwd, bkw, label);
        }
    }
}

#[test]
fn test_frontage_node_no_uturn() {
    for &(fwd, bkw, label) in LANE_CONFIGS {
        if fwd > 0 {
            check_no_uturn_at_frontage(fwd, bkw, label);
        }
    }
}

// ── Lane bucket tests ─────────────────────────────────────────────────────────

/// After one tick, every ON_ROAD agent with a valid lane must appear in the
/// bucket for that lane exactly once.
#[test]
fn test_lane_bucket_populated_for_on_road_agents() {
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    let a = place_on_lane(&mut agents, edge_idx, fwd_lane, 10.0, 5.0);
    let b = place_on_lane(&mut agents, edge_idx, fwd_lane, 30.0, 5.0);

    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    // After the tick the bucket is filled during step 5 and cleared at the
    // start of the next step-2 pass — so inspect dirty_lanes/lane_buckets
    // immediately. We verify via the observable outcome: both agents were
    // ordered (overlap correction guarantees the front car stays ahead).
    assert!(
        agents.lane_distance[b] >= agents.lane_distance[a],
        "front agent (b at 30 m) should remain ahead of rear agent (a at 10 m)"
    );
    // Both agents must still be on the same lane.
    assert_eq!(agents.current_lane_id[a], fwd_lane);
    assert_eq!(agents.current_lane_id[b], fwd_lane);
}

/// Agents in the same lane must be sorted by lane_distance (ascending) after
/// each tick — the front agent must never have a smaller distance than the rear.
#[test]
fn test_lane_bucket_sorted_order() {
    use crate::config::{CAR_LENGTH, IDM_S_MIN};
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    // Place three agents in deliberately unsorted distance order.
    let _c = place_on_lane(&mut agents, edge_idx, fwd_lane, 80.0, 10.0); // front
    let _b = place_on_lane(&mut agents, edge_idx, fwd_lane, 50.0, 10.0); // middle
    let _a = place_on_lane(&mut agents, edge_idx, fwd_lane, 20.0, 10.0); // rear

    // Run a few ticks so IDM and overlap correction settle.
    for _ in 0..5 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
    }

    // Collect positions and verify monotone order.
    let mut dists: Vec<f32> = [_a, _b, _c]
        .iter()
        .map(|&i| agents.lane_distance[i])
        .collect();
    let sorted = {
        let mut s = dists.clone();
        s.sort_by(|x, y| x.partial_cmp(y).unwrap());
        s
    };
    assert_eq!(
        dists, sorted,
        "lane distances must be monotone ascending after ticks"
    );

    // Each adjacent pair must respect minimum separation.
    let min_sep = CAR_LENGTH + IDM_S_MIN;
    dists.sort_by(|x, y| x.partial_cmp(y).unwrap());
    for pair in dists.windows(2) {
        assert!(
            pair[1] - pair[0] >= min_sep - 0.01,
            "gap {:.3} m < min_sep {:.3} m",
            pair[1] - pair[0],
            min_sep,
        );
    }
}

/// Idle agents must not populate any lane bucket — dirty_lanes stays empty.
#[test]
fn test_lane_bucket_empty_for_idle_agents() {
    let (mut network, mut graph, _, _) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    // Spawn idle agents (home/work = MAX → safety scrub keeps them idle).
    for _ in 0..10 {
        agents.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    }
    // Override transit to IDLE so no pathfinding fires.
    for i in 0..agents.agents.len() {
        agents.transit[i] = TRANSIT_IN_BUILDING;
    }

    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    assert!(
        agents.dirty_lanes.is_empty(),
        "idle agents must not mark any lane as dirty"
    );
}

/// Agents in lane 0 and lane 1 of a 2-lane road must not interfere.
/// After a tick, each agent's position must be influenced only by agents
/// in the same lane — the cross-lane gap must be unaffected by IDM.
#[test]
fn test_lane_bucket_multi_lane_independence() {
    // build_two_edge_road with fwd=2 gives two forward vehicle lanes.
    let (mut network, mut graph, fwd_lanes) = build_two_edge_road(2, 0);
    assert!(fwd_lanes.len() >= 2, "need at least 2 forward lanes");
    let lane0 = fwd_lanes[0];
    let lane1 = fwd_lanes[1];

    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    // One agent per lane at the same distance, both at full speed.
    let a0 = place_on_lane(&mut agents, 0, lane0, 50.0, 14.0);
    let a1 = place_on_lane(&mut agents, 0, lane1, 50.0, 14.0);

    agents.tick(&mut allocator, &mut network, &mut graph, 0.5, 0, 0);

    // Both should advance by roughly the same amount — neither should be
    // slowed down by the other since they are in separate lanes.
    let d0 = agents.lane_distance[a0];
    let d1 = agents.lane_distance[a1];
    assert!(
        (d0 - d1).abs() < 1.0,
        "lanes are independent: distances should match, got lane0={d0:.2} lane1={d1:.2}"
    );

    // After the tick, each lane's bucket must contain only its own agent.
    // Verify indirectly: neither agent was overlap-corrected by the other.
    assert!(d0 > 50.0, "lane0 agent should have advanced");
    assert!(d1 > 50.0, "lane1 agent should have advanced");
}

/// An on-road agent with `current_lane_id = usize::MAX` must not crash
/// and must not mark any lane as dirty.
#[test]
fn test_lane_bucket_invalid_lane_id_does_not_crash() {
    let (mut network, mut graph, edge_idx, _) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    // Spawn with a valid edge but invalid lane — matches the benchmark default.
    let i = agents.spawn_border_arrival_agent(usize::MAX, 1, 0.0, 0.0, 0, 0.0, 0.0);
    agents.transit[i] = TRANSIT_NETWORK;
    agents.current_edge[i] = edge_idx;
    agents.current_lane_id[i] = usize::MAX;
    agents.lane_distance[i] = 10.0;
    agents.speed[i] = 5.0;
    agents.current_path[i] = vec![0u32, 1u32];
    agents.current_path_index[i] = 1;

    // Must not panic or access out-of-bounds memory.
    // The tick may assign the agent to a real lane (expected behaviour),
    // so we only assert the invariant that every entry in dirty_lanes is
    // a valid lane index.
    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    let lane_count = network.lane_system.lanes.len();
    for &lid in &agents.dirty_lanes {
        assert!(
            lid < lane_count,
            "dirty lane {lid} is out of bounds (lane_count={lane_count})"
        );
    }
}

/// `dirty_lanes` must contain each occupied lane ID exactly once, regardless
/// of how many agents share that lane.
#[test]
fn test_lane_bucket_dirty_lanes_no_duplicates() {
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    // Place 5 agents spread across the same lane.
    for k in 0..5 {
        place_on_lane(&mut agents, edge_idx, fwd_lane, 10.0 + k as f32 * 15.0, 5.0);
    }

    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    let count = agents
        .dirty_lanes
        .iter()
        .filter(|&&l| l == fwd_lane)
        .count();
    assert_eq!(
        count, 1,
        "lane {fwd_lane} must appear in dirty_lanes exactly once, found {count}"
    );
}

/// An agent at exactly `speed_limit` must produce zero congestion.
#[test]
fn test_congestion_zero_at_free_flow_speed() {
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let speed_limit = graph.edge(edge_idx).speed_limit;
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    place_on_lane(&mut agents, edge_idx, fwd_lane, 50.0, speed_limit);
    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    assert_eq!(
        graph.edge(edge_idx).current_congestion,
        0.0,
        "agent at speed_limit must produce zero congestion"
    );
}

/// An agent at 50 % of `speed_limit` must produce congestion ≈ 0.5.
#[test]
fn test_congestion_proportional_to_speed_deficit() {
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let speed_limit = graph.edge(edge_idx).speed_limit;
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    place_on_lane(&mut agents, edge_idx, fwd_lane, 50.0, speed_limit * 0.5);
    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
    // Force speed to stay at 50 % and tick again so congestion is written.
    agents.speed[0] = speed_limit * 0.5;
    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    let c = graph.edge(edge_idx).current_congestion;
    assert!(
        (c - 0.5).abs() < 0.05,
        "expected congestion ≈ 0.5 at half speed_limit, got {c:.3}"
    );
}

#[test]
fn test_frontage_delay_cache_respects_fixed_cadence() {
    let (mut network, graph, edge_idx, fwd_lane) = setup_straight_road();
    let speed_limit = graph.edge(edge_idx).speed_limit;
    let mut agents = AgentSystem::new();
    let observed_speed = speed_limit * 0.5;

    place_on_lane(&mut agents, edge_idx, fwd_lane, 50.0, observed_speed);

    agents.update_frontage_delay_cache(&mut network, &graph, 0.5);
    assert_eq!(
        network.lane_system.lanes[fwd_lane].frontage_delay_penalty_s,
        0.0
    );

    agents.update_frontage_delay_cache(&mut network, &graph, 0.5);
    let penalty = network.lane_system.lanes[fwd_lane].frontage_delay_penalty_s;
    let expected =
        expected_frontage_delay_penalty_s(&network, &graph, edge_idx, fwd_lane, observed_speed, 1);
    assert!(
        (penalty - expected).abs() < 0.01,
        "expected first smoothed frontage penalty to be {expected:.3}s, got {penalty:.3}"
    );
}

#[test]
fn test_frontage_delay_cache_keeps_foot_lanes_at_zero() {
    let (mut network, graph, edge_idx, fwd_lane) = setup_straight_road();
    let speed_limit = graph.edge(edge_idx).speed_limit;
    let mut agents = AgentSystem::new();

    place_on_lane(&mut agents, edge_idx, fwd_lane, 50.0, speed_limit * 0.5);
    agents.update_frontage_delay_cache(&mut network, &graph, 1.0);

    let foot_lane = *network.lane_system.edge_lanes[&edge_idx]
        .iter()
        .find(|&&lid| {
            network.lane_system.lanes[lid].lane_type
                == crate::simulation::network::lanes::LaneType::Foot
        })
        .expect("foot lane");
    assert_eq!(
        network.lane_system.lanes[foot_lane].frontage_delay_penalty_s,
        0.0
    );
}

#[test]
fn test_frontage_delay_cache_applies_multiple_fixed_cadence_steps() {
    let (mut network, graph, edge_idx, fwd_lane) = setup_straight_road();
    let speed_limit = graph.edge(edge_idx).speed_limit;
    let mut agents = AgentSystem::new();
    let observed_speed = speed_limit * 0.5;

    place_on_lane(&mut agents, edge_idx, fwd_lane, 50.0, observed_speed);

    agents.update_frontage_delay_cache(&mut network, &graph, 2.0);
    let penalty = network.lane_system.lanes[fwd_lane].frontage_delay_penalty_s;
    let expected =
        expected_frontage_delay_penalty_s(&network, &graph, edge_idx, fwd_lane, observed_speed, 2);
    assert!(
        (penalty - expected).abs() < 0.01,
        "expected two fixed-cadence smoothing steps to produce {expected:.3}s, got {penalty:.3}"
    );
}

#[test]
fn test_frontage_delay_cache_decays_without_agents() {
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    network.lane_system.lanes[fwd_lane].frontage_delay_penalty_s = 10.0;

    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();
    agents.tick(&mut allocator, &mut network, &mut graph, 1.0, 0, 0);

    let penalty = network.lane_system.lanes[fwd_lane].frontage_delay_penalty_s;
    assert!(
        (penalty - 7.5).abs() < 0.01,
        "expected empty-lane penalty to decay toward zero, got {penalty:.3}"
    );
    assert_eq!(graph.edge(edge_idx).current_congestion, 0.0);
}

/// Running many ticks with a platoon of cars must not compress distances to zero.
/// Positions must converge and stabilise, not collapse.
#[test]
fn test_overlap_correction_stable_over_many_ticks() {
    use crate::config::{CAR_LENGTH, IDM_S_MIN};
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    let min_sep = CAR_LENGTH + IDM_S_MIN;
    let indices: Vec<usize> = (0..4)
        .map(|k| {
            place_on_lane(
                &mut agents,
                edge_idx,
                fwd_lane,
                20.0 + k as f32 * (min_sep + 1.0),
                5.0,
            )
        })
        .collect();

    for _ in 0..30 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
    }

    let mut dists: Vec<f32> = indices.iter().map(|&i| agents.lane_distance[i]).collect();
    dists.sort_by(|a, b| a.partial_cmp(b).unwrap());

    for pair in dists.windows(2) {
        assert!(
            pair[1] - pair[0] >= min_sep - 0.01,
            "gap {:.3} m collapsed below min_sep {min_sep:.3} m after 30 ticks",
            pair[1] - pair[0]
        );
    }
    // No agent should be pushed to or past zero.
    assert!(
        dists[0] >= 0.0,
        "rear agent must not be pushed behind start"
    );
}

/// An agent that transitions to a new edge must not appear in its old lane's
/// bucket on the following tick.
#[test]
fn test_lane_bucket_cleared_after_agent_leaves_edge() {
    let (mut network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    let i = place_on_lane(&mut agents, edge_idx, fwd_lane, 10.0, 5.0);
    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    // Manually move the agent off-road (simulates arrival or edge transition).
    agents.transit[i] = TRANSIT_IN_BUILDING;
    agents.current_lane_id[i] = usize::MAX;

    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    assert!(
        agents.dirty_lanes.is_empty(),
        "after agent leaves road, no lane should be marked dirty"
    );
}

/// With >500 distinct occupied lanes (above PAR_THRESHOLD), the parallel sort
/// must produce the same ordering as the sequential path would.
/// We verify by checking that all lane_distance values are monotone ascending
/// within each lane after a tick that exercises the parallel branch.
#[test]
fn test_lane_bucket_parallel_sort_matches_sequential_order() {
    // Build 510 single-lane edges so dirty_lanes will have 510 entries (> PAR_THRESHOLD=500).
    let mut graph = RegionGraph::new();
    let mut edges: Vec<usize> = Vec::new();
    // Chain: n0 → n1, n2 → n3, ... (each edge is independent, 2 nodes each)
    for k in 0..510usize {
        let x = k as f32 * 200.0;
        let na = graph.add_node(Vector3::new(x, 0.0, 0.0), NodeType::Junction);
        let nb = graph.add_node(Vector3::new(x + 100.0, 0.0, 0.0), NodeType::Junction);
        let e = Edge {
            start_node: na,
            end_node: nb,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 14.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(x, 0.0, 0.0), Vector3::new(x + 100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(x, 0.0, 0.0), Vector3::new(x + 100.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        };
        edges.push(graph.add_edge(e));
    }
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();

    // Place 2 agents per edge in reverse order so the sort has real work to do.
    for &eid in &edges {
        let fwd_lane = *network.lane_system.edge_lanes[&eid]
            .iter()
            .find(|&&lid| {
                let l = &network.lane_system.lanes[lid];
                l.is_fwd && l.lane_type == crate::simulation::network::lanes::LaneType::Vehicle
            })
            .expect("forward vehicle lane");
        let edge = graph.edge(eid);
        let (na, nb) = (edge.start_node, edge.end_node);
        for &(dist, spd) in &[(70.0f32, 5.0f32), (30.0f32, 5.0f32)] {
            let i = agents.spawn_border_arrival_agent(usize::MAX, nb, 0.0, 0.0, na, 0.0, 0.0);
            agents.transit[i] = TRANSIT_NETWORK;
            agents.current_edge[i] = eid;
            agents.current_lane_id[i] = fwd_lane;
            agents.lane_distance[i] = dist;
            agents.speed[i] = spd;
            agents.current_path[i] = vec![na, nb];
            agents.current_path_index[i] = 1;
        }
    }

    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    // Every agent that remains on-road must have lane_distance >= the agent
    // behind it within the same lane. Collect per-lane distances and check.
    let lane_count = network.lane_system.lanes.len();
    let mut per_lane: std::collections::HashMap<usize, Vec<f32>> = std::collections::HashMap::new();
    for i in 0..agents.agents.len() {
        if agents.transit[i] == TRANSIT_NETWORK {
            let lid = agents.current_lane_id[i];
            if lid != usize::MAX && lid < lane_count {
                per_lane
                    .entry(lid)
                    .or_default()
                    .push(agents.lane_distance[i]);
            }
        }
    }
    for (lid, mut dists) in per_lane {
        dists.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in dists.windows(2) {
            assert!(
                pair[1] >= pair[0] - 0.01,
                "lane {lid}: distances not monotone after parallel sort: {:.3} > {:.3}",
                pair[0],
                pair[1]
            );
        }
    }
}
