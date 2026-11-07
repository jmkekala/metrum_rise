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

    // The vehicle lane of `edge` running forward or backward.
    fn car_lane(&self, edge: usize, fwd: bool) -> usize {
        self.network.lane_system.edge_lanes[&edge]
            .iter()
            .copied()
            .find(|&lane| {
                let value = &self.network.lane_system.lanes[lane];
                value.lane_type == LaneType::Vehicle && value.is_fwd == fwd
            })
            .unwrap()
    }

    // A driver on road 1, its route still to be planned, and car lanes for the two doors.
    fn driver(&mut self, agents: &mut AgentSystem, target: usize) -> usize {
        for (building, edge) in [(0, 0), (1, 2)] {
            self.allocator.entrances[building].car_lane_fwd = self.car_lane(edge, true);
            self.allocator.entrances[building].car_lane_bkw = self.car_lane(edge, false);
        }
        let lane = self.car_lane(1, true);
        let idx = agents.spawn_border_arrival_agent(usize::MAX, 1, 0.0, 0.0);
        agents.home_building[idx] = 0;
        agents.transit[idx] = TRANSIT_NETWORK;
        agents.transit_mode[idx] = MODE_CAR;
        agents.current_node[idx] = 1;
        agents.current_edge[idx] = 1;
        agents.current_lane_id[idx] = lane;
        agents.lane_distance[idx] = 10.0;
        agents.target_building[idx] = target;
        let pos = BuildingAllocator::sample_pos_on_lane(&self.network.lane_system.lanes[lane], 10.0);
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

// `TRAFFIC-19`: walkers the watchdog used to move home.

// Whether any record so far is the watchdog's teleport.
fn teleported(agents: &AgentSystem) -> bool {
    agents
        .trip_failures
        .lines
        .iter()
        .any(|line| line.contains("sent home") || line.contains("sent to border"))
}

impl Block {
    // A walker of household home 0 standing at `node` with no lane, at `pos`, heading for
    // building `target` with no plan.
    fn walker_at_node(
        &self,
        agents: &mut AgentSystem,
        node: u32,
        pos: Vector2,
        target: usize,
    ) -> usize {
        let idx = agents.spawn_border_arrival_agent(usize::MAX, node, pos.x, pos.y);
        agents.home_building[idx] = 0;
        agents.transit[idx] = TRANSIT_NETWORK;
        agents.transit_mode[idx] = MODE_WALK;
        agents.speed[idx] = AGENT_WALK_SPEED_MS;
        agents.current_node[idx] = node;
        agents.current_edge[idx] = usize::MAX;
        agents.current_lane_id[idx] = usize::MAX;
        agents.target_building[idx] = target;
        agents.pos_x[idx] = pos.x;
        agents.pos_y[idx] = pos.y;
        idx
    }

    // The end of foot lane `lane`, where a walker that left it stands.
    fn lane_end(&self, lane: usize) -> Vector2 {
        let end = *self.network.lane_system.lanes[lane]
            .geometry
            .last()
            .unwrap();
        Vector2::new(end.x, end.z)
    }

    // A car-only road from a new node 60 m west of node 0 into node 0: the new node (4) has no
    // sidewalk unless `foot`.
    fn add_spur(&mut self, foot: bool) -> u32 {
        let node = self
            .graph
            .add_node(Vector3::new(-60.0, 0.0, 0.0), NodeType::Junction);
        let mut edge = create_test_edge(node, 0);
        edge.geometry = vec![self.graph.node(node).pos, self.graph.node(0).pos];
        edge.physical_geometry = edge.geometry.clone();
        if !foot {
            edge.allowed_types = TransitFlags::CAR;
            edge.sidewalks = crate::simulation::network::graph::SidewalkSides::NONE;
        }
        self.graph.add_edge(edge);
        self.graph.rebuild_adjacency_list();
        self.relane();
        node
    }

    // Rebuilds the lanes, keeping the doors on their sides.
    fn relane(&mut self) {
        let sides: Vec<bool> = (0..2)
            .map(|b| self.is_inner(self.allocator.entrances[b].foot_lane_fwd))
            .collect();
        self.rebuild();
        for (building, edge) in [(0, 0), (1, 2)] {
            self.door(building, edge, sides[building]);
        }
    }
}

#[test]
fn a_walker_at_a_node_with_no_lane_takes_the_sidewalk_it_stands_on_and_walks_home() {
    // Home on the outer side, the shop on the inner one, every crossing off: from the outer
    // sidewalk the shop is out of reach, home is not.
    let mut block = Block::new(false, true);
    block.crossings(false);
    let mut agents = AgentSystem::new();
    let lane = block.side_lane(1, false, true);
    let pos = block.lane_end(lane);
    let idx = block.walker_at_node(&mut agents, 2, pos, 1);
    block.tick(&mut agents);
    // It plans from the end of the sidewalk it stands on, gives the shop up and sets off home.
    let lines = &agents.trip_failures.lines;
    assert!(
        lines.len() == 1 && lines[0].contains(&format!("lane {lane} on edge 1")),
        "it takes the sidewalk it stands on: {lines:#?}"
    );
    let step = (agents.pos_x[idx] - pos.x).hypot(agents.pos_y[idx] - pos.y);
    assert!(step < 1.0, "it does not move to take it: {step} m");
    assert!(
        walk_until(&mut block, &mut agents, idx, 300.0, |a| at_home(a, idx)),
        "it walks home"
    );
    assert!(!teleported(&agents), "{:#?}", agents.trip_failures.lines);
    let lines = &agents.trip_failures.lines;
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(
        lines[0].contains("gives up and walks home to building 0"),
        "{lines:#?}"
    );
}

#[test]
fn a_walker_at_a_node_with_no_sidewalk_is_stranded_there_reported_and_walks_home_later() {
    let mut block = Block::new(false, false);
    let node = block.add_spur(false);
    let mut agents = AgentSystem::new();
    let pos = Vector2::new(-60.0, 0.0);
    let idx = block.walker_at_node(&mut agents, node, pos, 1);
    block.tick(&mut agents);
    assert!(stranded(&agents, idx));
    assert_eq!(agents.current_lane_id[idx], usize::MAX);
    assert_eq!(agents.current_node[idx], node);
    assert_eq!(
        agents.target_building[idx], 0,
        "it gave its trip up for home"
    );
    let stranded_at = agents.sim_time;
    let mut plans = vec![agents.next_replan_time[idx]];
    while plans.len() < 3 {
        block.tick(&mut agents);
        assert!(stranded(&agents, idx));
        assert_eq!((agents.pos_x[idx], agents.pos_y[idx]), (pos.x, pos.y));
        if agents.next_replan_time[idx] != *plans.last().unwrap() {
            plans.push(agents.next_replan_time[idx]);
        }
    }
    let waits = [
        plans[0] - stranded_at,
        plans[1] - plans[0],
        plans[2] - plans[1],
    ];
    for (wait, expected) in waits.iter().zip([5.0, 30.0, 120.0]) {
        assert!((wait - expected).abs() < 0.15, "waits {waits:?}");
    }
    agents.refresh_stranded_report(&block.network, &block.graph);
    assert_eq!(agents.stranded_report().junctions, vec![(node, 1)]);
    let lines = &agents.trip_failures.lines;
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(lines[0].contains("stranded with no way home"), "{lines:#?}");

    // The spur gets its sidewalks: the next plan home succeeds and it walks home.
    block.graph.edge_mut(4).allowed_types = TransitFlags::CAR | TransitFlags::FOOT;
    block.graph.edge_mut(4).sidewalks = crate::simulation::network::graph::SidewalkSides::BOTH;
    block.relane();
    agents.next_replan_time[idx] = agents.sim_time;
    assert!(
        walk_until(&mut block, &mut agents, idx, 400.0, |a| at_home(a, idx)),
        "it walks home"
    );
    assert!(!teleported(&agents), "{:#?}", agents.trip_failures.lines);
    assert_eq!(agents.trip_failures.lines.len(), 1, "no record per retry");
}

impl Block {
    // Removes road `edge`, as a bulldoze leaves it: out of the R-tree, deleted, the lanes rebuilt.
    fn remove_road(&mut self, edge: usize) {
        self.graph.remove_from_spatial_index(edge);
        self.graph.edge_mut(edge).deleted = true;
        self.graph.rebuild_adjacency_list();
        self.relane();
    }
}

// The bulldoze case (`TRAFFIC-19` probe, 10k city: walkers 123-347 m along a deleted road, more
// than a road edit's 30 m reattach reach from every sidewalk): road 1 is removed under a walker
// halfway along it, 50 m from both its nodes. It walks across the ground to the nearest sidewalk
// at walking pace, steps onto it and walks to its shop from there.
#[test]
fn a_walker_off_every_sidewalk_walks_to_the_nearest_one_and_on_to_its_trip() {
    let mut block = Block::new(false, false);
    block.remove_road(1);
    let mut agents = AgentSystem::new();
    let pos = Vector2::new(100.0, 50.0);
    let idx = block.walker_at_node(&mut agents, 2, pos, 1);
    agents.access_flags[idx] = ACCESS_PLAN_VALID;
    block.tick(&mut agents);
    let lines = &agents.trip_failures.lines;
    assert!(
        lines.len() == 1 && lines[0].contains("m to the nearest one and plans from there"),
        "one record as it sets off: {lines:#?}"
    );
    assert!(!stranded(&agents, idx));
    // Off the road it moves at the access walk's pace, never further in a tick.
    let mut last = pos;
    let mut walked = 0.0;
    while agents.current_lane_id[idx] == usize::MAX {
        let now = Vector2::new(agents.pos_x[idx], agents.pos_y[idx]);
        let step = now.distance_to(last);
        assert!(
            step <= AGENT_WALK_SPEED_MS * 0.1 + 1e-3,
            "no teleport: {step} m in one tick"
        );
        walked += step;
        last = now;
        assert!(walked < 60.0, "it walks straight to the sidewalk");
        block.tick(&mut agents);
    }
    let lane = &block.network.lane_system.lanes[agents.current_lane_id[idx]];
    assert_eq!(lane.lane_type, LaneType::Foot);
    assert!(
        walked > 30.0,
        "it walked the distance to the sidewalk: {walked} m"
    );
    // It walks on from the middle of that sidewalk, where its pose keeps its sideways offset
    // until a lane's end drops it, as the evicted walker's does below: no step on the road longer.
    let mut arrived = false;
    for _ in 0..6000 {
        let on_road = matches!(agents.transit[idx], TRANSIT_NETWORK | TRANSIT_INTERSECTION);
        block.tick(&mut agents);
        let now = Vector2::new(agents.pos_x[idx], agents.pos_y[idx]);
        if on_road && matches!(agents.transit[idx], TRANSIT_NETWORK | TRANSIT_INTERSECTION) {
            let step = now.distance_to(last);
            assert!(step < 2.0, "no teleport: {step} m in one tick");
        }
        last = now;
        if agents.transit[idx] == TRANSIT_IN_BUILDING && agents.current_building[idx] == 1 {
            arrived = true;
            break;
        }
    }
    assert!(
        arrived,
        "it walks on to its shop: {:#?}",
        agents.trip_failures.lines
    );
    assert!(!teleported(&agents), "{:#?}", agents.trip_failures.lines);
    assert_eq!(
        agents.trip_failures.lines.len(),
        1,
        "{:#?}",
        agents.trip_failures.lines
    );
}

#[test]
fn a_walker_beyond_the_walk_bound_of_every_sidewalk_is_stranded_where_it_stands() {
    // It stands 300 m east of road 1, beyond the 200 m it would walk to a sidewalk, with its trip
    // still planned.
    let mut block = Block::new(false, false);
    let mut agents = AgentSystem::new();
    let pos = Vector2::new(400.0, 50.0);
    let idx = block.walker_at_node(&mut agents, 2, pos, 1);
    agents.access_flags[idx] = ACCESS_PLAN_VALID;
    for _ in 0..1000 {
        block.tick(&mut agents);
        assert!(stranded(&agents, idx));
        assert_eq!(agents.current_lane_id[idx], usize::MAX);
        assert_eq!(
            (agents.pos_x[idx], agents.pos_y[idx]),
            (pos.x, pos.y),
            "never moved"
        );
    }
    assert_eq!(
        agents.target_building[idx], 0,
        "it gave its trip up for home"
    );
    assert!(
        agents.network_replan_failures[idx] >= 2,
        "it waits on its backoff"
    );
    agents.refresh_stranded_report(&block.network, &block.graph);
    assert_eq!(agents.stranded_report().junctions, vec![(2, 1)]);
    let lines = &agents.trip_failures.lines;
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(
        lines[0].contains("no position on the road") && lines[0].contains("stranded"),
        "{lines:#?}"
    );
}

// Walkers left at nodes with no lane and on sidewalks of the crossing-free block, with trips
// in and out of reach, plus filler agents for the parallel pass. Returns each walker's final
// state, the report and the records.
fn crossings_off_run(workers: usize) -> (Vec<WalkerState>, Vec<(u32, u32)>, Vec<String>) {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let mut block = Block::new(true, false);
            block.add_spur(false);
            block.crossings(false);
            let mut agents = AgentSystem::new();
            let mut ids = Vec::new();
            for node in 0..4u32 {
                for (edge, inner) in [(node as usize, false), ((node as usize + 3) % 4, true)] {
                    for fwd in [false, true] {
                        let lane = block.side_lane(edge, inner, fwd);
                        if lane_terminal(&block, lane) != node {
                            continue;
                        }
                        let pos = block.lane_end(lane);
                        for target in [0, 1] {
                            ids.push(block.walker_at_node(&mut agents, node, pos, target));
                        }
                    }
                }
            }
            ids.push(block.walker_at_node(&mut agents, 4, Vector2::new(-60.0, 0.0), 1));
            for k in 0..6 {
                ids.push(block.walker(&mut agents, k % 2, 1.0 + 7.0 * k as f32));
            }
            for _ in 0..500 {
                agents.spawn_housed_agent(0, 0.0, 0.0);
            }
            let state_of = |agents: &AgentSystem, i: usize| {
                (
                    agents.pos_x[i],
                    agents.pos_y[i],
                    agents.current_lane_id[i],
                    agents.transit[i],
                    agents.current_node[i],
                )
            };
            let mut last: Vec<_> = ids.iter().map(|&i| state_of(&agents, i)).collect();
            for tick in 0..3000 {
                block.tick(&mut agents);
                for (k, &i) in ids.iter().enumerate() {
                    let now = state_of(&agents, i);
                    let step = (now.0 - last[k].0).hypot(now.1 - last[k].1);
                    // On the road throughout (entering a building starts its door path); a
                    // walker's pose shifts sideways by its own offset as it leaves a lane end.
                    let on_road =
                        |transit| matches!(transit, TRANSIT_NETWORK | TRANSIT_INTERSECTION);
                    assert!(
                        tick == 0 || !on_road(last[k].3) || !on_road(now.3) || step < 2.0,
                        "walker {i} tick {tick}: {step} m in one tick, {:?} -> {now:?}, {:#?}",
                        last[k],
                        agents.trip_failures.lines
                    );
                    last[k] = now;
                }
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
            (
                state,
                agents.stranded_report().junctions.clone(),
                agents.trip_failures.lines.clone(),
            )
        })
}

