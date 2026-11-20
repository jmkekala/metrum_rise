// SPDX-License-Identifier: GPL-3.0-or-later

//! Walkers on one sidewalk overtake and make room for each other (`walker_crowd`).

use super::support::*;
use super::*;
use crate::simulation::economy::agents::walker_gait::walker_pace;
use crate::simulation::network::lanes::LaneType;

struct Sidewalk {
    network: TransitNetwork,
    graph: RegionGraph,
    allocator: BuildingAllocator,
    lane: usize,
    opposing: usize,
}

fn sidewalk() -> Sidewalk {
    let (network, graph, _, _) = setup_straight_road();
    let lanes = &network.lane_system;
    let lane = lanes.edge_lanes[&0]
        .iter()
        .copied()
        .find(|&id| lanes.lanes[id].lane_type == LaneType::Foot && lanes.lanes[id].is_fwd)
        .expect("a forward sidewalk lane");
    let opposing = lanes.opposing_foot_lane(lane).expect("its opposing lane");
    assert_eq!(lanes.opposing_foot_lane(opposing), Some(lane));
    // Everyone walks to a door on this sidewalk, far beyond where the tests look.
    let side = lanes.lanes[lane].lane_idx.signum();
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(create_test_building(0, side));
    allocator.entrances.push(BuildingEntrance {
        edge_idx: 0,
        side,
        entrance_s_m: 450.0,
        foot_lane_fwd: lane,
        foot_lane_bkw: opposing,
        ..BuildingEntrance::default()
    });
    Sidewalk {
        network,
        graph,
        allocator,
        lane,
        opposing,
    }
}

// The first schedule seed whose walking pace `wanted` accepts.
fn seed_with(wanted: impl Fn(f32) -> bool) -> u32 {
    (0..).find(|&seed| wanted(walker_pace(seed))).unwrap()
}

// A walker on `lane` at `d`, walking to the door 380 m on along it.
fn walker(agents: &mut AgentSystem, lane: usize, d: f32, seed: u32, fwd: bool) -> usize {
    let (from, to) = if fwd { (0, 1) } else { (1, 0) };
    let idx = place_on_lane(agents, 0, lane, d, 0.0);
    agents.transit_mode[idx] = MODE_WALK;
    agents.access_flags[idx] = ACCESS_PLAN_VALID;
    agents.current_node[idx] = from;
    agents.current_path[idx] = vec![from, to];
    agents.planned_detach_node[idx] = to;
    agents.planned_detach_lane_id[idx] = lane as u32;
    agents.planned_detach_lane_d[idx] = d + 380.0;
    agents.target_building[idx] = 0;
    agents.schedule_seed[idx] = seed;
    idx
}

fn tick(case: &mut Sidewalk, agents: &mut AgentSystem) {
    let clock = test_clock(0, 0);
    let allocator = &case.allocator;
    agents.tick(allocator, &mut case.network, &mut case.graph, 0.1, &clock);
    for (idx, flags) in agents.access_flags.iter().enumerate() {
        assert_ne!(flags & ACCESS_PLAN_VALID, 0, "walker {idx} lost its plan");
    }
}

#[test]
fn a_faster_walker_overtakes_on_a_clear_sidewalk() {
    let mut case = sidewalk();
    let mut agents = AgentSystem::new();
    let slow = walker(&mut agents, case.lane, 102.0, seed_with(|p| p < 0.9), true);
    let fast = walker(&mut agents, case.lane, 100.0, seed_with(|p| p > 1.1), true);
    for _ in 0..200 {
        tick(&mut case, &mut agents);
    }
    assert_eq!(agents.current_lane_id[fast], case.lane);
    assert!(
        agents.lane_distance[fast] > agents.lane_distance[slow] + 2.0,
        "fast at {} passed slow at {}",
        agents.lane_distance[fast],
        agents.lane_distance[slow]
    );
}

#[test]
fn a_faster_walker_follows_while_someone_comes_the_other_way_then_overtakes() {
    let mut case = sidewalk();
    let length = case.network.lane_system.lanes[case.opposing].length;
    let mut agents = AgentSystem::new();
    let slow = walker(&mut agents, case.lane, 102.0, seed_with(|p| p < 0.9), true);
    // Close enough behind to overtake in about a second and a half.
    let fast = walker(&mut agents, case.lane, 101.6, seed_with(|p| p > 1.1), true);
    // Coming the other way, about 4 m ahead of the pair: past the fast walker in about two.
    let oncoming = walker(&mut agents, case.opposing, length - 105.9, 0, false);
    // The door is 380 m on for the oncoming walker too: on its lane, short of the road end.
    agents.planned_detach_lane_d[oncoming] = length - 20.0;
    let mut met = false;
    for _ in 0..300 {
        tick(&mut case, &mut agents);
        let toward = length - agents.lane_distance[oncoming];
        if toward + 1.0 >= agents.lane_distance[fast] {
            // Until the oncoming walker is past the fast one, the fast one keeps behind.
            assert!(
                agents.lane_distance[fast] < agents.lane_distance[slow],
                "fast at {} overtook slow at {} with oncoming at {toward}",
                agents.lane_distance[fast],
                agents.lane_distance[slow]
            );
        } else {
            met = true;
        }
    }
    assert!(met, "the oncoming walker went past");
    assert!(
        agents.lane_distance[fast] > agents.lane_distance[slow],
        "fast overtook once the sidewalk cleared"
    );
}
