// SPDX-License-Identifier: GPL-2.0-only

//! Advance turn-lane choice and physical missed-turn recovery through generated connectors.

use super::support::*;
use super::*;
use crate::simulation::economy::agents::tick::{
    EntryLanes, lane_hosts_turn_preparation, preferred_entry_lanes,
};

#[test]
fn turn_lane_choice_reaches_left_straight_and_right_before_the_junction() {
    turn_lane_choice_ticks();
}

// Runs every `(lane count, destination)` case of the advance lane choice and returns, per case,
// the tick the car reached its turn lane on and its distance along it then.
fn turn_lane_choice_ticks() -> Vec<(u8, u32, usize, u32)> {
    let mut outcomes = Vec::new();
    for count in 2..=4 {
        for (destination, expected_slot) in [(3, 0), (2, 0), (4, usize::from(count - 1))] {
            let (mut network, mut graph, arms) = build_4way_junction(count, count);
            // Every approach lane goes straight (user decision: permissive default lane use), so a
            // straight car starts on, and keeps, the inner lane.
            let initial = if destination == 2 {
                0
            } else if expected_slot == 0 {
                usize::from(count - 1)
            } else {
                0
            };
            let mut agents = AgentSystem::new();
            let lane = arms[0][initial];
            let i = place_on_lane(&mut agents, 0, lane, 1.0, 5.0);
            agents.current_node[i] = 1;
            agents.current_path[i] = vec![1, 0, destination];
            agents.overtake_cooldown_s[i] = f32::MAX;
            let mut reached = false;
            for tick in 0..250 {
                agents.tick(
                    &BuildingAllocator::new(),
                    &mut network,
                    &mut graph,
                    0.1,
                    &test_clock(0, 0),
                );
                let lane_id = agents.current_lane_id[i];
                if lane_id == arms[0][expected_slot] {
                    reached = true;
                    assert!(
                        agents.lane_distance[i] < network.lane_system.lanes[lane_id].length - 12.0
                    );
                    outcomes.push((count, destination, tick, agents.lane_distance[i].to_bits()));
                    break;
                }
                assert_eq!(
                    agents.current_edge[i], 0,
                    "missed preparation: count={count}, destination={destination}"
                );
            }
            assert!(reached, "count={count}, destination={destination}");
        }
    }
    outcomes
}

#[test]
fn missed_turn_follows_a_real_connector_then_replans_and_reaches_the_door() {
    missed_turn_arrival_tick();
}

