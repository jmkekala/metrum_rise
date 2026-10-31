// SPDX-License-Identifier: GPL-2.0-only

//! `TRAFFIC-14` ring fixture: one block of the populated-city grid as a closed ring of four
//! one-way lanes (380 m, 30 m, 380 m, 30 m) joined by four 20 m corner connectors, the way the
//! 100k city's locked block is built. Trucks circulate on looping routes while more wait at
//! every corner to start there (node-origin entry, `TRAFFIC-07`). A variant adds a 390 m feeder
//! road into corner 0 whose queue merges into the ring through its own connector, as through
//! traffic from a crossing street does in the city. Isolated subsystem setup: no routing,
//! buildings or economy.

use super::*;
use crate::config::{CAR_LENGTH, IDM_S_MIN};
use crate::simulation::economy::agents::tick::{GridlockChecker, GridlockReport};
use crate::simulation::economy::agents::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_INTERSECTION,
    TRANSIT_NETWORK,
};
use crate::simulation::network::graph::Edge;
use crate::simulation::network::lanes::{Lane, LaneSuccessors};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::save::gridlock::round_trip_through_file;
use godot::prelude::Vector3;

// Block corners, counter-clockwise; edge k runs from corner k to corner k + 1.
const CORNERS: [(f32, f32); 4] = [(0.0, 0.0), (400.0, 0.0), (400.0, 50.0), (0.0, 50.0)];
// Junction mouths: every road lane stops this far short of both of its nodes.
const MOUTH_M: f32 = 10.0;
// Laps in every route: far more than the test drives.
const LAPS: usize = 40;

/// The ring: agents, lanes and graph. Road lanes are 0..4, connector k (lane 4 + k) turns at
/// corner k + 1 from road lane k onto road lane k + 1; with a feeder, lane 8 is the feeder road
/// (node 4 to corner 0) and lane 9 its connector onto road lane 0.
pub(super) struct Ring {
    pub agents: AgentSystem,
    pub network: TransitNetwork,
    pub graph: RegionGraph,
}

fn corner(k: usize) -> Vector3 {
    let (x, z) = CORNERS[k % 4];
    Vector3::new(x, 0.0, z)
}

// The route of a car heading for corner `next`: round the block, `LAPS` times.
fn looping_path(next: usize) -> Vec<u32> {
    (0..LAPS * 4).map(|step| ((next + step) % 4) as u32).collect()
}

fn lane(edge_id: usize, node_id: usize, geometry: Vec<Vector3>, next: Option<usize>) -> Lane {
    let mut cum_dist = vec![0.0];
    for pair in geometry.windows(2) {
        cum_dist.push(cum_dist.last().unwrap() + pair[0].distance_to(pair[1]));
    }
    Lane {
        edge_id,
        node_id,
        is_fwd: true,
        length: *cum_dist.last().unwrap(),
        geometry,
        cum_dist,
        next_lanes: next.map(LaneSuccessors::one).unwrap_or_default(),
        ..Default::default()
    }
}

// A one-way single-lane road edge from node `from` at `a` to node `to` at `b`.
fn road_edge(graph: &mut RegionGraph, from: u32, to: u32, a: Vector3, b: Vector3) -> usize {
    let length = a.distance_to(b);
    graph.add_edge(Edge {
        start_node: from,
        end_node: to,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 0,
        speed_limit: 13.89,
        base_cost: length / 13.89,
        physical_length: length,
        start_clip: MOUTH_M,
        end_clip: MOUTH_M,
        geometry: vec![a, b],
        physical_geometry: vec![a, b],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    })
}

