// SPDX-License-Identifier: GPL-2.0-only

//! Give-way on entry to a roundabout ring (`ROAD-62`): the rule on its own, the lane-end hold and
//! claim on a converted junction, the gridlock check's view and the fork's drain test. Isolated
//! subsystem setup: no routing, buildings or economy.

use super::super::data::AgentSystem;
use super::gridlock::waited_for;
use super::traffic::ring_entry_blocker;
use crate::config::{CAR_LENGTH, IDM_S_MIN};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::{Lane, LaneType, RingEntry};
use crate::simulation::network::roadbed::fixtures::converted_ring;
use crate::simulation::network::roundabouts::RoundaboutApplied;
use godot::prelude::Vector2;

const NO_LANE: u32 = u32::MAX;

// A converted junction with its lanes built.
struct Site {
    graph: RegionGraph,
    network: TransitNetwork,
    applied: RoundaboutApplied,
}

impl Site {
    fn new(diameter_m: f32, angles_deg: &[f32], arm: (u8, u8)) -> Self {
        let (mut graph, applied) = converted_ring(diameter_m, angles_deg, arm, false).unwrap();
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        Self {
            graph,
            network,
            applied,
        }
    }

    fn lanes(&self) -> &[Lane] {
        &self.network.lane_system.lanes
    }

    // The arm's far node.
    fn far(&self, arm: usize) -> u32 {
        let edge = self.graph.edge(self.applied.arms[arm]);
        let ring_node = self.applied.nodes[arm];
        if edge.start_node == ring_node {
            edge.end_node
        } else {
            edge.start_node
        }
    }

    // Arm `arm`'s outermost vehicle lane toward the ring.
    fn entry_lane(&self, arm: usize) -> usize {
        let id = self.applied.arms[arm];
        let inward = self.graph.edge(id).end_node == self.applied.nodes[arm];
        let lanes = self.network.lane_system.edge_lanes[&id].iter().copied();
        lanes
            .filter(|&lane| {
                let lane = &self.lanes()[lane];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd == inward
            })
            .max_by_key(|&lane| self.lanes()[lane].lane_idx.abs())
            .unwrap()
    }

    // The ring lane of ring edge `edge`: a one-lane ring's only one.
    fn ring_lane(&self, edge: usize) -> usize {
        let mut lanes = self.network.lane_system.edge_lanes[&edge].iter().copied();
        lanes
            .find(|&lane| self.lanes()[lane].lane_type == LaneType::Vehicle)
            .unwrap()
    }

    // The index of the ring node after `node` in the sense of travel.
    fn next_on_ring(&self, node: usize) -> usize {
        let end = self.graph.edge(self.applied.ring[node]).end_node;
        self.applied.nodes.iter().position(|&n| n == end).unwrap()
    }

    // The node path from arm `from`'s far node round the ring and out along arm `to`.
    fn path(&self, from: usize, to: usize) -> Vec<u32> {
        let mut path = vec![self.far(from), self.applied.nodes[from]];
        let mut at = from;
        while at != to {
            at = self.next_on_ring(at);
            path.push(self.applied.nodes[at]);
        }
        path.push(self.far(to));
        path
    }

    // Spawns a car on arm `from`'s entry lane `metres` before its end.
    fn queue_car(&self, agents: &mut AgentSystem, from: usize, to: usize, metres: f32) -> usize {
        let lane = self.entry_lane(from);
        let path = self.path(from, to);
        let far = path[0];
        let id = agents.spawn_debug_car(path, &self.network.lane_system, lane, Vector2::ZERO);
        agents.agents.current_node[id] = far;
        agents.agents.current_path_index[id] = 1;
        agents.agents.lane_distance[id] = (self.lanes()[lane].length - metres).max(0.0);
        id
    }

    // The entry connector of arm `arm` and what it gives way to.
    fn entry(&self, arm: usize) -> (usize, RingEntry) {
        let lane = self.entry_lane(arm);
        let mut next = self.lanes()[lane].next_lanes.iter().copied();
        let system = &self.network.lane_system;
        next.find_map(|c| Some((c, system.ring_entry(c)?)))
            .expect("an arm's entry lane has a ring entry")
    }
}

fn four() -> Site {
    Site::new(36.0, &[0.0, 90.0, 180.0, 270.0], (1, 1))
}

// A bucket of one car at `d`, the agent's index being irrelevant to the rule.
fn with_car(buckets: &mut [Vec<(f32, usize)>], lane: usize, d: f32) {
    buckets[lane].push((d, 0));
}

