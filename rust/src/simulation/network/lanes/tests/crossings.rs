// SPDX-License-Identifier: GPL-2.0-only

//! Junction crossings and sidewalk corners from the junction layout (`ROAD-68-C`): zebras right
//! outside the curb returns, shared kerb points with the node pieces, sidewalk lanes ending on the
//! crossing line, corner walks on the sidewalk, and repeatable rebuilds.

use super::super::super::graph::RegionGraph;
use super::super::super::roadbed::fixtures::{connect, point};
use super::super::super::roadbed::{BandKind, NodePiece, RoadbedSystem};
use super::super::super::types::NodeType;
use super::super::{CrosswalkMarking, LaneSystem, LaneType};
use godot::prelude::*;
use std::collections::HashSet;

const ARM_M: f32 = 120.0;
const HUB: u32 = 0;

// A two-way two-lane road with sidewalks from the hub at each angle (degrees), each arm ending at
// a terminal; even arms leave the hub from their edge's start, odd ones from its end.
fn star(angles: &[f32], sloped: bool) -> RegionGraph {
    let mut graph = RegionGraph::new();
    let hub = graph.add_node(point(12.5, -7.25, sloped), NodeType::Junction);
    for (index, &angle) in angles.iter().enumerate() {
        let (sin, cos) = angle.to_radians().sin_cos();
        let far = point(12.5 + cos * ARM_M, -7.25 + sin * ARM_M, sloped);
        let far = graph.add_node(far, NodeType::Junction);
        let edge = if index % 2 == 0 {
            connect(&mut graph, hub, far, 7.0, sloped)
        } else {
            connect(&mut graph, far, hub, 7.0, sloped)
        };
        let edge = graph.edge_mut(edge);
        edge.fwd_lanes = 1;
        edge.bkw_lanes = 1;
    }
    graph.rebuild_intersection_clips();
    graph
}

fn fixtures() -> Vec<(String, RegionGraph)> {
    let mut set = Vec::new();
    for sloped in [false, true] {
        let mut add =
            |name: &str, graph: RegionGraph| set.push((format!("{name}/{sloped}"), graph));
        add("t", star(&[0.0, 180.0, 270.0], sloped));
        add("four_way", star(&[0.0, 90.0, 180.0, 270.0], sloped));
        add(
            "six_way",
            star(&[0.0, 60.0, 120.0, 180.0, 240.0, 300.0], sloped),
        );
        add("y_40", star(&[270.0, 70.0, 110.0], sloped));
        // A bend crosses only where its node forces it (D8).
        let mut bend = star(&[15.0, 60.0], sloped);
        bend.set_crosswalk_override(HUB, 0, true);
        bend.rebuild_intersection_clips();
        add("forced_bend_45", bend);
    }
    set
}

// Lanes first, then the roadbed: neither depends on the other's having run.
fn built(graph: &mut RegionGraph) -> (LaneSystem, RoadbedSystem) {
    let mut lanes = LaneSystem::new();
    lanes.rebuild(graph);
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(graph);
    assert!(roadbed.failed_nodes().is_empty());
    (lanes, roadbed)
}

fn hub_piece(roadbed: &RoadbedSystem) -> &NodePiece {
    roadbed.node_piece(HUB).expect("hub node piece")
}

fn markings(lanes: &LaneSystem) -> Vec<CrosswalkMarking> {
    lanes.node_lanes[&(HUB as usize)]
        .iter()
        .filter_map(|&lane_id| lanes.crosswalk_marking(lane_id))
        .collect()
}

fn marking_for(lanes: &LaneSystem, edge: usize) -> CrosswalkMarking {
    let found: Vec<_> = markings(lanes)
        .into_iter()
        .filter(|marking| marking.edge_id == edge)
        .collect();
    assert_eq!(found.len(), 1, "one zebra per crossable arm");
    found[0]
}

fn bits(point: Vector3) -> [u32; 3] {
    [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()]
}

