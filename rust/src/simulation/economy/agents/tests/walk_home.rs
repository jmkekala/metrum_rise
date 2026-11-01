// SPDX-License-Identifier: GPL-2.0-only

//! A walker whose route is gone gives its trip up and walks home, or is stranded with a backoff
//! when home is out of reach too (`ROAD-74`, the user's option (c)).
//!
//! The fixture is a square block of four roads. With every crossing off its inner and outer
//! sidewalks are two walk networks, so a walker on the outer one cannot reach a door on the inner
//! one; the editor would clear such routes, so these tests change the graph directly.

use super::support::*;
use super::*;
use crate::config::AGENT_WALK_SPEED_MS;
use crate::simulation::network::lanes::LaneType;

struct Block {
    network: TransitNetwork,
    graph: RegionGraph,
    allocator: BuildingAllocator,
}

impl Block {
    // The block with a home (building 0) and a shop (building 1) on the given sides of roads 0
    // and 2.
    fn new(home_inner: bool, shop_inner: bool) -> Self {
        let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        let mut graph = RegionGraph::new();
        for &(x, z) in &corners {
            graph.add_node(Vector3::new(x, 0.0, z), NodeType::Junction);
        }
        for from in 0..4u32 {
            let to = (from + 1) % 4;
            let (a, b) = (graph.node(from).pos, graph.node(to).pos);
            let mut edge = create_test_edge(from, to);
            edge.geometry = vec![a, b];
            edge.physical_geometry = edge.geometry.clone();
            graph.add_edge(edge);
        }
        graph.rebuild_adjacency_list();
        let mut block = Self {
            network: TransitNetwork::new(),
            graph,
            allocator: BuildingAllocator::new(),
        };
        block.rebuild();
        for (edge, inner) in [(0, home_inner), (2, shop_inner)] {
            block
                .allocator
                .buildings
                .push(create_test_building(edge, 1));
            block.allocator.entrances.push(BuildingEntrance::default());
            block.door(block.allocator.entrances.len() - 1, edge, inner);
        }
        block
    }

    fn rebuild(&mut self) {
        self.graph.rebuild_intersection_clips();
        self.network.lane_system.rebuild(&mut self.graph);
        self.network.cch_graph = CchGraph::build(&self.graph);
    }

    // Turns every crossing on or off and rebuilds the lanes, keeping the doors on their sides.
    fn crossings(&mut self, on: bool) {
        for node in 0..4u32 {
            for edge in self.graph.node_adjacency(node).to_vec() {
                self.graph.set_crosswalk_override(node, edge, on);
            }
        }
        self.rebuild();
        for (building, edge) in [(0, 0), (1, 2)] {
            let inner = self.is_inner(self.allocator.entrances[building].foot_lane_fwd);
            self.door(building, edge, inner);
        }
    }

    // Whether foot lane `lane` runs along the block's inner side.
    fn is_inner(&self, lane: usize) -> bool {
        let lane = &self.network.lane_system.lanes[lane];
        let mid = BuildingAllocator::sample_pos_on_lane(lane, lane.length * 0.5);
        mid.distance_to(Vector2::new(50.0, 50.0)) < 50.0
    }

    // The foot lane of `edge` on the inner or outer side, forward or backward.
    fn side_lane(&self, edge: usize, inner: bool, fwd: bool) -> usize {
        self.network.lane_system.edge_lanes[&edge]
            .iter()
            .copied()
            .find(|&lane| {
                let value = &self.network.lane_system.lanes[lane];
                value.lane_type == LaneType::Foot
                    && value.is_fwd == fwd
                    && self.is_inner(lane) == inner
            })
            .unwrap()
    }

    fn door(&mut self, building: usize, edge: usize, inner: bool) {
        let fwd = self.side_lane(edge, inner, true);
        let lane = &self.network.lane_system.lanes[fwd];
        let curb = BuildingAllocator::sample_pos_on_lane(lane, 50.0);
        self.allocator.entrances[building] = BuildingEntrance {
            edge_idx: edge,
            side: 1,
            entrance_s_m: 50.0,
            foot_lane_fwd: fwd,
            foot_lane_bkw: self.side_lane(edge, inner, false),
            door_pos: curb,
            curb_pos: curb,
            ..BuildingEntrance::default()
        };
    }

