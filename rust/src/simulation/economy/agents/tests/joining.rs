// SPDX-License-Identifier: GPL-3.0-or-later

//! Joining a creeping queue (`TRAFFIC-45`) on a 1 km two-way road at 1 m/s between dead ends 0
//! (west) and 1 (east): the westbound lane is a queue going stop and go, a body and its
//! standstill gap apart plus half a second at its speed, so it never leaves the two bodies' hole
//! an attach slot needs. A car held at its attach point waits `CROSSING_GIVE_UP_S`, then holds its place:
//! the body behind stops short of its slot, and it enters once the body ahead has cleared it.

use super::support::*;
use super::*;
use crate::config::CAR_BODY_LENGTH_M;
use crate::simulation::economy::agents::tick::CROSSING_GIVE_UP_S;

const DT: f32 = 0.25;
const SPEED: f32 = 1.0;
// A queued car's distance to the next one at its speed: its body, the standstill gap and
// half a second (`IDM_T_HEAD`) at 1 m/s.
const SPACING: f32 = 5.1;

struct Queue {
    agents: AgentSystem,
    allocator: BuildingAllocator,
    network: TransitNetwork,
    graph: RegionGraph,
    westbound: usize,
}

fn queue() -> Queue {
    let mut graph = RegionGraph::new();
    graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    graph.add_node(Vector3::new(1_000.0, 0.0, 0.0), NodeType::Junction);
    let road = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(1_000.0, 0.0, 0.0)];
    graph.add_edge(Edge {
        physical_length: 1_000.0,
        base_cost: 1_000.0 / SPEED,
        speed_limit: SPEED,
        geometry: road.to_vec(),
        physical_geometry: road.to_vec(),
        ..create_test_edge(0, 1)
    });
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let westbound = bkw_vehicle_lanes(&network, 0)[0];
    let mut q = Queue {
        agents: AgentSystem::new(),
        allocator: BuildingAllocator::new(),
        network,
        graph,
        westbound,
    };
    let mut d = 0.0;
    while d < 700.0 {
        q.through(d);
        d += SPACING;
    }
    q
}

impl Queue {
    // A car bound for the western dead end, on the westbound lane at `d`, or held at its attach
    // point there.
    fn car(&mut self, d: f32, held: bool) -> usize {
        let a = &mut self.agents;
        let idx = a.spawn_border_arrival_agent(usize::MAX, 1, 0.0, 0.0);
        a.transit_mode[idx] = MODE_CAR;
        if held {
            a.agents
                .hold_for_lane_attach(idx, &self.network.lane_system, self.westbound, d, 1);
        } else {
            a.agents
                .enter_lane(idx, &self.network.lane_system, self.westbound, d, 1);
            a.speed[idx] = SPEED;
        }
        a.current_path[idx] = vec![1, 0];
        a.current_path_index[idx] = 1;
        a.access_flags[idx] |= ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
        a.freight_target_border_node[idx] = 0;
        a.planned_detach_node[idx] = 0;
        idx
    }

    fn through(&mut self, d: f32) -> usize {
        self.car(d, false)
    }

    fn joiner(&mut self, d: f32) -> usize {
        self.car(d, true)
    }

    // One tick, then a car fed at the lane's start whenever the last one has moved a spacing on,
    // so the queue stays full; checks that no two bodies on the lane overlap.
    fn tick(&mut self) {
        self.agents.tick(
            &self.allocator,
            &mut self.network,
            &mut self.graph,
            DT,
            &test_clock(0, 0),
        );
        let bodies: Vec<_> = self.agents.lane_buckets[self.westbound]
            .iter()
            .filter(|entry| entry.2 > 0.0)
            .copied()
            .collect();
        for pair in bodies.windows(2) {
            assert!(
                pair[1].0 - pair[0].0 >= pair[0].2 + pair[1].2 - 1e-3,
                "bodies overlap: {pair:?}"
            );
        }
        // A car enters at the start only into a free slot there, a stop line counted, as at a
        // junction (`lane_entry_slot_clear`).
        let lane = &self.agents.lane_buckets[self.westbound];
        if lane.first().is_none_or(|first| first.0 >= SPACING) {
            self.through(0.0);
        }
    }

    fn joined(&self, car: usize) -> bool {
        self.agents.current_lane_id[car] == self.westbound
    }
}

