// SPDX-License-Identifier: GPL-2.0-only

//! `TRAFFIC-14` ring fixture: one block of the populated-city grid as a closed ring of four
//! one-way lanes (380 m, 30 m, 380 m, 30 m) joined by four 20 m corner connectors, the way the
//! 100k city's locked block is built. Trucks circulate on looping routes while more wait at
//! every corner to start there (node-origin entry, `TRAFFIC-07`). Isolated subsystem setup: no
//! routing, buildings or economy.

use super::*;
use crate::config::{CAR_LENGTH, IDM_S_MIN};
use crate::simulation::economy::agents::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_INTERSECTION,
    TRANSIT_NETWORK,
};
use crate::simulation::network::graph::Edge;
use crate::simulation::network::lanes::{Lane, LaneSuccessors};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use godot::prelude::Vector3;

// Block corners, counter-clockwise; edge k runs from corner k to corner k + 1.
const CORNERS: [(f32, f32); 4] = [(0.0, 0.0), (400.0, 0.0), (400.0, 50.0), (0.0, 50.0)];
// Junction mouths: every road lane stops this far short of both of its nodes.
const MOUTH_M: f32 = 10.0;
// Laps in every route: far more than the test drives.
const LAPS: usize = 40;

/// The ring: agents, lanes and graph. Road lanes are 0..4, connector k (lane 4 + k) turns at
/// corner k + 1 from road lane k onto road lane k + 1.
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

/// Builds the ring with `spacing_m` between circulating trucks (0 for none) and `waiting`
/// trucks held at every corner for their first lane.
pub(super) fn ring(spacing_m: f32, waiting: usize) -> Ring {
    let mut agents = AgentSystem::new();
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    for k in 0..4 {
        graph.add_node(corner(k), NodeType::Junction);
    }
    for k in 0..4 {
        let (a, b) = (corner(k), corner(k + 1));
        let geometry = vec![a, b];
        let length = a.distance_to(b);
        graph.add_edge(Edge {
            start_node: k as u32,
            end_node: ((k + 1) % 4) as u32,
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
            geometry: geometry.clone(),
            physical_geometry: geometry,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
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
    Ring {
        agents,
        network,
        graph,
    }
}

/// What the ring holds after a run: cars per lane (roads 0..4, connectors 4..8), cars still
/// waiting at corners, cars that moved over the last `window` ticks, connector heads that
/// report a speed above 0.5 m/s while standing at the connector's end, and lane entries
/// (a car's lane changed) over the whole run.
#[derive(Debug)]
pub(super) struct RingState {
    pub per_lane: [usize; 8],
    pub waiting: usize,
    pub moved_in_window: usize,
    pub stale_connector_heads: usize,
    pub entries: usize,
}

/// Ticks the ring `ticks` times at `dt`, checking separation every tick, and reports its
/// state; "moved" compares each car's lane and distance over the last `window` ticks.
pub(super) fn run_ring(ring: &mut Ring, ticks: usize, window: usize, dt: f32) -> RingState {
    let allocator = BuildingAllocator::new();
    let clock = TimeSystem::new();
    let n = ring.agents.len();
    let mut entries = 0;
    let mut mark = Vec::new();
    for tick in 0..ticks {
        let before: Vec<usize> = ring.agents.current_lane_id.clone();
        if tick == ticks - window {
            mark = (0..n)
                .map(|i| (ring.agents.current_lane_id[i], ring.agents.lane_distance[i]))
                .collect();
        }
        ring.agents
            .tick(&allocator, &mut ring.network, &mut ring.graph, dt, &clock);
        entries += (0..n)
            .filter(|&i| ring.agents.current_lane_id[i] != before[i])
            .count();
        let mut on_lane: [Vec<f32>; 8] = Default::default();
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
    let mut per_lane = [0; 8];
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
        entries,
    }
}

#[test]
#[ignore = "TRAFFIC-14: documents the current lock; the build round turns it into a progress test"]
fn traffic14_full_ring_locks() {
    // Circulating trucks every 12 m plus 120 trucks waiting at every corner: the corners
    // refill the ring whenever an entry clears, until every lane and connector is full.
    let mut ring = ring(12.0, 120);
    let state = run_ring(&mut ring, 4_000, 400, 0.5);
    eprintln!("TRAFFIC14_RING_FIXTURE {state:?}");
    // Full as the city's ring: 141 cars on a 380 m lane, 12 on a 30 m lane, 8 on a 20 m
    // connector; nobody moves for the last 200 game seconds while trucks still wait.
    assert_eq!(state.moved_in_window, 0, "{state:?}");
    assert!(state.waiting > 0, "{state:?}");
    for k in 0..4 {
        let fit = (ring.network.lane_system.lanes[k].length / (CAR_LENGTH + IDM_S_MIN)) as usize;
        assert!(state.per_lane[k] >= fit, "lane {k}: {state:?}");
    }
}