#[test]
fn a_ring_entry_gives_way_to_each_of_its_three_holds_in_turn() {
    let site = four();
    let lanes = site.lanes();
    let system = &site.network.lane_system;
    let (connector, entry) = site.entry(0);
    let (circulating, ring_in) = (entry.circulating as usize, entry.ring_in as usize);
    let ring_out = lanes[connector].next_lanes[0];
    let empty = vec![Vec::new(); lanes.len()];
    assert_eq!(ring_entry_blocker(connector, system, &empty), None);

    let mut buckets = empty.clone();
    with_car(&mut buckets, circulating, lanes[circulating].length - 0.5);
    assert_eq!(
        ring_entry_blocker(connector, system, &buckets),
        Some(circulating)
    );

    let gap = CAR_LENGTH + IDM_S_MIN + 20.0 / 3.6 * crate::config::IDM_T_HEAD;
    let mut buckets = empty.clone();
    with_car(&mut buckets, ring_in, lanes[ring_in].length - gap + 0.01);
    assert_eq!(
        ring_entry_blocker(connector, system, &buckets),
        Some(ring_in)
    );
    let mut buckets = empty.clone();
    with_car(&mut buckets, ring_in, lanes[ring_in].length - gap - 0.01);
    assert_eq!(ring_entry_blocker(connector, system, &buckets), None);
    // A head standing at the lane's end is not arriving: it waits for its exit or for the two
    // lanes the rule tests itself.
    let mut buckets = empty.clone();
    with_car(&mut buckets, ring_in, lanes[ring_in].length);
    assert_eq!(ring_entry_blocker(connector, system, &buckets), None);

    let mut buckets = empty.clone();
    with_car(&mut buckets, ring_out, 1.0);
    assert_eq!(
        ring_entry_blocker(connector, system, &buckets),
        Some(ring_out)
    );
    // A car past the slot leaves room ahead: this is the slot test, not the box rule.
    let mut buckets = empty;
    with_car(&mut buckets, ring_out, CAR_LENGTH + IDM_S_MIN + 0.1);
    assert_eq!(ring_entry_blocker(connector, system, &buckets), None);
}

#[test]
fn an_ordinary_connector_owes_no_give_way() {
    let site = four();
    let lanes = site.lanes();
    let system = &site.network.lane_system;
    let (entry_connector, entry) = site.entry(0);
    // The circulating connector, and an exit off the ring, are no ring entries.
    assert_eq!(system.ring_entry(entry.circulating as usize), None);
    let exit = lanes[entry.ring_in as usize]
        .next_lanes
        .iter()
        .copied()
        .find(|&next| next != entry.circulating as usize)
        .unwrap();
    assert_eq!(system.ring_entry(exit), None);
    let mut buckets = vec![Vec::new(); lanes.len()];
    with_car(&mut buckets, entry.circulating as usize, 0.0);
    assert_eq!(ring_entry_blocker(exit, system, &buckets), None);
    assert!(ring_entry_blocker(entry_connector, system, &buckets).is_some());
}

// What the yield run records: the tick the circulating car left the connector ahead of the entry,
// the tick the entry car took its connector, and where it stood the tick before.
#[derive(Debug, PartialEq)]
struct YieldRun {
    circulating_clear_tick: usize,
    entered_tick: usize,
    speed_before_bits: u32,
    final_state: Vec<(usize, u32, u32)>,
}

fn run_yield(workers: usize) -> YieldRun {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let mut site = four();
            let mut agents = AgentSystem::new();
            let (connector, entry) = site.entry(0);
            let circulating = entry.circulating as usize;
            // The ring car starts inside the gap on the ring lane into arm 0's node and carries
            // straight on, two nodes round; the entry car stands a metre from its line and would
            // cut it off.
            let before = site
                .applied
                .nodes
                .iter()
                .position(|&node| {
                    let ring_in = site.lanes()[entry.ring_in as usize].edge_id;
                    site.graph.edge(ring_in).start_node == node
                })
                .unwrap();
            let far_past = site.next_on_ring(site.next_on_ring(0));
            let ring_lane = site.ring_lane(site.applied.ring[before]);
            assert_eq!(ring_lane, entry.ring_in as usize);
            let mut path = site.path(before, far_past);
            path.remove(0);
            let ring_car = agents.spawn_debug_car(
                path.clone(),
                &site.network.lane_system,
                ring_lane,
                Vector2::ZERO,
            );
            agents.agents.current_node[ring_car] = path[0];
            agents.agents.current_path_index[ring_car] = 1;
            agents.agents.speed[ring_car] = 5.0;
            agents.agents.lane_distance[ring_car] = (site.lanes()[ring_lane].length - 5.0).max(0.0);
            let entry_car = site.queue_car(&mut agents, 0, 2, 1.0);

            let allocator = BuildingAllocator::new();
            let time = TimeSystem::new();
            let (mut circulating_clear, mut entered, mut speed_before) = (None, None, 0);
            let mut seen_on_connector = false;
            for tick in 0..600 {
                speed_before = agents.agents.speed[entry_car].to_bits();
                agents.tick(&allocator, &mut site.network, &mut site.graph, 0.1, &time);
                let on = agents.agents.current_lane_id[ring_car];
                seen_on_connector |= on == circulating;
                if seen_on_connector && on != circulating && circulating_clear.is_none() {
                    circulating_clear = Some(tick);
                }
                if agents.agents.current_lane_id[entry_car] == connector {
                    entered = Some(tick);
                    break;
                }
            }
            let cars = [ring_car, entry_car];
            YieldRun {
                circulating_clear_tick: circulating_clear.expect("the ring car went through"),
                entered_tick: entered.expect("the entry car took the ring"),
                speed_before_bits: speed_before,
                final_state: cars
                    .iter()
                    .map(|&id| {
                        (
                            agents.agents.current_lane_id[id],
                            agents.agents.lane_distance[id].to_bits(),
                            agents.agents.speed[id].to_bits(),
                        )
                    })
                    .collect(),
            }
        })
}

