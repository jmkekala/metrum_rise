// SPDX-License-Identifier: GPL-2.0-only

//! Footpaths joining sidewalks and each other at junctions.

use super::super::super::graph::{Edge, RegionGraph};
use super::super::super::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use super::super::{LaneSystem, LaneType};
use crate::config;
use godot::prelude::*;
use std::collections::{HashSet, VecDeque};

const ASPHALT_HALF_WIDTH_M: f32 = config::LANE_WIDTH;

fn add_edge(graph: &mut RegionGraph, start: u32, end: u32, footpath: bool) -> usize {
    let p0 = graph.node(start).pos;
    let p1 = graph.node(end).pos;
    graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: if footpath {
            TransitType::Foot
        } else {
            TransitType::Road
        },
        allowed_types: if footpath {
            TransitFlags::FOOT
        } else {
            TransitFlags::CAR | TransitFlags::FOOT
        },
        class: EdgeClass::Standard,
        width: if footpath { 2.0 } else { 7.0 },
        fwd_lanes: u8::from(!footpath),
        bkw_lanes: u8::from(!footpath),
        speed_limit: 13.9,
        base_cost: 1.0,
        physical_length: p0.distance_to(p1),
        geometry: vec![p0, p1],
        physical_geometry: vec![p0, p1],
        ..Default::default()
    })
}

struct Crossing {
    graph: RegionGraph,
    lanes: LaneSystem,
    center: u32,
    west: usize,
    east: usize,
    far_east: usize,
    path: usize,
}

// A through road west-east with a footpath leaving its north side at the centre node, and one
// more road beyond the east arm.
fn road_with_footpath() -> Crossing {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let west_end = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let east_end = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let far = graph.add_node(Vector3::new(200.0, 0.0, 0.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, 60.0), NodeType::Junction);
    let west = add_edge(&mut graph, center, west_end, false);
    let east = add_edge(&mut graph, center, east_end, false);
    let far_east = add_edge(&mut graph, east_end, far, false);
    let path = add_edge(&mut graph, center, north, true);
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);
    Crossing {
        graph,
        lanes,
        center,
        west,
        east,
        far_east,
        path,
    }
}

fn foot_lane(lanes: &LaneSystem, edge: usize, is_fwd: bool) -> usize {
    lanes.edge_lanes[&edge]
        .iter()
        .copied()
        .find(|&id| lanes.lanes[id].lane_type == LaneType::Foot && lanes.lanes[id].is_fwd == is_fwd)
        .expect("footpath lane")
}

