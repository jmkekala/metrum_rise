// SPDX-License-Identifier: GPL-2.0-only

//! Junction crossings and sidewalk corners from the junction layout (`ROAD-68-C`): zebras right
//! outside the curb returns, shared kerb points with the node pieces, sidewalk lanes ending on the
//! crossing line, corner walks on the sidewalk, and repeatable rebuilds. Degree-two nodes cross
//! once, on the legacy-ordered arm (D8): a bend on its node piece, a pass-through node on the
//! span beside it, with kerb points from the span's own section row.

use super::super::super::graph::RegionGraph;
use super::super::super::roadbed::fixtures::{bend, connect, point};
use super::super::super::roadbed::{
    BandKind, EdgeEnd, LayoutError, NodeError, NodePiece, RoadSection, RoadbedSystem,
};
use super::super::super::types::NodeType;
use super::super::{CrosswalkMarking, LaneSystem, LaneType};
use godot::prelude::*;
use std::collections::{BTreeSet, HashSet};

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
        // A bend crosses once, on its first arm by angle in (-180, 180] degrees (D8).
        add("bend_45", star(&[15.0, 60.0], sloped));
        add("bend_across_west", star(&[170.0, 210.0], sloped));
    }
    set
}

// Pass-through nodes at the hub: straight both ways, and bent by 8 degrees.
fn pass_through_fixtures() -> Vec<(String, RegionGraph)> {
    let mut set = Vec::new();
    for sloped in [false, true] {
        for (name, angles) in [
            ("north_south", [90.0, 270.0]),
            ("south_north", [270.0, 90.0]),
            ("bent", [80.0, 252.0]),
        ] {
            set.push((
                format!("pass_through_{name}/{sloped}"),
                star(&angles, sloped),
            ));
        }
    }
    set
}

// The hub's arm that crosses by default (D8): least direction angle in (-pi, pi], then least
// edge index. An oracle independent of the roadbed's arm collection.
fn legacy_first_edge(graph: &RegionGraph) -> usize {
    let hub = graph.node(HUB).pos;
    let angle = |edge: usize| {
        let edge = graph.edge(edge);
        let points = &edge.physical_geometry;
        let next = if edge.start_node == HUB {
            points[1]
        } else {
            points[points.len() - 2]
        };
        f64::from(next.z - hub.z).atan2(f64::from(next.x - hub.x))
    };
    let mut edges = graph.node_adjacency(HUB).to_vec();
    edges.sort_by(|&a, &b| angle(a).total_cmp(&angle(b)).then(a.cmp(&b)));
    edges[0]
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
            if arm.crosswalk.is_some() {
                check_crossing_walks(&name, &lanes, arm.key.edge);
            }
        }
    }
}