    // A walker of household home 0 on the outer forward sidewalk of road 1, `d` metres from its
    // end, heading for building `target` with no plan yet.
    fn walker(&self, agents: &mut AgentSystem, target: usize, d: f32) -> usize {
        let lane = self.side_lane(1, false, true);
        let len = self.network.lane_system.lanes[lane].length;
        let idx = agents.spawn_border_arrival_agent(usize::MAX, 1, 0.0, 0.0);
        agents.home_building[idx] = 0;
        agents.transit[idx] = TRANSIT_NETWORK;
        agents.transit_mode[idx] = MODE_WALK;
        agents.speed[idx] = AGENT_WALK_SPEED_MS;
        agents.current_node[idx] = 1;
        agents.current_edge[idx] = 1;
        agents.current_lane_id[idx] = lane;
        agents.lane_distance[idx] = (len - d).max(0.0);
        agents.target_building[idx] = target;
        let pos =
            BuildingAllocator::sample_pos_on_lane(&self.network.lane_system.lanes[lane], len - d);
        agents.pos_x[idx] = pos.x;
        agents.pos_y[idx] = pos.y;
        idx
    }

    fn tick(&mut self, agents: &mut AgentSystem) {
        agents.tick(
            &self.allocator,
            &mut self.network,
            &mut self.graph,
            0.1,
            &test_clock(0, 0),
        );
    }
}

// Ticks until `done`, at most `seconds`, asserting no on-road step longer than a walker's.
fn walk_until(
    block: &mut Block,
    agents: &mut AgentSystem,
    idx: usize,
    seconds: f32,
    mut done: impl FnMut(&AgentSystem) -> bool,
) -> bool {
    let mut last = (agents.pos_x[idx], agents.pos_y[idx]);
    for tick in 0..(seconds / 0.1) as usize {
        let on_road = matches!(agents.transit[idx], TRANSIT_NETWORK | TRANSIT_INTERSECTION);
        block.tick(agents);
        let pos = (agents.pos_x[idx], agents.pos_y[idx]);
        // The first tick places a walker the test moved onto a rebuilt lane.
        if tick > 0
            && on_road
            && matches!(agents.transit[idx], TRANSIT_NETWORK | TRANSIT_INTERSECTION)
        {
            let step = (pos.0 - last.0).hypot(pos.1 - last.1);
            assert!(step < 1.0, "no teleport: {step} m in one tick");
        }
        last = pos;
        if done(agents) {
            return true;
        }
    }
    false
}

fn at_home(agents: &AgentSystem, idx: usize) -> bool {
    agents.transit[idx] == TRANSIT_IN_BUILDING && agents.current_building[idx] == 0
}

fn stranded(agents: &AgentSystem, idx: usize) -> bool {
    agents.access_flags[idx] & ACCESS_WALK_STRANDED != 0
}

#[test]
fn a_walker_whose_shop_becomes_unreachable_gives_up_and_walks_home() {
    let mut block = Block::new(false, true);
    let mut agents = AgentSystem::new();
    let idx = block.walker(&mut agents, 1, 20.0);
    block.tick(&mut agents);
    assert!(
        agents.access_flags[idx] & ACCESS_PLAN_VALID != 0
            && agents.planned_detach_lane_id[idx] != u32::MAX,
        "it plans the walk to the shop"
    );
    // Every crossing goes while it walks: the toggle's lane rebuild reattaches it to its rebuilt
    // sidewalk and its exit to the rebuilt door lane.
    let (lane, d) = (agents.current_lane_id[idx], agents.lane_distance[idx]);
    let key = |network: &TransitNetwork, lane: usize| {
        let value = &network.lane_system.lanes[lane];
        (value.edge_id, value.is_fwd, value.lane_idx)
    };
    let (lane_key, exit_key) = (
        key(&block.network, lane),
        key(&block.network, agents.planned_detach_lane_id[idx] as usize),
    );
    block.crossings(false);
    let find = |network: &TransitNetwork, wanted| {
        (0..network.lane_system.lanes.len())
            .find(|&id| {
                network.lane_system.lanes[id].edge_id != usize::MAX && key(network, id) == wanted
            })
            .unwrap()
    };
    agents.current_lane_id[idx] = find(&block.network, lane_key);
    agents.lane_distance[idx] = d;
    agents.planned_detach_lane_id[idx] = find(&block.network, exit_key) as u32;

    assert!(
        walk_until(&mut block, &mut agents, idx, 300.0, |a| at_home(a, idx)),
        "it walks home"
    );
    assert_eq!(agents.activity[idx], ACTIVITY_HOME);
    let lines = &agents.trip_failures.lines;
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(
        lines[0].contains("gives up and walks home to building 0"),
        "{lines:#?}"
    );
}