// Drives the missed-turn case to the door, asserting a physical lane at every tick, and returns
// the arrival tick.
fn missed_turn_arrival_tick() -> usize {
    let (mut network, mut graph, arms) = build_4way_junction(2, 2);
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(create_test_building(2, -1));
    let destination = bkw_vehicle_lanes(&network, 2)[1];
    allocator.entrances.push(BuildingEntrance {
        edge_idx: 2,
        side: -1,
        entrance_s_m: 70.0,
        door_pos: Vector2::new(5.0, -70.0),
        curb_pos: Vector2::new(4.0, -70.0),
        car_lane_bkw: destination,
        ..BuildingEntrance::default()
    });
    let mut agents = AgentSystem::new();
    let source = arms[0][1]; // straight/right: cannot take the requested left turn.
    let end = network.lane_system.lanes[source].length;
    let i = place_on_lane(&mut agents, 0, source, end - 2.0, 5.0);
    agents.current_node[i] = 1;
    agents.current_path[i] = vec![1, 0, 3];
    agents.target_building[i] = 0;
    agents.access_flags[i] = ACCESS_PLAN_VALID;
    agents.planned_detach_node[i] = 0;
    agents.planned_detach_lane_id[i] = destination as u32;
    agents.planned_detach_lane_d[i] = 70.0;
    agents.overtake_cooldown_s[i] = f32::MAX;
    // A blocked adjacent lane ensures no last-instant lateral acquisition is possible.
    let blocker = place_on_lane(&mut agents, 0, arms[0][0], end - 2.0, 0.0);
    agents.current_node[blocker] = 1;
    agents.current_path[blocker] = vec![1, 0, 3];
    let mut previous = source;
    let mut saw_onward = false;
    let mut arrived = None;
    for tick in 0..3000 {
        agents.tick(&allocator, &mut network, &mut graph, 0.1, &test_clock(0, 0));
        let now = agents.current_lane_id[i];
        if now != previous && now != usize::MAX {
            let before = &network.lane_system.lanes[previous];
            let after = &network.lane_system.lanes[now];
            if before.edge_id != after.edge_id {
                let entered_then_changed = agents.lane_change_from_lane_id[i] != u32::MAX
                    && before
                        .next_lanes
                        .contains(&(agents.lane_change_from_lane_id[i] as usize));
                assert!(
                    before.next_lanes.contains(&now) || entered_then_changed,
                    "tick {tick}: nonphysical handoff {previous}->{now}"
                );
            }
            if after.edge_id == usize::MAX && previous == source {
                let outgoing = &network.lane_system.lanes[after.next_lanes[0]];
                assert_ne!(outgoing.edge_id, 2, "cannot make a forbidden left turn");
                saw_onward = true;
            }
        }
        if agents.transit[i] == TRANSIT_ACCESS_INGRESS || agents.transit[i] == TRANSIT_IN_BUILDING {
            arrived = Some(tick);
            break;
        }
        assert_ne!(now, usize::MAX, "tick {tick}: car lost its physical lane");
        previous = now;
    }
    assert!(saw_onward);
    arrived.unwrap_or_else(|| {
        panic!(
            "car deadlocked at lane {} distance {} with path {:?}",
            agents.current_lane_id[i], agents.lane_distance[i], agents.current_path[i]
        )
    })
}

#[test]
fn freight_missed_turn_replans_on_its_outgoing_road_and_reaches_the_border() {
    freight_missed_turn_arrival_tick();
}

// Drives the freight missed-turn case to its border node and returns the arrival tick.
fn freight_missed_turn_arrival_tick() -> usize {
    let (mut network, mut graph, arms) = build_4way_junction(2, 2);
    let mut agents = AgentSystem::new();
    let source = arms[0][1];
    let end = network.lane_system.lanes[source].length;
    let i = place_on_lane(&mut agents, 0, source, end - 2.0, 5.0);
    agents.current_node[i] = 1;
    agents.current_path[i] = vec![1, 0, 3];
    agents.access_flags[i] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
    agents.freight_target_border_node[i] = 3;
    let mut previous = source;
    let mut arrived = None;
    for tick in 0..2000 {
        agents.tick(
            &BuildingAllocator::new(),
            &mut network,
            &mut graph,
            0.1,
            &test_clock(0, 0),
        );
        let now = agents.current_lane_id[i];
        if now == usize::MAX {
            assert_eq!(agents.current_node[i], 3);
            arrived = Some(tick);
            break;
        }
        if now != previous {
            let from = &network.lane_system.lanes[previous];
            let to = &network.lane_system.lanes[now];
            if from.edge_id != to.edge_id {
                let changing_from = agents.lane_change_from_lane_id[i] as usize;
                assert!(from.next_lanes.contains(&now) || from.next_lanes.contains(&changing_from));
            }
        }
        previous = now;
    }
    arrived.expect("freight car never reached its border node")
}

