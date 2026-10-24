// SPDX-License-Identifier: GPL-2.0-only

//! Footpaths joining sidewalks and each other at junctions. Each ends at its mouth in its node's
//! junction layout (D5).

use super::super::super::graph::{Edge, RegionGraph};
use super::super::super::roadbed::NodeArms;
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

// The mouth station of footpath `edge` in `node`'s layout: an attached arm's at a road node, an
// ordinary arm's where only footpaths meet.
fn footpath_mouth_m(graph: &RegionGraph, node: u32, edge: usize) -> f64 {
    let layout = NodeArms::collect(graph, node).unwrap().layout().unwrap();
    let arms = layout.arms.iter().map(|arm| (arm.key, arm.mouth_station_m));
    let attached = layout
        .attachments
        .iter()
        .map(|attached| (attached.key, attached.mouth_station_m));
    arms.chain(attached)
        .find(|(key, _)| key.edge == edge)
        .unwrap()
        .1
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
    // Its mouth lies a metre beyond the sidewalk's outer edge, where its ribbon meets it.
    let sidewalk_edge_m = ASPHALT_HALF_WIDTH_M + config::SIDEWALK_WIDTH;
    let mouth_m = footpath_mouth_m(&c.graph, c.center, c.path);
    assert_eq!(mouth_m, f64::from(sidewalk_edge_m) + 1.0);
    assert_eq!((end.x, f64::from(end.z)), (0.0, mouth_m), "footpath end");
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
    let mouth_m = footpath_mouth_m(&graph, center, to_a) as f32;
    assert!((end.distance_to(Vector3::ZERO) - mouth_m).abs() < 1e-5);
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
    // A footpath arm gives the node a layout (D5): cars drive through it on one junction
    // connector between the two mouths.
    let [connector] = *c.lanes.lanes[from].next_lanes else {
        panic!("one connector out of the west arm");
    };
    assert_eq!(
        &*c.lanes.lanes[connector].next_lanes,
        &[to],
        "cars still drive straight through a junction with a footpath arm"
    );
    assert!(!c.graph.is_vehicle_turn_rule(c.path, 0));
    assert!(c.graph.is_vehicle_turn_rule(c.west, 0));
}

// D5 fixtures: footpath lanes end at their layout mouths; at the hub every arriving foot lane
// reaches every leaving one on one connector; an attached footpath joins its sector's corner
// walk without touching the carriageway.
#[test]
fn footpath_junction_lanes_end_at_their_mouths_and_join_the_corner_walks() {
    use super::super::super::roadbed::fixtures::footpath_fixtures;
    use super::super::super::roadbed::{BandKind, EdgeEnd, NodeWalkways, RoadbedSystem};
    for (name, mut graph) in [false, true].into_iter().flat_map(footpath_fixtures) {
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        let mut roadbed = RoadbedSystem::new(64.0);
        roadbed.compile_dirty(&graph);
        let walkways = NodeWalkways::collect(&graph, 0).expect("hub walkways");
        let arriving_at_hub = |id: usize| {
            let lane = &lanes.lanes[id];
            let edge = graph.edge(lane.edge_id);
            lane.lane_type == LaneType::Foot
                && (if lane.is_fwd {
                    edge.end_node
                } else {
                    edge.start_node
                }) == 0
        };
        let leaving_hub = |id: usize| {
            let lane = &lanes.lanes[id];
            let edge = graph.edge(lane.edge_id);
            lane.lane_type == LaneType::Foot
                && (if lane.is_fwd {
                    edge.start_node
                } else {
                    edge.end_node
                }) == 0
        };
        let ids: Vec<usize> = graph
            .node_adjacency(0)
            .iter()
            .flat_map(|edge| lanes.edge_lanes[edge].iter().copied())
            .collect();
        for &arriving in ids.iter().filter(|&&id| arriving_at_hub(id)) {
            let targets: HashSet<usize> = lanes.lanes[arriving]
                .next_lanes
                .iter()
                .map(|&connector| lanes.lanes[connector].next_lanes[0])
                .collect();
            for &leaving in ids.iter().filter(|&&id| leaving_hub(id)) {
                assert!(
                    targets.contains(&leaving),
                    "{name}: {arriving} -> {leaving}"
                );
            }
            let lane = &lanes.lanes[arriving];
            if graph.edge(lane.edge_id).primary_type != TransitType::Foot {
                continue;
            }
            let end = if graph.edge(lane.edge_id).start_node == 0 {
                EdgeEnd::Start
            } else {
                EdgeEnd::End
            };
            let key = super::super::super::roadbed::ArmKey {
                edge: lane.edge_id,
                end,
            };
            let mouth = walkways.footpath_end(key).expect("footpath end").point;
            let last = *lane.geometry.last().unwrap();
            assert_eq!((last.x, last.z), (mouth.x as f32, mouth.z as f32), "{name}");
            let Some(join) = walkways.footpath_join(key) else {
                continue;
            };
            let corner = join.corner;
            let ends = [corner.points[0], corner.points[corner.points.len() - 1]]
                .map(|point| (point.x as f32, point.z as f32));
            let mut joins = 0;
            for &connector in lane.next_lanes.iter() {
                let geometry = &lanes.lanes[connector].geometry;
                let last = geometry[geometry.len() - 1];
                if !ends.contains(&(last.x, last.z)) {
                    continue;
                }
                joins += 1;
                for pair in geometry.windows(2) {
                    for step in 0..=8 {
                        let point = pair[0].lerp(pair[1], step as f32 / 8.0);
                        let band = roadbed.band_at(point.x, point.z);
                        assert!(
                            band.is_some() && band != Some(BandKind::Carriageway),
                            "{name}: join at {point:?} on {band:?}"
                        );
                    }
                }
            }
            assert!(
                joins >= 2,
                "{name}: joins both sidewalk ends, {joins} of {:?} {:?}",
                ends,
                lane.next_lanes
                    .iter()
                    .map(|&c| *lanes.lanes[c].geometry.last().unwrap())
                    .collect::<Vec<_>>()
            );
        }
    }
}