fn xz(point: Vector3) -> Vector2 {
    Vector2::new(point.x, point.z)
}

#[test]
fn zebras_fill_the_straight_approach_right_outside_the_corners() {
    for (name, mut graph) in fixtures() {
        let (lanes, roadbed) = built(&mut graph);
        let piece = hub_piece(&roadbed);
        let mut crossable = 0;
        for arm in &piece.layout().arms {
            let Some(crosswalk) = arm.crosswalk else {
                continue;
            };
            crossable += 1;
            // The strip's inner edge is where the farther of the arm's two curb returns ends.
            let tangents: Vec<f64> = [arm.left_corner, arm.right_corner]
                .into_iter()
                .flatten()
                .map(|corner| corner.station_m)
                .collect();
            assert!(!tangents.is_empty(), "{name}");
            assert_eq!(
                crosswalk.inner_station_m,
                tangents.iter().copied().fold(1.0, f64::max),
                "{name}"
            );
            assert_eq!(
                crosswalk.station_m,
                crosswalk.inner_station_m + 1.5,
                "{name}"
            );
            assert_eq!(
                arm.mouth_station_m,
                crosswalk.inner_station_m + 3.0,
                "{name}"
            );
            // The 3 m strip lies on the carriageway, square to the road at the crossing line.
            let marking = marking_for(&lanes, arm.key.edge);
            let across = xz(marking.end) - xz(marking.start);
            let along = Vector2::new(-across.y, across.x).normalized();
            for (t, s) in [(0.02, -1.45), (0.98, -1.45), (0.02, 1.45), (0.98, 1.45)] {
                let corner = xz(marking.start) + across * t + along * s;
                assert_eq!(
                    roadbed.band_at(corner.x, corner.y),
                    Some(BandKind::Carriageway),
                    "{name}: zebra strip corner {corner:?} off the carriageway"
                );
            }
        }
        assert!(crossable > 0, "{name}");
        // No zebra anywhere else: every marking at the hub is a layout crossing.
        assert_eq!(markings(&lanes).len(), crossable, "{name}");
    }
}

#[test]
fn lane_kerb_points_are_the_node_piece_kerb_points_bitwise() {
    for (name, mut graph) in fixtures() {
        let (lanes, roadbed) = built(&mut graph);
        let piece = hub_piece(&roadbed);
        for arm in &piece.layout().arms {
            let Some(kerbs) = piece.crosswalk_kerbs(arm.key) else {
                continue;
            };
            let marking = marking_for(&lanes, arm.key.edge);
            let mut lane_kerbs = [bits(marking.start), bits(marking.end)];
            let mut piece_kerbs = kerbs.map(bits);
            lane_kerbs.sort_unstable();
            piece_kerbs.sort_unstable();
            assert_eq!(lane_kerbs, piece_kerbs, "{name}: arm {:?}", arm.key);
        }
    }
}