// Determinism: the three runs' ticks and positions (lane distance bits), as recorded. Passing under
// `RAYON_NUM_THREADS=1` and `=8` alike shows the advance lane choice and the missed-turn recovery
// do not depend on the worker count. The 4-lane left and right cases were re-pinned for the solid
// approach zone (tick 37 -> 34): their three-step preparation starts inside the window, and the
// zone reserves 8 m more of the turn budget, so each step is shorter and the last ends earlier.
// The two arrivals were re-pinned for the entry-lane choice (`TRAFFIC-23`, 279 -> 286 and
// 330 -> 337): after the missed turn the car's dead-end U-turn and the entries after it take the
// lane its next turn needs instead of the shortest connector (with that preference off at the lane
// end: 279 and 329). The straight cases keep the inner lane they start on (user decision:
// permissive default lane use, every lane goes straight), at the ticks recorded before.
#[test]
fn turn_lane_runs_match_their_recorded_ticks() {
    let runs = (
        turn_lane_choice_ticks(),
        missed_turn_arrival_tick(),
        freight_missed_turn_arrival_tick(),
    );
    let choice = vec![
        (2, 3, 0, 1_070_042_648),
        (2, 2, 0, 1_070_042_648),
        (2, 4, 0, 1_070_042_648),
        (3, 3, 20, 1_102_347_023),
        (3, 2, 0, 1_070_042_648),
        (3, 4, 20, 1_102_347_023),
        (4, 3, 34, 1_109_872_857),
        (4, 2, 0, 1_070_042_648),
        (4, 4, 34, 1_109_872_857),
    ];
    assert_eq!(runs, (choice, 286, 337));
}

// TRAFFIC-23 (a): the entry preference walks the route straight through equal-count junctions
// and stops at its next turn (the turn's lane group), at the planned detach lane on the road, at a
// whitelisted junction or a one-lane road (no preference).
#[test]
fn preferred_entry_lanes_follow_the_route_to_its_next_turn() {
    let mut city = build_short_block_street(50.0, [(100.0, 100.0); 3]);
    let [j0, j1, j2] = city.junctions;
    let far = |city: &ShortBlockStreet, edge: usize| city.graph.edge(edge).end_node;
    let (west2, east2) = (far(&city, city.arms[2].0), far(&city, city.arms[2].1));
    let south = far(&city, city.street[3]);
    let prefer = |city: &ShortBlockStreet, path: &[u32], flags: u8, detach_lane: usize| {
        preferred_entry_lanes(
            city.street[1],
            j0,
            path,
            2,
            flags,
            j1,
            detach_lane,
            &city.network,
            &city.graph,
        )
    };
    let only = |lanes: Option<EntryLanes>, lane_idx: i8| {
        let lanes = lanes.expect("a preference");
        assert!(lanes.contains(lane_idx) && !lanes.contains(1 - lane_idx));
    };
    // Heading south, east is the left turn (inner lane) and west the right turn (outer lane).
    only(prefer(&city, &[j0, j1, j2, east2], 0, usize::MAX), 0);
    only(prefer(&city, &[j0, j1, j2, west2], 0, usize::MAX), 1);
    assert_eq!(prefer(&city, &[j0, j1, j2, south], 0, usize::MAX), None);
    let detach = vehicle_lane(&city.network, city.street[1], 1);
    only(prefer(&city, &[j0, j1], ACCESS_PLAN_VALID, detach), 1);
    let east1 = far(&city, city.arms[1].1);
    let one_lane = preferred_entry_lanes(
        city.arms[1].1,
        j1,
        &[j1, east1],
        2,
        0,
        u32::MAX,
        usize::MAX,
        &city.network,
        &city.graph,
    );
    assert_eq!(one_lane, None);
    city.graph
        .add_lane_connection(j2, city.street[2], 0, city.arms[2].1, -1);
    assert_eq!(prefer(&city, &[j0, j1, j2, east2], 0, usize::MAX), None);
}

// TRAFFIC-23 (b): a two-lane approach hosts a turn preparation only when the 18 m change and the
// end buffers fit before its solid zone (more than 38.25 m); a one-lane road always does.
#[test]
fn only_a_lane_with_room_for_a_change_hosts_a_turn_preparation() {
    for (link, hosts) in [(50.0, false), (58.0, false), (60.0, true), (400.0, true)] {
        let city = build_short_block_street(link, [(100.0, 100.0); 3]);
        let lane = vehicle_lane(&city.network, city.street[1], 1);
        let length = city.network.lane_system.lanes[lane].length;
        assert!(city.network.lane_system.needs_turn_preparation(lane));
        assert_eq!(length > 38.25, hosts, "link {link} m: lane {length} m");
        assert_eq!(lane_hosts_turn_preparation(lane, &city.network), hosts);
        let arm = vehicle_lane(&city.network, city.arms[1].0, 0);
        assert!(lane_hosts_turn_preparation(arm, &city.network));
    }
}