#[test]
fn a_walker_with_no_way_home_waits_on_its_backoff_and_walks_home_once_it_has_one() {
    // Home and shop on the inner side, the walker on the outer one: neither can be reached.
    let mut block = Block::new(true, true);
    block.crossings(false);
    let mut agents = AgentSystem::new();
    let idx = block.walker(&mut agents, 1, 1.0);
    let lane = block.side_lane(1, false, true);
    block.tick(&mut agents);
    assert!(stranded(&agents, idx));
    assert_eq!(
        agents.target_building[idx], 0,
        "it gave its trip up for home"
    );
    let stranded_at = agents.sim_time;
    // The plans home after 5 s, 30 s, then every 2 min.
    let mut plans = vec![agents.next_replan_time[idx]];
    while plans.len() < 5 {
        block.tick(&mut agents);
        assert!(stranded(&agents, idx));
        if agents.next_replan_time[idx] != *plans.last().unwrap() {
            plans.push(agents.next_replan_time[idx]);
        }
    }
    let mut waits = vec![plans[0] - stranded_at];
    waits.extend(plans.windows(2).map(|pair| pair[1] - pair[0]));
    for (wait, expected) in waits.iter().zip([5.0, 30.0, 120.0, 120.0, 120.0]) {
        assert!((wait - expected).abs() < 0.15, "waits {waits:?}");
    }
    // It waits at the end of its lane, and the report names that junction.
    let node = block.graph.edge(1).end_node;
    assert_eq!(agents.current_lane_id[idx], lane);
    assert_eq!(
        agents.lane_distance[idx],
        block.network.lane_system.lanes[lane].length
    );
    agents.refresh_stranded_report(&block.network, &block.graph);
    assert_eq!(agents.stranded_report().junctions, vec![(node, 1)]);
    let lines = &agents.trip_failures.lines;
    assert_eq!(
        lines.len(),
        1,
        "one record when it becomes stranded: {lines:#?}"
    );
    assert!(lines[0].contains("stranded with no way home"), "{lines:#?}");

    // The crossings return: its next plan home succeeds and it walks home.
    block.crossings(true);
    agents.current_lane_id[idx] = block.side_lane(1, false, true);
    assert!(
        walk_until(&mut block, &mut agents, idx, 400.0, |a| at_home(a, idx)),
        "it walks home"
    );
    agents.refresh_stranded_report(&block.network, &block.graph);
    assert!(agents.stranded_report().junctions.is_empty());
    assert_eq!(agents.trip_failures.lines.len(), 1, "no record per retry");
}

#[test]
fn a_walker_already_walking_home_is_stranded_when_home_becomes_unreachable() {
    let mut block = Block::new(true, true);
    block.crossings(false);
    let mut agents = AgentSystem::new();
    let idx = block.walker(&mut agents, 0, 1.0);
    block.tick(&mut agents);
    assert!(stranded(&agents, idx));
    assert_eq!(agents.target_building[idx], 0);
    let lines = &agents.trip_failures.lines;
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(lines[0].contains("stranded with no way home"), "{lines:#?}");
}

// Walkers stranded at two corners of the block; returns each walker's final state and the report.
// (lane, distance bits, next plan time bits, backoff step, flags, x bits) per walker.
type WalkerState = (usize, u32, u32, u8, u8, u32);

fn strand_at_two_corners(workers: usize) -> (Vec<WalkerState>, Vec<(u32, u32)>) {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let mut block = Block::new(true, true);
            block.crossings(false);
            let mut agents = AgentSystem::new();
            let mut ids = Vec::new();
            for k in 0..12 {
                ids.push(block.walker(&mut agents, 1, 1.0 + 3.0 * k as f32));
            }
            // Four more on road 0's outer sidewalk, waiting at its end (node 1).
            let lane = block.side_lane(0, false, true);
            let len = block.network.lane_system.lanes[lane].length;
            for k in 0..4 {
                let idx = block.walker(&mut agents, 1, 1.0);
                agents.current_node[idx] = 0;
                agents.current_edge[idx] = 0;
                agents.current_lane_id[idx] = lane;
                agents.lane_distance[idx] = len - 1.0 - 3.0 * k as f32;
                ids.push(idx);
            }
            // More than `PAR_THRESHOLD` agents, so the movement pass runs on the pool.
            for _ in 0..500 {
                agents.spawn_housed_agent(0, 0.0, 0.0);
            }
            for _ in 0..1000 {
                block.tick(&mut agents);
            }
            agents.refresh_stranded_report(&block.network, &block.graph);
            let state = ids
                .iter()
                .map(|&idx| {
                    (
                        agents.current_lane_id[idx],
                        agents.lane_distance[idx].to_bits(),
                        agents.next_replan_time[idx].to_bits(),
                        agents.network_replan_failures[idx],
                        agents.access_flags[idx],
                        agents.pos_x[idx].to_bits(),
                    )
                })
                .collect();
            (state, agents.stranded_report().junctions.clone())
        })
}

#[test]
fn the_report_names_each_junction_with_its_stranded_walkers_for_1_and_8_workers() {
    let serial = strand_at_two_corners(1);
    // Road 1's walkers wait at node 2, road 0's at node 1.
    assert_eq!(serial.1, vec![(1, 4), (2, 12)]);
    assert!(
        serial
            .0
            .iter()
            .all(|state| state.4 & ACCESS_WALK_STRANDED != 0)
    );
    assert_eq!(serial, strand_at_two_corners(8));
}