#[test]
fn sidewalk_lanes_end_on_the_crossing_line_and_both_directions_walk_the_kerb_segment() {
    for (name, mut graph) in fixtures() {
        let (lanes, roadbed) = built(&mut graph);
        for arm in &hub_piece(&roadbed).layout().arms {
            if arm.crosswalk.is_none() {
                continue;
            }
            let marking = marking_for(&lanes, arm.key.edge);
            let crossings: Vec<usize> = lanes.node_lanes[&(HUB as usize)]
                .iter()
                .copied()
                .filter(|&lane_id| {
                    lanes.crosswalk(lane_id).map(|c| c.edge_id) == Some(arm.key.edge)
                })
                .collect();
            assert_eq!(crossings.len(), 2, "{name}: two directions");
            assert_eq!(
                crossings
                    .iter()
                    .filter(|&&lane_id| lanes.crosswalk_marking(lane_id).is_some())
                    .count(),
                1,
                "{name}: one marking"
            );
            for &connector_id in &crossings {
                let connector = &lanes.lanes[connector_id];
                let geometry = &connector.geometry;
                assert_eq!(geometry.len(), 4, "{name}: lane end, kerb, kerb, lane end");
                let kerbs = [geometry[1], geometry[2]];
                assert!(
                    kerbs == [marking.start, marking.end] || kerbs == [marking.end, marking.start],
                    "{name}: a direction leaves the zebra's kerb segment"
                );
                // The walker leaves its sidewalk lane's last point, square across the kerb line.
                let inbound = lanes
                    .lanes
                    .iter()
                    .position(|lane| {
                        lane.edge_id == arm.key.edge && lane.next_lanes.contains(&connector_id)
                    })
                    .expect("inbound sidewalk lane");
                assert_eq!(lanes.lanes[inbound].lane_type, LaneType::Foot);
                assert_eq!(lanes.lanes[inbound].geometry.last(), Some(&geometry[0]));
                let target = &lanes.lanes[connector.next_lanes[0]];
                assert_eq!(target.geometry.first(), geometry.last(), "{name}");
                let line = xz(kerbs[1]) - xz(kerbs[0]);
                for end in [geometry[0], geometry[3]] {
                    let off_line = line.cross(xz(end) - xz(kerbs[0])).abs() / line.length();
                    assert!(
                        off_line < 1e-4,
                        "{name}: lane end {off_line} m off the kerb line"
                    );
                }
            }
        }
    }
}

#[test]
fn corner_walks_stay_on_the_sidewalk() {
    for (name, mut graph) in fixtures() {
        let (lanes, roadbed) = built(&mut graph);
        let kerbs: HashSet<[u32; 3]> = markings(&lanes)
            .iter()
            .flat_map(|marking| [bits(marking.start), bits(marking.end)])
            .collect();
        let on_kerb = |point: Vector3| kerbs.contains(&bits(point));
        let mut walks = 0;
        for &lane_id in &lanes.node_lanes[&(HUB as usize)] {
            let lane = &lanes.lanes[lane_id];
            if lane.lane_type != LaneType::Foot
                || lane.length <= 0.001
                || lanes.crosswalk(lane_id).is_some()
            {
                continue;
            }
            walks += 1;
            for pair in lane.geometry.windows(2) {
                let midpoint = (pair[0] + pair[1]) * 0.5;
                let checked = [
                    (!on_kerb(pair[0])).then_some(pair[0]),
                    (!(on_kerb(pair[0]) && on_kerb(pair[1]))).then_some(midpoint),
                ];
                for point in checked.into_iter().flatten() {
                    assert_eq!(
                        roadbed.band_at(point.x, point.z),
                        Some(BandKind::Sidewalk),
                        "{name}: walk {lane_id} leaves the sidewalk at {point:?}"
                    );
                }
            }
        }
        assert!(walks > 0, "{name}");
    }
}

// Every connector at the hub, as exact bits, in a canonical order.
fn hub_connectors(lanes: &LaneSystem) -> Vec<Vec<[u32; 3]>> {
    let mut connectors: Vec<Vec<[u32; 3]>> = lanes.node_lanes[&(HUB as usize)]
        .iter()
        .map(|&lane_id| {
            lanes.lanes[lane_id]
                .geometry
                .iter()
                .copied()
                .map(bits)
                .collect()
        })
        .collect();
    connectors.sort_unstable();
    connectors
}

#[test]
fn crossings_and_corner_walks_repeat_bitwise_through_full_and_local_rebuilds() {
    for (name, mut graph) in fixtures() {
        let (mut lanes, _) = built(&mut graph);
        let first = hub_connectors(&lanes);
        let mut again = LaneSystem::new();
        again.rebuild(&mut graph);
        assert_eq!(hub_connectors(&again), first, "{name}: full rebuild");
        // A local rebuild through one arm lays the hub out again from the same graph.
        lanes.rebuild_edges_incremental(&mut graph, &HashSet::from([0]));
        assert_eq!(hub_connectors(&lanes), first, "{name}: local rebuild");
    }
}