/// Builds the ring with `spacing_m` between circulating trucks (0 for none), `waiting` trucks
/// held at every corner for their first lane and `feeder` trucks queued on the feeder road.
pub(super) fn ring(spacing_m: f32, waiting: usize, feeder: usize) -> Ring {
    let mut agents = AgentSystem::new();
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    for k in 0..4 {
        graph.add_node(corner(k), NodeType::Junction);
    }
    for k in 0..4 {
        road_edge(
            &mut graph,
            k as u32,
            ((k + 1) % 4) as u32,
            corner(k),
            corner(k + 1),
        );
    }
    for k in 0..4 {
        let (a, b) = (corner(k), corner(k + 1));
        let dir = (b - a).normalized();
        let geometry = vec![a + dir * MOUTH_M, b - dir * MOUTH_M];
        network
            .lane_system
            .lanes
            .push(lane(k, usize::MAX, geometry, Some(4 + k)));
        network.lane_system.edge_lanes.insert(k, vec![k]);
    }
    for k in 0..4 {
        // A right-angle turn round corner k + 1, from road lane k's end to road lane k + 1's start.
        let (a, b, c) = (corner(k), corner(k + 1), corner(k + 2));
        let into = (b - a).normalized();
        let out = (c - b).normalized();
        let geometry = vec![b - into * MOUTH_M, b, b + out * MOUTH_M];
        network
            .lane_system
            .lanes
            .push(lane(usize::MAX, (k + 1) % 4, geometry, Some((k + 1) % 4)));
    }
    let truck = |agents: &mut AgentSystem, next: usize| {
        let i = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.transit_mode[i] = MODE_CAR;
        agents.transit[i] = TRANSIT_NETWORK;
        agents.current_path[i] = looping_path(next);
        agents.current_path_index[i] = 0;
        agents.access_flags[i] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
        agents.freight_target_border_node[i] = *agents.current_path[i].last().unwrap();
        agents.freight_shipment_id[i] = 1_000 + i as u64;
        i
    };
    if spacing_m > 0.0 {
        for k in 0..4 {
            let length = network.lane_system.lanes[k].length;
            let mut d = 1.0;
            while d < length {
                let i = truck(&mut agents, (k + 1) % 4);
                agents.current_lane_id[i] = k;
                agents.current_edge[i] = k;
                agents.current_node[i] = k as u32;
                agents.lane_distance[i] = d;
                d += spacing_m;
            }
        }
    }
    for k in 0..4 {
        for _ in 0..waiting {
            let i = truck(&mut agents, (k + 1) % 4);
            agents.current_node[i] = k as u32;
        }
    }
    if feeder > 0 {
        let (a, b) = (Vector3::new(-400.0, 0.0, 0.0), corner(0));
        let source = graph.add_node(a, NodeType::Junction);
        let edge = graph.add_edge(Edge {
            start_node: source,
            end_node: 0,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 0,
            speed_limit: 13.89,
            base_cost: 400.0 / 13.89,
            physical_length: 400.0,
            start_clip: MOUTH_M,
            end_clip: MOUTH_M,
            geometry: vec![a, b],
            physical_geometry: vec![a, b],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
        let lanes = &mut network.lane_system.lanes;
        let dir = (b - a).normalized();
        lanes.push(lane(edge, usize::MAX, vec![a + dir * MOUTH_M, b - dir * MOUTH_M], Some(9)));
        // Straight on through corner 0 onto road lane 0.
        let start = lanes[0].geometry[0];
        lanes.push(lane(usize::MAX, 0, vec![b - dir * MOUTH_M, start], Some(0)));
        network.lane_system.edge_lanes.insert(edge, vec![8]);
        let length = network.lane_system.lanes[8].length;
        for slot in 0..feeder {
            let i = truck(&mut agents, 0);
            agents.current_lane_id[i] = 8;
            agents.current_edge[i] = edge;
            agents.current_node[i] = source;
            agents.lane_distance[i] = length - slot as f32 * (CAR_LENGTH + IDM_S_MIN + 0.4);
        }
    }
    Ring {
        agents,
        network,
        graph,
    }
}

/// What the ring holds after a run: cars per lane (roads 0..4, connectors 4..8), cars still
/// waiting at corners, cars that moved over the last `window` ticks, and connector heads that
/// report a speed above 0.5 m/s while standing at the connector's end.
#[derive(Debug)]
pub(super) struct RingState {
    pub per_lane: Vec<usize>,
    pub waiting: usize,
    pub moved_in_window: usize,
    pub stale_connector_heads: usize,
}

/// Ticks the ring `ticks` times at `dt`, checking separation every tick, and reports its
/// state; "moved" compares each car's lane and distance over the last `window` ticks.
pub(super) fn run_ring(ring: &mut Ring, ticks: usize, window: usize, dt: f32) -> RingState {
    let allocator = BuildingAllocator::new();
    let clock = TimeSystem::new();
    let n = ring.agents.len();
    let mut mark = Vec::new();
    for tick in 0..ticks {
        if tick == ticks - window {
            mark = (0..n)
                .map(|i| (ring.agents.current_lane_id[i], ring.agents.lane_distance[i]))
                .collect();
        }
        ring.agents
            .tick(&allocator, &mut ring.network, &mut ring.graph, dt, &clock);
        let mut on_lane: Vec<Vec<f32>> = vec![Vec::new(); ring.network.lane_system.lanes.len()];
        for i in 0..n {
            if let Some(cars) = on_lane.get_mut(ring.agents.current_lane_id[i]) {
                cars.push(ring.agents.lane_distance[i]);
            }
        }
        for (lane, cars) in on_lane.iter_mut().enumerate() {
            cars.sort_by(f32::total_cmp);
            for pair in cars.windows(2) {
                assert!(
                    pair[1] - pair[0] >= CAR_LENGTH + IDM_S_MIN - 1e-3,
                    "tick {tick}: lane {lane} cars at {} and {}",
                    pair[0],
                    pair[1]
                );
            }
        }
    }
    let agents = &ring.agents;
    let lanes = &ring.network.lane_system.lanes;
    let mut per_lane = vec![0; lanes.len()];
    let mut waiting = 0;
    let mut stale_connector_heads = 0;
    for i in 0..n {
        let lane = agents.current_lane_id[i];
        if lane == usize::MAX {
            waiting += 1;
            continue;
        }
        per_lane[lane] += 1;
        if agents.transit[i] == TRANSIT_INTERSECTION
            && agents.lane_distance[i] + 1.0 >= lanes[lane].length
            && agents.speed[i] > 0.5
        {
            stale_connector_heads += 1;
        }
    }
    let moved_in_window = (0..n)
        .filter(|&i| {
            let (lane, d) = mark[i];
            lane != agents.current_lane_id[i] || (d - agents.lane_distance[i]).abs() > 1e-3
        })
        .count();
    RingState {
        per_lane,
        waiting,
        moved_in_window,
        stale_connector_heads,
    }
}

// Asserts the lock: full as the city's ring (141 cars on a 380 m lane, 12 on a 30 m lane),
// nobody moving over the window while trucks still wait.
fn assert_locked(ring: &Ring, state: &RingState) {
    assert_eq!(state.moved_in_window, 0, "{state:?}");
    assert!(state.waiting > 0, "{state:?}");
    for k in 0..4 {
        let fit = (ring.network.lane_system.lanes[k].length / (CAR_LENGTH + IDM_S_MIN)) as usize;
        assert!(state.per_lane[k] >= fit, "lane {k}: {state:?}");
    }
}

#[test]
fn traffic16_connector_heads_stop_at_a_blocked_exit() {
    // The locked ring's four connector heads stand at their connector's end behind a full exit
    // lane. They brake for that exit's entry slot and read 0, not their turn speed.
    let mut ring = ring(12.0, 120, 0);
    let state = run_ring(&mut ring, 4_000, 400, 0.5);
    assert_locked(&ring, &state);
    assert_eq!(state.stale_connector_heads, 0, "{state:?}");
    let agents = &ring.agents;
    let lanes = &ring.network.lane_system.lanes;
    let heads: Vec<f32> = (4..8)
        .map(|connector| {
            let head = (0..agents.len())
                .filter(|&i| agents.current_lane_id[i] == connector)
                .max_by(|&a, &b| agents.lane_distance[a].total_cmp(&agents.lane_distance[b]))
                .expect("a full connector");
            assert!(agents.lane_distance[head] + 1.0 >= lanes[connector].length);
            agents.speed[head]
        })
        .collect();
    assert_eq!(heads, [0.0; 4]);
}

/// Adds an escape at every corner and builds routing for the detour's route query: road lane k
/// may also turn at corner k + 1 onto a one-way spur to an outside node, whose return road runs
/// to corner k + 2 and merges into road lane k + 2. No route uses the escapes until a detour.
fn add_escapes(ring: &mut Ring) {
    let centre = Vector3::new(200.0, 0.0, 25.0);
    for k in 0..4 {
        let (b, c) = (corner(k + 1), corner(k + 2));
        let out = b + (b - centre).normalized() * 100.0;
        let graph = &mut ring.graph;
        let o = graph.add_node(out, NodeType::Junction);
        let spur = road_edge(graph, ((k + 1) % 4) as u32, o, b, out);
        let back = road_edge(graph, o, ((k + 2) % 4) as u32, out, c);
        let lanes = &mut ring.network.lane_system;
        let first = lanes.lanes.len();
        let (spur_lane, turn_out, back_lane, turn_back, merge) =
            (first, first + 1, first + 2, first + 3, first + 4);
        let into = (b - corner(k)).normalized();
        let d1 = (out - b).normalized();
        let d2 = (c - out).normalized();
        let ring_start = lanes.lanes[(k + 2) % 4].geometry[0];
        lanes.lanes.push(lane(
            spur,
            usize::MAX,
            vec![b + d1 * MOUTH_M, out - d1 * MOUTH_M],
            Some(turn_back),
        ));
        lanes.lanes.push(lane(
            usize::MAX,
            (k + 1) % 4,
            vec![b - into * MOUTH_M, b, b + d1 * MOUTH_M],
            Some(spur_lane),
        ));
        lanes.lanes.push(lane(
            back,
            usize::MAX,
            vec![out + d2 * MOUTH_M, c - d2 * MOUTH_M],
            Some(merge),
        ));
        lanes.lanes.push(lane(
            usize::MAX,
            o as usize,
            vec![out - d1 * MOUTH_M, out, out + d2 * MOUTH_M],
            Some(back_lane),
        ));
        lanes.lanes.push(lane(
            usize::MAX,
            (k + 2) % 4,
            vec![c - d2 * MOUTH_M, ring_start],
            Some((k + 2) % 4),
        ));
        lanes.lanes[k].next_lanes.push(turn_out);
        lanes.edge_lanes.insert(spur, vec![spur_lane]);
        lanes.edge_lanes.insert(back, vec![back_lane]);
    }
    ring.network.cch_graph = CchGraph::build(&ring.graph);
}

/// What a ring run with gridlock checks saw: per check the checker's report, the cars that
/// moved since the previous check, and the final state of every car (lane, distance bits, path)
/// for worker-count and reload comparisons.
#[derive(Debug, PartialEq)]
struct CheckedRun {
    reports: Vec<GridlockReport>,
    moved: Vec<usize>,
    cars: Vec<(usize, u32, Vec<u32>)>,
}

/// Ticks the ring for `checks` check intervals of `interval` ticks at dt 0.5 s, running a
/// gridlock check after each interval as the `gridlock check` slot does after a tick. With
/// `reload_at`, the checker's state is saved, serialized and restored as a load would, halfway
/// through that interval.
fn run_checked(
    ring: &mut Ring,
    checks: usize,
    interval: usize,
    reload_at: Option<usize>,
) -> CheckedRun {
    let allocator = BuildingAllocator::new();
    let clock = TimeSystem::new();
    let n = ring.agents.len();
    let mut reports = Vec::new();
    let mut moved = Vec::new();
    let place = |agents: &AgentSystem| -> Vec<(usize, u32)> {
        (0..n)
            .map(|i| (agents.current_lane_id[i], agents.lane_distance[i].to_bits()))
            .collect()
    };
    for check in 0..checks {
        let before = place(&ring.agents);
        for tick in 0..interval {
            if reload_at == Some(check) && tick == interval / 2 {
                // Through the save file's `gridlock_state` row (save format 109).
                let path = std::env::temp_dir().join(format!(
                    "metrum_rise_gridlock_reload_{}.sqlite",
                    std::process::id()
                ));
                let saved = ring.agents.gridlock.save(&ring.network);
                let loaded = round_trip_through_file(&path, &saved).unwrap();
                let _ = std::fs::remove_file(&path);
                assert_eq!(loaded, saved);
                ring.agents.gridlock = GridlockChecker::restore(loaded).unwrap();
                ring.agents.gridlock.bind_loaded_lanes(&ring.network);
            }
            ring.agents
                .tick(&allocator, &mut ring.network, &mut ring.graph, 0.5, &clock);
        }
        let now = ((check + 1) * interval) as u64;
        ring.agents
            .check_gridlock(&allocator, &ring.network, &ring.graph, now);
        assert_eq!(ring.agents.len(), n, "no car is removed");
        let after = place(&ring.agents);
        moved.push(before.iter().zip(&after).filter(|(a, b)| a != b).count());
        reports.push(ring.agents.gridlock.report().clone());
    }
    let agents = &ring.agents;
    let cars = (0..n)
        .map(|i| {
            (
                agents.current_lane_id[i],
                agents.lane_distance[i].to_bits(),
                agents.current_path[i]
                    [agents.current_path_index[i].min(agents.current_path[i].len())..]
                    .iter()
                    .take(6)
                    .copied()
                    .collect(),
            )
        })
        .collect();
    CheckedRun {
        reports,
        moved,
        cars,
    }
}

// Check interval of the ring runs: 60 game seconds, one game minute.
const CHECK_TICKS: usize = 120;
// Check intervals of a ring run: long enough for the ring to fill and lock (about 4,000 ticks).
const RING_CHECKS: usize = 60;

#[test]
fn traffic14_ring_with_exits_detours_out_of_its_lock() {
    // The corners refill the ring until it locks (`TRAFFIC-14`). A lock is confirmed at the check
    // after it is first seen; one road head with a free escape detours there, so no cycle is
    // confirmed at two checks in a row and nothing is ever reported as saturation.
    let mut reference = None;
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let run = pool.install(|| {
            let mut ring = ring(12.0, 120, 0);
            add_escapes(&mut ring);
            run_checked(&mut ring, RING_CHECKS, CHECK_TICKS, None)
        });
        if let Some(expected) = &reference {
            assert_eq!(&run, expected, "workers={workers}");
            continue;
        }
        let last = run.reports.last().unwrap();
        eprintln!(
            "TRAFFIC14_RING_EXITS {last:?} cycles={:?} detours={:?} moved={:?}",
            run.reports.iter().map(|r| r.cycles).collect::<Vec<_>>(),
            run.reports.iter().map(|r| r.detours).collect::<Vec<_>>(),
            run.moved
        );
        assert!(
            last.detours >= 1,
            "the ring locked and a head detoured: {last:?}"
        );
        assert_eq!(last.saturation_reports, 0, "{last:?}");
        for (check, pair) in run.reports.windows(2).enumerate() {
            assert!(
                pair[0].confirmed == 0 || pair[1].confirmed == 0,
                "a cycle stayed confirmed at checks {check} and {}",
                check + 1
            );
        }
        for (check, report) in run.reports.iter().enumerate() {
            if report.confirmed > 0 && check + 1 < run.moved.len() {
                assert!(
                    run.moved[check + 1] > 0,
                    "no progress after the detour at check {check}"
                );
            }
        }
        reference = Some(run);
    }
}

#[test]
fn traffic14_ring_without_exits_stays_locked_and_reports_saturation() {
    // With no way out, a confirmed cycle is genuine saturation: nothing detours, no car is
    // moved or removed, and the ring's four junctions are reported.
    let mut ring = ring(12.0, 120, 0);
    let run = run_checked(&mut ring, RING_CHECKS, CHECK_TICKS, None);
    let last = run.reports.last().unwrap();
    eprintln!("TRAFFIC14_RING_LOCKED {last:?} moved={:?}", run.moved);
    assert_eq!(last.detours, 0);
    assert_eq!(last.saturated, 1, "{last:?}");
    assert_eq!(last.junctions, [0, 1, 2, 3]);
    assert!(last.saturation_reports >= 1);
    assert_eq!(*run.moved.last().unwrap(), 0, "the locked ring stands");
}

#[test]
fn traffic14_gridlock_check_continues_across_a_reload() {
    // A reload between two checks, the checker's state written to a save file's
    // `gridlock_state` row and read back, matches the continuous run: same reports, same
    // detours, same cars.
    let continuous = {
        let mut ring = ring(12.0, 120, 0);
        add_escapes(&mut ring);
        run_checked(&mut ring, RING_CHECKS, CHECK_TICKS, None)
    };
    let first_cycle = continuous
        .reports
        .iter()
        .position(|report| report.cycles > 0)
        .expect("the ring locks");
    // Reload just after the check that first saw the cycle: confirmation needs what it saw.
    let reloaded = {
        let mut ring = ring(12.0, 120, 0);
        add_escapes(&mut ring);
        run_checked(&mut ring, RING_CHECKS, CHECK_TICKS, Some(first_cycle + 1))
    };
    assert!(continuous.reports.last().unwrap().detours >= 1);
    assert_eq!(reloaded, continuous);
}