fn reachable(lanes: &LaneSystem, from: usize, to: usize) -> bool {
    let mut seen = HashSet::from([from]);
    let mut queue = VecDeque::from([from]);
    while let Some(id) = queue.pop_front() {
        if id == to {
            return true;
        }
        for &next in lanes.lanes[id].next_lanes.iter() {
            if seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    false
}

#[test]
fn footpath_stops_at_the_sidewalk_and_joins_both_corners_without_crossing_the_road() {
    let c = road_with_footpath();
    // The footpath starts at the centre, so its backward lane arrives there.
    let arriving = foot_lane(&c.lanes, c.path, false);
    let leaving = foot_lane(&c.lanes, c.path, true);
    let end = *c.lanes.lanes[arriving].geometry.last().unwrap();
    let sidewalk_edge_m = ASPHALT_HALF_WIDTH_M + config::SIDEWALK_WIDTH;
    assert!(
        (end.z - sidewalk_edge_m).abs() < 0.01,
        "footpath ends at z = {}",
        end.z
    );
    assert_eq!(c.lanes.lanes[leaving].geometry[0], end);

    let mut joined_roads = HashSet::new();
    for &connector in c.lanes.lanes[arriving].next_lanes.iter() {
        let lane = &c.lanes.lanes[connector];
        assert_eq!(lane.node_id, c.center as usize);
        let target = &c.lanes.lanes[lane.next_lanes[0]];
        // North-side sidewalks only: reaching the south side needs a crosswalk instead.
        if target.geometry[0].z > 0.0 && target.edge_id != c.path {
            joined_roads.insert(target.edge_id);
            assert!(
                lane.geometry
                    .iter()
                    .all(|p| p.z >= ASPHALT_HALF_WIDTH_M - 0.01),
                "connector to the north sidewalk crosses the asphalt: {:?}",
                lane.geometry
            );
        }
    }
    assert_eq!(joined_roads, HashSet::from([c.west, c.east]));

    // A walker on the far east sidewalk can now walk up the footpath, and back.
    let far_sidewalk = c.lanes.edge_lanes[&c.far_east]
        .iter()
        .copied()
        .find(|&id| c.lanes.lanes[id].lane_type == LaneType::Foot && !c.lanes.lanes[id].is_fwd)
        .unwrap();
    assert!(reachable(&c.lanes, far_sidewalk, leaving));
    assert!(reachable(
        &c.lanes,
        arriving,
        c.lanes.edge_lanes[&c.far_east]
            .iter()
            .copied()
            .find(|&id| c.lanes.lanes[id].lane_type == LaneType::Foot && c.lanes.lanes[id].is_fwd)
            .unwrap()
    ));
}

#[test]
fn footpaths_meeting_only_each_other_connect_through_the_node() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let a = graph.add_node(Vector3::new(-50.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(0.0, 0.0, 50.0), NodeType::Junction);
    let to_a = add_edge(&mut graph, center, a, true);
    let to_b = add_edge(&mut graph, center, b, true);
    graph.rebuild_adjacency_list();
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);

    let arriving_from_a = foot_lane(&lanes, to_a, false);
    let leaving_to_b = foot_lane(&lanes, to_b, true);
    let end = *lanes.lanes[arriving_from_a].geometry.last().unwrap();
    assert!((end.distance_to(Vector3::ZERO) - 1.0).abs() < 0.01);
    assert!(
        lanes.lanes[arriving_from_a]
            .next_lanes
            .iter()
            .any(|&connector| lanes.lanes[connector].next_lanes[0] == leaving_to_b)
    );
}

#[test]
fn partial_rebuild_keeps_the_footpath_trim_and_its_corner_links() {
    let mut c = road_with_footpath();
    let geometry = |c: &Crossing| {
        [false, true].map(|fwd| {
            c.lanes.lanes[foot_lane(&c.lanes, c.path, fwd)]
                .geometry
                .clone()
        })
    };
    let before = geometry(&c);
    // Rebuilds the centre's connectors through the east arm, but not the footpath's lanes.
    c.lanes
        .rebuild_edges_incremental(&mut c.graph, &HashSet::from([c.far_east]));
    assert_eq!(geometry(&c), before);
    let arriving = foot_lane(&c.lanes, c.path, false);
    let targets: HashSet<_> = c.lanes.lanes[arriving]
        .next_lanes
        .iter()
        .map(|&connector| c.lanes.lanes[c.lanes.lanes[connector].next_lanes[0]].edge_id)
        .collect();
    assert!(targets.contains(&c.west) && targets.contains(&c.east));
}

#[test]
fn footpath_rules_keep_car_turns_open_through_a_full_rebuild() {
    // The first rebuild records the footpath's pedestrian routes on the centre node; a second
    // full rebuild (as on load) must still read the node as open to cars, not as whitelisted.
    let mut c = road_with_footpath();
    assert!(
        c.graph
            .node(c.center)
            .lane_connections
            .keys()
            .any(|&(edge_idx, _)| edge_idx == c.path),
        "the footpath's pedestrian routes are recorded on the node"
    );
    c.lanes.rebuild(&mut c.graph);
    let car_lane = |edge: usize, is_fwd: bool| {
        c.lanes.edge_lanes[&edge]
            .iter()
            .copied()
            .find(|&id| {
                c.lanes.lanes[id].lane_type == LaneType::Vehicle
                    && c.lanes.lanes[id].is_fwd == is_fwd
            })
            .unwrap()
    };
    // West runs centre -> west_end, east centre -> east_end: west's backward lane enters the
    // centre and east's forward lane leaves it.
    let (from, to) = (car_lane(c.west, false), car_lane(c.east, true));
    // A footpath arm leaves the road a pass-through for cars: the lanes meet end to start and
    // link directly. A junction connector between coincident ends would loop backwards.
    assert_eq!(
        &*c.lanes.lanes[from].next_lanes,
        &[to],
        "cars still drive straight through a junction with a footpath arm"
    );
    assert!(!c.graph.is_vehicle_turn_rule(c.path, 0));
    assert!(c.graph.is_vehicle_turn_rule(c.west, 0));
}