#[test]
fn an_entry_yields_to_a_circulating_car_and_enters_after_it_passes() {
    let run = run_yield(1);
    assert!(
        run.entered_tick > run.circulating_clear_tick,
        "the entry cut the ring car off: {run:?}"
    );
    // It stood at the line while the ring car went by.
    assert!(
        f32::from_bits(run.speed_before_bits) < 3.0,
        "the hold did not brake the car: {run:?}"
    );
    assert_eq!(run, run_yield(4));
}

#[test]
fn the_gridlock_check_sees_a_yielding_head_waiting_for_the_ring() {
    let site = four();
    let mut agents = AgentSystem::new();
    let lanes = site.lanes();
    let (_, entry) = site.entry(0);
    let (circulating, ring_in) = (entry.circulating as usize, entry.ring_in as usize);
    let entry_lane = site.entry_lane(0);
    let head = site.queue_car(&mut agents, 0, 2, 0.0);
    let waits = |other: Option<(usize, f32)>| {
        let mut buckets = vec![Vec::new(); lanes.len()];
        buckets[entry_lane].push((lanes[entry_lane].length, head));
        if let Some((lane, d)) = other {
            buckets[lane].push((d, usize::MAX));
        }
        waited_for(
            &agents.agents,
            entry_lane,
            &site.network,
            &site.graph,
            &buckets,
        )
    };
    assert_eq!(waits(None), NO_LANE);
    assert_eq!(waits(Some((circulating, 1.0))), circulating as u32);
    assert_eq!(
        waits(Some((ring_in, lanes[ring_in].length - 2.0))),
        ring_in as u32
    );
}

// Six uneven arms (the fork's test), six cars queued on each going the long way round: 2,400 ticks
// of 0.1 s. Every car leaves, whatever the worker count.
fn drain(workers: usize, arm: (u8, u8)) -> Vec<(usize, u32, u32)> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let angles = [0.0, 0.9, 2.0, 3.2, 4.2, 5.3_f32].map(f32::to_degrees);
            let mut site = Site::new(60.0, &angles, arm);
            let mut agents = AgentSystem::new();
            let mut targets = Vec::new();
            for from in 0..6 {
                // The arm before this one in the sense of travel: five ring edges away.
                let to = (0..6)
                    .find(|&other| site.next_on_ring(other) == from)
                    .unwrap();
                for queue in 0..6 {
                    site.queue_car(&mut agents, from, to, 2.0 + queue as f32 * 8.0);
                    targets.push(site.applied.arms[to]);
                }
            }
            let allocator = BuildingAllocator::new();
            let time = TimeSystem::new();
            let mut exited = vec![false; targets.len()];
            for _ in 0..2400 {
                agents.tick(&allocator, &mut site.network, &mut site.graph, 0.1, &time);
                for (id, &target) in targets.iter().enumerate() {
                    exited[id] |= agents.agents.current_edge[id] == target
                        && agents.agents.current_path_index[id] > 2;
                    assert!(agents.agents.speed[id].is_finite());
                }
                if exited.iter().all(|&exited| exited) {
                    break;
                }
            }
            assert!(
                exited.iter().all(|&exited| exited),
                "roundabout queues must drain: {exited:?}"
            );
            (0..targets.len())
                .map(|id| {
                    (
                        agents.agents.current_edge[id],
                        agents.agents.lane_distance[id].to_bits(),
                        agents.agents.speed[id].to_bits(),
                    )
                })
                .collect()
        })
}

#[test]
fn queues_on_six_uneven_arms_drain_the_long_way_round_on_any_worker_count() {
    assert_eq!(drain(1, (1, 1)), drain(4, (1, 1)));
}

#[test]
fn queues_on_a_two_lane_ring_drain_the_long_way_round_on_any_worker_count() {
    assert_eq!(drain(1, (2, 2)), drain(4, (2, 2)));
}