// Both directions of the hub's crossing on `edge` walk its marking's kerb segment, from and to
// sidewalk lane ends on the kerb line.
fn check_crossing_walks(name: &str, lanes: &LaneSystem, edge: usize) {
    let marking = marking_for(lanes, edge);
    let crossings: Vec<usize> = lanes.node_lanes[&(HUB as usize)]
        .iter()
        .copied()
        .filter(|&lane_id| lanes.crosswalk(lane_id).map(|c| c.edge_id) == Some(edge))
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
            .position(|lane| lane.edge_id == edge && lane.next_lanes.contains(&connector_id))
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

#[test]
fn corner_walks_stay_on_the_sidewalk() {
    for (name, mut graph) in fixtures().into_iter().chain(pass_through_fixtures()) {
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
    for (name, mut graph) in fixtures().into_iter().chain(pass_through_fixtures()) {
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

// The edges whose arms cross at the hub, from the lanes' markings.
fn crossing_edges(lanes: &LaneSystem) -> BTreeSet<usize> {
    markings(lanes)
        .iter()
        .map(|marking| marking.edge_id)
        .collect()
}

fn hub_end(graph: &RegionGraph, edge: usize) -> EdgeEnd {
    if graph.edge(edge).start_node == HUB {
        EdgeEnd::Start
    } else {
        EdgeEnd::End
    }
}

#[test]
fn degree_two_nodes_cross_once_on_the_legacy_first_arm() {
    let mut bends = Vec::new();
    for sloped in [false, true] {
        for angles in [[15.0, 60.0], [170.0, 210.0], [100.0, 160.0], [300.0, 200.0]] {
            bends.push((format!("bend_{angles:?}/{sloped}"), star(&angles, sloped)));
        }
    }
    for (name, mut graph) in bends.into_iter().chain(pass_through_fixtures()) {
        let (lanes, roadbed) = built(&mut graph);
        let first = legacy_first_edge(&graph);
        assert_eq!(crossing_edges(&lanes), BTreeSet::from([first]), "{name}");
        match roadbed.node_piece(HUB) {
            // A bend's crossing is its layout's zebra strip.
            Some(piece) => {
                let crossable: Vec<usize> = piece
                    .layout()
                    .arms
                    .iter()
                    .filter(|arm| arm.crosswalk.is_some())
                    .map(|arm| arm.key.edge)
                    .collect();
                assert_eq!(crossable, [first], "{name}");
            }
            // A pass-through node's crossing lies on its arm's span, the other span has none.
            None => {
                for edge in graph.node_adjacency(HUB).to_vec() {
                    let strip = roadbed
                        .span_piece(edge)
                        .and_then(|span| span.crossing_strip_m(hub_end(&graph, edge)));
                    assert_eq!(strip.is_some(), edge == first, "{name}: edge {edge}");
                }
            }
        }
    }
}

// Kerb points of a span section row: its carriageway-edge vertices at ground level, as the span
// mesh places them (`center + lateral * offset`, the band's evaluated height).
fn section_kerbs(section: &RoadSection) -> [Vector3; 2] {
    let bands = &section.bands;
    let first = bands
        .iter()
        .position(|band| band.kind == BandKind::Carriageway)
        .unwrap();
    let last = bands
        .iter()
        .rposition(|band| band.kind == BandKind::Carriageway)
        .unwrap();
    // A raised band beyond each edge, so the edge vertex is the carriageway's own.
    assert!(first > 0 && last + 1 < bands.len());
    let vertex = |lateral_m: f32, height_m: f32| {
        let p = section.center_xz + section.lateral_xz * f64::from(lateral_m);
        Vector3::new(p.x as f32, height_m, p.y as f32)
    };
    [
        vertex(bands[first].lateral_start_m, bands[first].height_start_m),
        vertex(bands[last + 1].lateral_start_m, bands[last].height_end_m),
    ]
}

#[test]
fn pass_through_crossing_lies_beside_the_node_on_the_span_row() {
    for (name, mut graph) in pass_through_fixtures() {
        let (lanes, roadbed) = built(&mut graph);
        assert!(roadbed.node_piece(HUB).is_none(), "{name}: no node piece");
        let edge = legacy_first_edge(&graph);
        let marking = marking_for(&lanes, edge);
        let span = roadbed.span_piece(edge).expect("span");
        let sections = span.sections();
        let end = hub_end(&graph, edge);
        // From the node's mouth out to 3 m, with a section row on its middle.
        let (low, high) = span.crossing_strip_m(end).expect("crossing strip");
        assert_eq!(high - low, 3.0, "{name}");
        match end {
            EdgeEnd::Start => assert_eq!(low, sections[0].s_m, "{name}"),
            EdgeEnd::End => assert_eq!(high, sections[sections.len() - 1].s_m, "{name}"),
        }
        let line_s = 0.5 * (low + high);
        let section = sections
            .iter()
            .find(|section| section.s_m == line_s)
            .expect("section row on the crossing line");
        // Its kerb points are that row's vertices, bitwise.
        let mut lane_kerbs = [bits(marking.start), bits(marking.end)];
        let mut row_kerbs = section_kerbs(section).map(bits);
        lane_kerbs.sort_unstable();
        row_kerbs.sort_unstable();
        assert_eq!(lane_kerbs, row_kerbs, "{name}");
        // Square to the road, right beside the node.
        let across = xz(marking.end) - xz(marking.start);
        let lateral = Vector2::new(section.lateral_xz.x as f32, section.lateral_xz.y as f32);
        assert!(across.normalized().cross(lateral).abs() < 1e-5, "{name}");
        let node = xz(graph.node(HUB).pos);
        let middle = (xz(marking.start) + xz(marking.end)) * 0.5;
        assert!((middle.distance_to(node) - 1.5).abs() < 0.01, "{name}");
        // The strip lies on the carriageway.
        let along = Vector2::new(-across.y, across.x).normalized();
        for (t, s) in [(0.02, -1.45), (0.98, -1.45), (0.02, 1.45), (0.98, 1.45)] {
            let corner = xz(marking.start) + across * t + along * s;
            assert_eq!(
                roadbed.band_at(corner.x, corner.y),
                Some(BandKind::Carriageway),
                "{name}: zebra strip corner {corner:?} off the carriageway"
            );
        }
        check_crossing_walks(&name, &lanes, edge);
    }
}

#[test]
fn crossing_overrides_at_degree_two_nodes_follow_the_legacy_rule() {
    // Node overrides by edge, and the edges that then cross; both fixtures cross on edge 0 by
    // default.
    type Case = (&'static [(usize, bool)], &'static [usize]);
    let cases: [Case; 5] = [
        (&[], &[0]),
        (&[(0, true)], &[0]),
        // `false` on the first arm moves the default crossing to the other arm.
        (&[(0, false)], &[1]),
        (&[(0, false), (1, false)], &[]),
        (&[(1, true)], &[0, 1]),
    ];
    for sloped in [false, true] {
        for (fixture, angles) in [("bend", [15.0, 60.0]), ("pass_through", [270.0, 90.0])] {
            for (overrides, expected) in cases {
                let name = format!("{fixture}/{sloped}: {overrides:?}");
                let mut graph = star(&angles, sloped);
                for &(edge, enabled) in overrides {
                    graph.set_crosswalk_override(HUB, edge, enabled);
                }
                graph.rebuild_intersection_clips();
                let (lanes, roadbed) = built(&mut graph);
                let expected: BTreeSet<usize> = expected.iter().copied().collect();
                assert_eq!(crossing_edges(&lanes), expected, "{name}");
                for edge in [0, 1] {
                    let crosses = match roadbed.node_piece(HUB) {
                        Some(piece) => piece
                            .layout()
                            .arms
                            .iter()
                            .any(|arm| arm.key.edge == edge && arm.crosswalk.is_some()),
                        None => roadbed
                            .span_piece(edge)
                            .and_then(|span| span.crossing_strip_m(hub_end(&graph, edge)))
                            .is_some(),
                    };
                    assert_eq!(crosses, expected.contains(&edge), "{name}: edge {edge}");
                }
                for &edge in &expected {
                    check_crossing_walks(&name, &lanes, edge);
                }
            }
        }
    }
}

// Open question 13: a bend's default crossing that its first arm cannot hold moves to the other
// arm. A 15 m arm leaves its end 7.5 m: room for the 3.5 m corner (tangent at 7 m) but not for the
// strip beyond even the smallest one (5.5 m + 3 m).
#[test]
fn a_bend_crossing_that_does_not_fit_on_its_first_arm_moves_to_the_other_arm() {
    for sloped in [false, true] {
        let mut graph = bend(15.0, 120.0, sloped);
        assert_eq!(legacy_first_edge(&graph), 0);
        let (lanes, roadbed) = built(&mut graph);
        assert_eq!(crossing_edges(&lanes), BTreeSet::from([1]), "{sloped}");
        let piece = hub_piece(&roadbed);
        for arm in &piece.layout().arms {
            assert_eq!(arm.crosswalk.is_some(), arm.key.edge == 1, "{sloped}");
            // The short arm keeps its full-radius corner.
            if arm.key.edge == 0 {
                assert_eq!(arm.mouth_station_m, 7.0, "{sloped}");
            }
        }
        check_crossing_walks(&format!("{sloped}"), &lanes, 1);
        // With room on both arms the first arm crosses, as before.
        let (lanes, _) = built(&mut bend(120.0, 120.0, sloped));
        assert_eq!(crossing_edges(&lanes), BTreeSet::from([0]), "{sloped}");
    }
}

#[test]
fn a_bend_that_holds_its_crossing_on_neither_arm_is_rejected() {
    for sloped in [false, true] {
        let mut graph = bend(15.0, 15.0, sloped);
        let mut roadbed = RoadbedSystem::new(64.0);
        roadbed.compile_dirty(&graph);
        assert_eq!(roadbed.failed_nodes(), [HUB], "{sloped}");
        // The first arm's reason is reported.
        let first = super::super::super::roadbed::ArmKey {
            edge: 0,
            end: EdgeEnd::Start,
        };
        assert_eq!(
            roadbed.node_error(HUB),
            Some(NodeError::Layout(LayoutError::CornerDoesNotFit { arm: first })),
            "{sloped}"
        );
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        assert!(crossing_edges(&lanes).is_empty(), "{sloped}");
    }
}