fn lane_terminal(block: &Block, lane: usize) -> u32 {
    let value = &block.network.lane_system.lanes[lane];
    let edge = block.graph.edge(value.edge_id);
    if value.is_fwd {
        edge.end_node
    } else {
        edge.start_node
    }
}

#[test]
fn no_walker_teleports_in_a_crossing_free_block_and_the_run_is_the_same_for_1_and_8_workers() {
    let serial = crossings_off_run(1);
    assert!(
        !serial
            .2
            .iter()
            .any(|line| line.contains("sent home") || line.contains("sent to border")),
        "{:#?}",
        serial.2
    );
    assert!(
        serial.1.iter().any(|&(node, _)| node == 4),
        "the walker at the sidewalk-free node is reported there: {:?}",
        serial.1
    );
    assert_eq!(serial, crossings_off_run(8));
}

// The traced end-of-path case (`TRAFFIC-19`, 100k city, agent 140399): a resident walking home
// loses that home to an eviction mid-lane, so its trip and route are cleared there. It walks to
// the end of its lane and keeps the lane, where the control dropped it and left the walker at
// the node. It waits there reported as waiting for a home (round T19b). Rehoused, it plans from
// that sidewalk and walks to its new home.
#[test]
fn an_evicted_walker_keeps_its_lane_at_its_path_end_and_walks_from_it_when_rehoused() {
    let mut block = Block::new(false, true);
    let mut agents = AgentSystem::new();
    let idx = block.walker(&mut agents, 0, 20.0);
    let lane = agents.current_lane_id[idx];
    block.tick(&mut agents);
    assert!(
        agents.access_flags[idx] & ACCESS_PLAN_VALID != 0,
        "it plans the walk home"
    );
    agents.evict_household_member_home(idx, 0);
    assert_eq!(agents.target_building[idx], usize::MAX);
    // It walks on to the end of its lane (where its pose drops its sideways offset) and waits.
    for _ in 0..400 {
        block.tick(&mut agents);
    }
    let length = block.network.lane_system.lanes[lane].length;
    assert_eq!(
        (agents.current_lane_id[idx], agents.lane_distance[idx]),
        (lane, length),
        "it waits at the end of its lane, on it"
    );
    assert_eq!(agents.transit[idx], TRANSIT_NETWORK);
    // It waits reported, as a resident waiting for a home, not a stranded walker.
    agents.refresh_stranded_report(&block.network, &block.graph);
    assert_eq!(agents.stranded_report().waiting_for_home, 1);
    assert!(agents.stranded_report().junctions.is_empty());
    // Rehoused in the shop's building across the block, which the crossings reach.
    agents.relocate_household_member_home(idx, 0, 1, false);
    assert_eq!(agents.target_building[idx], 1);
    agents.refresh_stranded_report(&block.network, &block.graph);
    assert_eq!(agents.stranded_report().waiting_for_home, 0);
    // It sets off from the end of its lane, where its pose regains its sideways offset: no step
    // on the road longer than that.
    let mut last = (agents.pos_x[idx], agents.pos_y[idx]);
    let mut arrived = false;
    for _ in 0..4000 {
        let on_road =
            agents.transit[idx] == TRANSIT_NETWORK || agents.transit[idx] == TRANSIT_INTERSECTION;
        block.tick(&mut agents);
        let pos = (agents.pos_x[idx], agents.pos_y[idx]);
        if on_road && matches!(agents.transit[idx], TRANSIT_NETWORK | TRANSIT_INTERSECTION) {
            let step = (pos.0 - last.0).hypot(pos.1 - last.1);
            assert!(step < 2.0, "no teleport: {step} m in one tick");
        }
        last = pos;
        if agents.transit[idx] == TRANSIT_IN_BUILDING && agents.current_building[idx] == 1 {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "it walks to its new home");
    assert!(!teleported(&agents), "{:#?}", agents.trip_failures.lines);
}

// `TRAFFIC-24` (10k contract city, resident 2750): a walker whose job ends on its way to work
// (`assign_work_building` clears its destination, exit and route mid-lane) walks to the end of its
// lane and, with a home, plans home from there and walks home: no record, since no trip failed,
// and no teleport. A shopper walking behind it on the same sidewalk passes; before, the first
// waited at its lane's end for good and the shopper queued behind it.
#[test]
fn a_walker_whose_job_ends_on_the_way_walks_home_from_its_lane_end_and_frees_the_sidewalk() {
    let mut block = Block::new(false, false);
    let mut agents = AgentSystem::new();
    let worker = block.walker(&mut agents, 1, 30.0);
    agents.work_building[worker] = 1;
    agents.activity[worker] = ACTIVITY_WORK;
    let shopper = block.walker(&mut agents, 1, 34.0);
    agents.activity[shopper] = ACTIVITY_SHOPPING;
    let lane = agents.current_lane_id[worker];
    let length = block.network.lane_system.lanes[lane].length;
    block.tick(&mut agents);
    assert!(
        [worker, shopper]
            .iter()
            .all(|&idx| agents.access_flags[idx] & ACCESS_PLAN_VALID != 0),
        "both plan the walk to building 1"
    );
    agents.assign_work_building(worker, usize::MAX, 0);
    assert_eq!(agents.target_building[worker], usize::MAX);
    assert_eq!(agents.access_flags[worker] & ACCESS_PLAN_VALID, 0);
    // It walks on to the end of its lane and plans home from there.
    assert!(
        walk_until(&mut block, &mut agents, worker, 60.0, |a| {
            a.target_building[worker] == 0
        }),
        "it sets off home"
    );
    assert_eq!(
        (agents.current_lane_id[worker], agents.lane_distance[worker]),
        (lane, length),
        "from the end of its lane"
    );
    assert_eq!(agents.activity[worker], ACTIVITY_HOME);
    assert!(agents.access_flags[worker] & ACCESS_PLAN_VALID != 0);
    let in_building = |a: &AgentSystem, idx: usize, b: usize| {
        a.transit[idx] == TRANSIT_IN_BUILDING && a.current_building[idx] == b
    };
    let (mut home, mut shop) = (false, false);
    assert!(
        walk_until(&mut block, &mut agents, worker, 400.0, |a| {
            home |= at_home(a, worker);
            shop |= in_building(a, shopper, 1);
            home && shop
        }),
        "it walks home and the shopper behind it reaches the shop"
    );
    assert!(
        agents.trip_failures.lines.is_empty(),
        "{:#?}",
        agents.trip_failures.lines
    );
}

// `TRAFFIC-29`, off a parking leg: a driver whose job ends on the way drives on to the end of its
// lane, turns for home from there and drives in, with no record since no trip failed.
#[test]
fn a_driver_whose_job_ends_on_the_way_drives_home_from_its_lane_end() {
    let mut block = Block::new(false, false);
    let mut agents = AgentSystem::new();
    let idx = block.driver(&mut agents, 1);
    agents.work_building[idx] = 1;
    agents.activity[idx] = ACTIVITY_WORK;
    block.tick(&mut agents);
    assert!(agents.access_flags[idx] & ACCESS_PLAN_VALID != 0, "it plans the drive");
    agents.assign_work_building(idx, usize::MAX, 0);
    assert_eq!(agents.target_building[idx], usize::MAX);
    for _ in 0..600 {
        block.tick(&mut agents);
        if agents.target_building[idx] == 0 {
            break;
        }
    }
    assert_eq!(agents.target_building[idx], 0, "it turns for home");
    assert_eq!(agents.activity[idx], ACTIVITY_HOME);
    for _ in 0..3_000 {
        block.tick(&mut agents);
        if at_home(&agents, idx) {
            break;
        }
    }
    assert!(at_home(&agents, idx), "it drives home");
    assert!(
        agents.trip_failures.lines.is_empty(),
        "{:#?}",
        agents.trip_failures.lines
    );
}

// With no home it stands off the lane, out of the way, until its household's next one gives it a
// destination; it drives there from where it stands.
#[test]
fn a_driver_with_no_home_whose_job_ends_on_the_way_stands_off_the_lane_until_rehoused() {
    let mut block = Block::new(false, false);
    let mut agents = AgentSystem::new();
    let idx = block.driver(&mut agents, 1);
    agents.home_building[idx] = usize::MAX;
    agents.work_building[idx] = 1;
    agents.activity[idx] = ACTIVITY_WORK;
    block.tick(&mut agents);
    agents.assign_work_building(idx, usize::MAX, 0);
    for _ in 0..600 {
        block.tick(&mut agents);
    }
    assert_eq!(agents.current_lane_id[idx], usize::MAX);
    assert_eq!(agents.target_building[idx], usize::MAX);
    agents.relocate_household_member_home(idx, usize::MAX, 0, false);
    for _ in 0..3_000 {
        block.tick(&mut agents);
        if at_home(&agents, idx) {
            break;
        }
    }
    assert!(at_home(&agents, idx), "it drives to its new home");
}

// `TRAFFIC-24`: a walker with no home whose job ends on the way keeps waiting at the end of its
// lane for its household's next home (round T19b), with no plan and no record.
#[test]
fn a_walker_with_no_home_whose_job_ends_on_the_way_waits_at_its_lane_end() {
    let mut block = Block::new(false, false);
    let mut agents = AgentSystem::new();
    let idx = block.walker(&mut agents, 1, 20.0);
    agents.home_building[idx] = usize::MAX;
    agents.work_building[idx] = 1;
    agents.activity[idx] = ACTIVITY_WORK;
    let lane = agents.current_lane_id[idx];
    block.tick(&mut agents);
    agents.assign_work_building(idx, usize::MAX, 0);
    for _ in 0..400 {
        block.tick(&mut agents);
    }
    assert_eq!(
        (agents.current_lane_id[idx], agents.lane_distance[idx]),
        (lane, block.network.lane_system.lanes[lane].length),
        "it waits at the end of its lane, on it"
    );
    assert_eq!(agents.target_building[idx], usize::MAX);
    assert_eq!(agents.access_flags[idx], 0, "no plan, not stranded");
    assert!(agents.trip_failures.lines.is_empty());
    // The stranded report counts it as waiting for a home.
    agents.refresh_stranded_report(&block.network, &block.graph);
    assert_eq!(agents.stranded_report().waiting_for_home, 1);
}
