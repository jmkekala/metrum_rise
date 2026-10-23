// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68-C` node-piece acceptance: the fixture set flat and on a 2.5% slope, watertight pieces
//! and mouths, heights on the node plane, mouths on the arms' offset curves, insertion-order
//! invariance, short edges between junctions and local rebuilds.

use super::*;
use crate::simulation::network::roadbed::fixtures::{connect, point, road_edge};
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem, SpanPiece};
use crate::simulation::network::types::NodeType;
use std::collections::{BTreeMap, BTreeSet};

const ARM_M: f32 = 160.0;
const TWO_LANE_M: f32 = 7.0;
const FOUR_LANE_M: f32 = 14.0;

type PointBits = [u64; 3];

fn bits(point: RoadVec3) -> PointBits {
    [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()]
}

// A node at `centre` with one arm per `(angle in degrees, width)`, each ending at a terminal.
// Even arms leave the centre from their edge's start, odd arms from its end.
fn star(centre: (f32, f32), arms: &[(f32, f32)], sloped: bool) -> RegionGraph {
    let mut graph = RegionGraph::new();
    let hub = graph.add_node(point(centre.0, centre.1, sloped), NodeType::Junction);
    for (index, &(angle_deg, width)) in arms.iter().enumerate() {
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let far = point(centre.0 + cos * ARM_M, centre.1 + sin * ARM_M, sloped);
        let far = graph.add_node(far, NodeType::Junction);
        if index % 2 == 0 {
            connect(&mut graph, hub, far, width, sloped);
        } else {
            connect(&mut graph, far, hub, width, sloped);
        }
    }
    graph.rebuild_intersection_clips();
    graph
}

fn equal(angles: &[f32]) -> Vec<(f32, f32)> {
    angles.iter().map(|&angle| (angle, TWO_LANE_M)).collect()
}

// The acceptance fixture set: T, four- to six-way, Y junctions, bends, mixed widths; every arm
// ends in a terminal.
fn fixtures(sloped: bool) -> Vec<(String, RegionGraph)> {
    let centre = (37.5, -12.25);
    let mut set = vec![
        (
            "t".to_owned(),
            star(centre, &equal(&[0.0, 180.0, 270.0]), sloped),
        ),
        (
            "four_way".to_owned(),
            star(centre, &equal(&[0.0, 90.0, 180.0, 270.0]), sloped),
        ),
        (
            "five_way".to_owned(),
            star(centre, &equal(&[0.0, 72.0, 144.0, 216.0, 288.0]), sloped),
        ),
        (
            "six_way".to_owned(),
            star(
                centre,
                &equal(&[0.0, 60.0, 120.0, 180.0, 240.0, 300.0]),
                sloped,
            ),
        ),
    ];
    for y_deg in [25.0, 40.0, 60.0] {
        let arms = equal(&[270.0, 90.0 - y_deg * 0.5, 90.0 + y_deg * 0.5]);
        set.push((format!("y_{y_deg}"), star(centre, &arms, sloped)));
    }
    for bend_deg in [10.0, 30.0, 50.0, 70.0, 90.0, 110.0, 130.0, 150.0, 160.0] {
        let arms = equal(&[15.0, 15.0 + bend_deg]);
        set.push((format!("bend_{bend_deg}"), star(centre, &arms, sloped)));
    }
    for side_deg in [40.0, 60.0, 90.0] {
        let arms = [
            (0.0, FOUR_LANE_M),
            (180.0, FOUR_LANE_M),
            (180.0 - side_deg, TWO_LANE_M),
        ];
        set.push((format!("mixed_{side_deg}"), star(centre, &arms, sloped)));
    }
    // A through road that widens at a side road: the straight sector's radial step (D4).
    let step = [(0.0, FOUR_LANE_M), (180.0, TWO_LANE_M), (270.0, TWO_LANE_M)];
    set.push(("width_step".to_owned(), star(centre, &step, sloped)));
    set
}

fn compiled(graph: &RegionGraph) -> RoadbedSystem {
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(graph);
    assert!(roadbed.is_current());
    roadbed
}

fn node_pieces<'a>(roadbed: &'a RoadbedSystem, graph: &RegionGraph) -> Vec<&'a NodePiece> {
    (0..graph.node_count() as u32)
        .filter_map(|node| roadbed.node_piece(node))
        .collect()
}