// Ticks until every one of `cars` is on the lane; returns each one's join time and the final
// state of all of them as bits.
fn join(workers: usize, at: &[f32]) -> (Vec<f32>, Vec<u32>) {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let mut q = queue();
            let cars: Vec<usize> = at.iter().map(|&d| q.joiner(d)).collect();
            let mut joined_at = vec![f32::NAN; cars.len()];
            for tick in 1..=800 {
                q.tick();
                let t = tick as f32 * DT;
                for (k, &car) in cars.iter().enumerate() {
                    if joined_at[k].is_nan() && q.joined(car) {
                        joined_at[k] = t;
                        assert_eq!(
                            q.agents.crossing_wait_s[car], 0.0,
                            "its wait outlived its join"
                        );
                    }
                }
                if joined_at.iter().all(|t| !t.is_nan()) {
                    let a = &q.agents;
                    let bits = (0..a.len())
                        .flat_map(|i| [a.lane_distance[i].to_bits(), a.speed[i].to_bits()])
                        .collect();
                    return (joined_at, bits);
                }
            }
            panic!("joined only at {joined_at:?}");
        })
}

#[test]
fn a_car_held_at_its_slot_joins_a_creeping_queue_after_holding_its_place() {
    let (joined, bits) = join(1, &[400.0]);
    // No hole ever opens on its own: it joins only by its hold, taken at 30 s, and within the
    // time the two bodies about its slot need to clear it at the queue's pace (stop and go, a
    // spacing about every 10 s).
    assert!(
        joined[0] > CROSSING_GIVE_UP_S,
        "joined at {} s with no hold",
        joined[0]
    );
    assert!(
        joined[0] <= CROSSING_GIVE_UP_S + 35.0,
        "joined only at {} s",
        joined[0]
    );
    for workers in [3, 8] {
        assert_eq!(
            join(workers, &[400.0]).1,
            bits,
            "{workers} workers changed the outcome"
        );
    }
}

#[test]
fn a_held_car_stops_the_body_behind_short_of_its_slot() {
    let mut q = queue();
    let car = q.joiner(400.0);
    let half = CAR_BODY_LENGTH_M * 0.5;
    let mut held = false;
    while !q.joined(car) {
        q.tick();
        let entries = &q.agents.lane_buckets[q.westbound];
        let Some(&(stop, _, _)) = entries
            .iter()
            .find(|entry| entry.1 == car && entry.2 == 0.0)
        else {
            continue;
        };
        held = true;
        // Every body behind its stop line stands short of it.
        for &(d, agent, h) in entries {
            if agent != car && d < stop {
                assert!(
                    d + h <= stop + 1e-3,
                    "body {agent} at {d} runs over the line {stop}"
                );
            }
        }
        assert!(stop <= 400.0 - half, "the line {stop} lies in its slot");
    }
    assert!(held, "it joined without holding its place");
}

// Two cars held at slots that overlap each other both join: one after the other, never through
// each other, whatever their order.
#[test]
fn two_holds_at_adjacent_slots_both_join() {
    let (joined, bits) = join(1, &[400.0, 403.0]);
    assert!(joined.iter().all(|&t| t <= 120.0), "joined at {joined:?}");
    assert_eq!(
        join(8, &[400.0, 403.0]).1,
        bits,
        "8 workers changed the outcome"
    );
    let (joined, _) = join(1, &[400.0, 400.0]);
    assert!(
        joined.iter().all(|&t| t <= 120.0),
        "one point, joined at {joined:?}"
    );
}

// A slot at the lane's start, inside the junction's clearance: its line stands at the start, so
// the next car waits on the junction side, and it joins.
#[test]
fn a_hold_at_the_lane_start_joins() {
    let (joined, _) = join(1, &[3.0]);
    assert!(joined[0] <= 60.0, "joined at {}", joined[0]);
}

// A road edit over the held lane releases the hold with its wait; it waits again where the edit
// left it.
#[test]
fn a_road_edit_releases_the_hold() {
    let mut q = queue();
    let car = q.joiner(400.0);
    while q.agents.crossing_wait_s[car] < CROSSING_GIVE_UP_S + 1.0 {
        q.tick();
        assert!(!q.joined(car), "it joined while waiting");
    }
    assert!(
        q.agents.lane_buckets[q.westbound]
            .iter()
            .any(|entry| entry.1 == car)
    );
    let edges = std::collections::HashSet::from([0]);
    q.agents
        .invalidate_lane_ids_for_edges(&edges, &q.network.lane_system, &q.graph);
    assert_eq!(q.agents.crossing_wait_s[car], 0.0, "the edit left its hold");
}