// TRAFFIC-23: a car turning right from a one-lane cross street onto a 2+2 street of 30 m links
// and left off it one junction later enters the inner lane at once (a single-lane group fans to
// every lane) and makes its left turn without a miss.
#[test]
fn right_then_left_across_a_short_block() {
    let mut city = build_short_block_street(50.0, [(100.0, 100.0); 3]);
    let [j0, j1, _] = city.junctions;
    let west0 = city.graph.edge(city.arms[0].0).end_node;
    let east1 = city.graph.edge(city.arms[1].1).end_node;
    let source = vehicle_lane(&city.network, city.arms[0].0, -1);
    let length = city.network.lane_system.lanes[source].length;
    let mut agents = AgentSystem::new();
    let i = place_on_lane(&mut agents, city.arms[0].0, source, length - 30.0, 5.0);
    agents.current_node[i] = west0;
    agents.current_path[i] = vec![west0, j0, j1, east1];
    agents.access_flags[i] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
    agents.freight_target_border_node[i] = east1;
    agents.overtake_cooldown_s[i] = f32::MAX;
    let mut entered = None;
    for _ in 0..2000 {
        agents.tick(
            &BuildingAllocator::new(),
            &mut city.network,
            &mut city.graph,
            0.1,
            &test_clock(0, 0),
        );
        let lane_id = agents.current_lane_id[i];
        if lane_id == usize::MAX {
            assert_eq!(agents.current_node[i], east1);
            break;
        }
        let lane = &city.network.lane_system.lanes[lane_id];
        assert_ne!(lane.edge_id, city.street[2], "missed the left turn");
        if lane.edge_id == city.street[1] && entered.is_none() {
            entered = Some(lane.lane_idx);
        }
    }
    assert_eq!(entered, Some(0), "entered the street's inner lane");
    assert_eq!(
        agents.current_lane_id[i],
        usize::MAX,
        "never reached its border node"
    );
}

// TRAFFIC-23 (b): a car on the outer lane of a 30 m link misses its left turn at a T, goes on
// along the street, and its replan leaves the next junction only on a road its lane reaches:
// not the short left arm the unrestricted route takes, but straight on (and back round).
#[test]
fn missed_turn_on_a_short_street_turns_off_and_replans() {
    let mut city = build_short_block_street(50.0, [(100.0, 100.0), (0.0, 100.0), (300.0, 60.0)]);
    let [j0, j1, _] = city.junctions;
    let east1 = city.graph.edge(city.arms[1].1).end_node;
    let source = vehicle_lane(&city.network, city.street[1], 1);
    let length = city.network.lane_system.lanes[source].length;
    assert!(!lane_hosts_turn_preparation(source, &city.network));
    let mut agents = AgentSystem::new();
    let i = place_on_lane(&mut agents, city.street[1], source, length - 2.0, 5.0);
    agents.current_node[i] = j0;
    agents.current_path[i] = vec![j0, j1, east1];
    agents.access_flags[i] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
    agents.freight_target_border_node[i] = east1;
    agents.overtake_cooldown_s[i] = f32::MAX;
    let mut missed = false;
    for _ in 0..4000 {
        agents.tick(
            &BuildingAllocator::new(),
            &mut city.network,
            &mut city.graph,
            0.1,
            &test_clock(0, 0),
        );
        let lane_id = agents.current_lane_id[i];
        if lane_id == usize::MAX {
            assert_eq!(agents.current_node[i], east1);
            break;
        }
        let edge = city.network.lane_system.lanes[lane_id].edge_id;
        missed |= edge == city.street[2];
        assert_ne!(
            edge, city.arms[2].1,
            "the replan left on a road the lane does not reach"
        );
        assert_ne!(edge, city.arms[2].0, "the replan took the long way round");
    }
    assert!(missed, "the car could not have made the left turn");
    assert_eq!(
        agents.current_lane_id[i],
        usize::MAX,
        "never reached its border node"
    );
}