fn span_of(roadbed: &RoadbedSystem, key: ArmKey) -> Option<&SpanPiece> {
    roadbed.span_piece(key.edge)
}

fn span_mouth(span: &SpanPiece, end: EdgeEnd) -> &MouthSection {
    match end {
        EdgeEnd::Start => span.start_mouth(),
        EdgeEnd::End => span.end_mouth(),
    }
}

// Undirected edges of a mesh by vertex positions, with the triangles bordering each.
fn edge_uses(mesh: &PieceMesh) -> BTreeMap<(PointBits, PointBits), usize> {
    let mut uses = BTreeMap::new();
    for triangle in mesh.triangles() {
        let points = triangle.map(|vertex| bits(mesh.positions()[vertex as usize]));
        for i in 0..3 {
            let (a, b) = (points[i], points[(i + 1) % 3]);
            *uses.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    uses
}

fn chain_edges(mesh: &PieceMesh) -> BTreeSet<(PointBits, PointBits)> {
    mesh.outer_chains()
        .iter()
        .flat_map(|chain| {
            chain.vertices().windows(2).map(|pair| {
                let (a, b) = (
                    bits(mesh.positions()[pair[0] as usize]),
                    bits(mesh.positions()[pair[1] as usize]),
                );
                (a.min(b), a.max(b))
            })
        })
        .collect()
}

#[test]
fn every_fixture_builds_its_node_pieces_flat_and_sloped() {
    for sloped in [false, true] {
        for (name, graph) in fixtures(sloped) {
            let roadbed = compiled(&graph);
            assert!(
                roadbed.failed_nodes().is_empty(),
                "{name}: failed {:?}",
                roadbed
                    .failed_nodes()
                    .iter()
                    .map(|&node| (node, roadbed.node_error(node)))
                    .collect::<Vec<_>>()
            );
            let hub = roadbed.node_piece(0);
            let degree = graph.node_adjacency(0).len();
            match degree {
                2 => assert_eq!(hub.unwrap().kind(), NodeKind::Bend, "{name}"),
                _ => assert_eq!(hub.unwrap().kind(), NodeKind::Junction, "{name}"),
            }
            for far in 1..graph.node_count() as u32 {
                assert_eq!(
                    roadbed.node_piece(far).map(NodePiece::kind),
                    Some(NodeKind::Terminal),
                    "{name}: node {far}"
                );
            }
            for edge in 0..graph.edge_count() {
                assert!(roadbed.span_piece(edge).is_some(), "{name}: span {edge}");
            }
        }
    }
}

#[test]
fn node_pieces_are_watertight_and_meet_spans_on_shared_mouth_edges() {
    for sloped in [false, true] {
        for (name, graph) in fixtures(sloped) {
            let roadbed = compiled(&graph);
            for piece in node_pieces(&roadbed, &graph) {
                let mesh = piece.mesh();
                let uses = edge_uses(mesh);
                let chains = chain_edges(mesh);
                let mut mouth_points = BTreeSet::new();
                for (_, mouth) in piece.mouths() {
                    mouth_points.extend(mouth.vertices.iter().map(|&point| bits(point)));
                }
                for (&(a, b), &count) in &uses {
                    let at_mouth = piece.mouths().iter().any(|(_, mouth)| {
                        let row: BTreeSet<_> = mouth.vertices.iter().map(|&p| bits(p)).collect();
                        row.contains(&a) && row.contains(&b)
                    });
                    match count {
                        2 => assert!(!chains.contains(&(a, b)), "{name}: shared chain edge"),
                        1 => assert!(
                            at_mouth || chains.contains(&(a, b)),
                            "{name} node {}: open edge {a:?} {b:?}",
                            piece.node()
                        ),
                        _ => panic!("{name} node {}: edge borders {count}", piece.node()),
                    }
                }
                for chain in &chains {
                    assert_eq!(uses.get(chain), Some(&1), "{name}: chain edge borders one");
                }
                // Every open mouth edge is an open mouth edge of the span there, bitwise.
                for (key, mouth) in piece.mouths() {
                    let Some(span) = span_of(&roadbed, *key) else {
                        continue;
                    };
                    let shared = span_mouth(span, key.end);
                    assert_eq!(
                        shared.vertices, mouth.vertices,
                        "{name}: span copies the mouth"
                    );
                    let span_uses = edge_uses(span.mesh());
                    let row: BTreeSet<_> = mouth.vertices.iter().map(|&p| bits(p)).collect();
                    for (edge, &count) in &uses {
                        if count == 1 && row.contains(&edge.0) && row.contains(&edge.1) {
                            let point = |b: PointBits| b.map(f64::from_bits);
                            assert_eq!(
                                span_uses.get(edge),
                                Some(&1),
                                "{name}: mouth edge {:?} {:?} of {key:?}, row {:?}",
                                point(edge.0),
                                point(edge.1),
                                mouth.vertices
                            );
                        }
                    }
                }
                // Curb faces look at the lower band: a step off the face lands on ground level.
                for triangle in mesh.curb_face_triangles() {
                    let [a, b, c] = triangle.map(|vertex| mesh.positions()[vertex as usize]);
                    let normal = -(b - a).cross(c - a);
                    let centroid = (a + b + c) / 3.0;
                    let probe = RoadVec2::new(centroid.x, centroid.z)
                        + RoadVec2::new(normal.x, normal.z).normalize() * 0.05;
                    let ground = piece.surface_height_m(probe, Level::Ground);
                    let height = roadbed
                        .height_at(probe.x as f32, probe.y as f32)
                        .expect("road beside a curb face");
                    assert!(
                        (height - ground).abs() < 0.02,
                        "{name} node {}: curb face looks at {height} not ground {ground}",
                        piece.node()
                    );
                }
            }
        }
    }
}

#[test]
fn node_heights_are_the_plane_plus_level_bitwise() {
    for sloped in [false, true] {
        for (name, graph) in fixtures(sloped) {
            let roadbed = compiled(&graph);
            for piece in node_pieces(&roadbed, &graph) {
                for &point in piece.mesh().positions() {
                    let xz = RoadVec2::new(point.x, point.z);
                    let levels = [Level::Ground, Level::Raised]
                        .map(|level| f64::from(piece.surface_height_m(xz, level)).to_bits());
                    assert!(
                        levels.contains(&point.y.to_bits()),
                        "{name} node {}: {point:?} is off P + level",
                        piece.node()
                    );
                }
            }
        }
    }
}

#[test]
fn mouth_rows_are_the_arm_offset_curve_points_bitwise() {
    for (name, graph) in fixtures(true) {
        let roadbed = compiled(&graph);
        for piece in node_pieces(&roadbed, &graph) {
            let arms = NodeArms::collect(&graph, piece.node()).unwrap();
            let layout = arms.layout().unwrap();
            assert_eq!(&layout, piece.layout());
            for arm in &layout.arms {
                let input = arms.input(arm.input_index);
                let mouth = piece.mouth(arm.key).unwrap();
                let sign = lateral_sign(arm.key.end);
                for (slot, (lateral, level)) in row_slots(&mouth.profile).enumerate() {
                    let xz = input.offset_point(arm.mouth_station_m, sign * f64::from(lateral));
                    let vertex = mouth.vertices[slot];
                    assert_eq!((vertex.x, vertex.z), (xz.x, xz.y), "{name}: mouth XZ");
                    let y = f64::from(piece.surface_height_m(xz, level));
                    assert_eq!(vertex.y.to_bits(), y.to_bits(), "{name}: mouth height");
                }
            }
        }
    }
}

// Every node piece triangle as a sorted triple of vertex positions.
fn canonical_triangles(piece: &NodePiece) -> Vec<[PointBits; 3]> {
    let mesh = piece.mesh();
    let mut triangles: Vec<_> = mesh
        .triangles()
        .iter()
        .map(|triangle| {
            let mut points = triangle.map(|vertex| bits(mesh.positions()[vertex as usize]));
            points.sort_unstable();
            points
        })
        .collect();
    triangles.sort_unstable();
    triangles
}

#[test]
fn node_pieces_do_not_depend_on_edge_insertion_order_or_direction() {
    for sloped in [false, true] {
        let centre = (-210.0, 95.5);
        let arms = [
            (0.0, TWO_LANE_M),
            (90.0, FOUR_LANE_M),
            (180.0, TWO_LANE_M),
            (300.0, TWO_LANE_M),
        ];
        let forward = star(centre, &arms, sloped);
        let mut reordered = RegionGraph::new();
        let hub = reordered.add_node(point(centre.0, centre.1, sloped), NodeType::Junction);
        for edge in (0..forward.edge_count()).rev() {
            // The same points, with the opposite edge direction to `star`'s.
            let source = forward.edge(edge);
            let far_id = if source.start_node == 0 {
                source.end_node
            } else {
                source.start_node
            };
            let far = reordered.add_node(forward.node(far_id).pos, NodeType::Junction);
            let mut points = source.geometry.clone();
            points.reverse();
            let (start, end) = if source.start_node == 0 {
                (far, hub)
            } else {
                (hub, far)
            };
            reordered.add_edge(road_edge(start, end, points, source.width));
        }
        reordered.rebuild_intersection_clips();
        let a = compiled(&forward);
        let b = compiled(&reordered);
        assert_eq!(
            canonical_triangles(a.node_piece(0).unwrap()),
            canonical_triangles(b.node_piece(hub).unwrap()),
            "sloped {sloped}"
        );
    }
}

#[test]
fn neighbouring_junctions_on_a_short_edge_fit_or_report_corner_does_not_fit() {
    for link_m in [10.0, 13.0, 16.0, 20.0, 25.0, 30.0, 40.0] {
        let mut graph = RegionGraph::new();
        let west = graph.add_node(point(0.0, 0.0, true), NodeType::Junction);
        let east = graph.add_node(point(link_m, 0.0, true), NodeType::Junction);
        connect(&mut graph, west, east, TWO_LANE_M, true);
        for (node, x) in [(west, 0.0), (east, link_m)] {
            for (dx, dz) in [(0.0, ARM_M), (0.0, -ARM_M)] {
                let far = graph.add_node(point(x + dx, dz, true), NodeType::Junction);
                connect(&mut graph, node, far, TWO_LANE_M, true);
            }
        }
        let roadbed = compiled(&graph);
        for node in [west, east] {
            match roadbed.node_error(node) {
                None => {
                    let piece = roadbed.node_piece(node).unwrap();
                    let link = piece.layout().arms.iter().find(|arm| arm.key.edge == 0);
                    assert!(link.unwrap().mouth_station_m <= 0.5 * f64::from(link_m) + 1e-9);
                }
                Some(error) => assert!(
                    matches!(
                        error,
                        NodeError::Layout(LayoutError::CornerDoesNotFit { .. })
                    ),
                    "{link_m} m: {error:?}"
                ),
            }
        }
        // Each end keeps to its half, so the two node pieces never overlap.
        if let (Some(a), Some(b)) = (roadbed.node_piece(west), roadbed.node_piece(east)) {
            let mouth = |piece: &NodePiece| {
                piece
                    .layout()
                    .arms
                    .iter()
                    .find(|arm| arm.key.edge == 0)
                    .unwrap()
                    .mouth_station_m
            };
            assert!(mouth(a) + mouth(b) <= f64::from(link_m) + 1e-9);
        }
    }
}

#[test]
fn an_edit_rebuilds_only_its_closure() {
    // Two T junctions far apart; edge 1 belongs to the first.
    let mut graph = star((0.0, 0.0), &equal(&[0.0, 180.0, 270.0]), false);
    let offset = graph.node_count() as u32;
    let second = star((900.0, 0.0), &equal(&[0.0, 180.0, 270.0]), false);
    for node in 0..second.node_count() as u32 {
        graph.add_node(second.node(node).pos, NodeType::Junction);
    }
    for edge in 0..second.edge_count() {
        let mut copy = second.edge(edge).clone();
        copy.start_node += offset;
        copy.end_node += offset;
        graph.add_edge(copy);
    }
    graph.rebuild_intersection_clips();
    let mut roadbed = compiled(&graph);
    let world = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
    let before: BTreeMap<_, _> = roadbed.pieces_in_bounds(world).collect();
    roadbed.mark_edge_dirty(1);
    roadbed.compile_dirty(&graph);
    let after: BTreeMap<_, _> = roadbed.pieces_in_bounds(world).collect();
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    let edge = graph.edge(1);
    let nodes = [edge.start_node, edge.end_node];
    let closure_edges: BTreeSet<usize> = nodes
        .iter()
        .flat_map(|&node| graph.node_adjacency(node).iter().copied())
        .collect();
    for (id, generation) in &after {
        let rebuilt = match id {
            PieceId::Node(node) => nodes.contains(node),
            PieceId::Span(edge) => closure_edges.contains(edge),
        };
        assert_eq!(generation != &before[id], rebuilt, "{id:?}");
    }
}
