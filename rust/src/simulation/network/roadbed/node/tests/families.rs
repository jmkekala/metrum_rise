// SPDX-License-Identifier: GPL-2.0-only

//! The remaining road families at nodes (`ROAD-68-F` part 2), flat and on the 2.5% slope: roads
//! without sidewalks (raised shoulders, D6) at junctions, bends and road ends; mixed widths and
//! profiles across straight sectors (D4 tapers on straight and curved arms, band-only changes);
//! bridge decks and tunnel road pieces meeting grounded roads at a node piece or a span mouth.
//! Every node piece is watertight, on its plane and free of slivers, and meets its spans on
//! shared mouth edges; spans are watertight and pass-through pairs share one mouth row; only
//! car roads with sidewalks cross (D8); structure ends have no node piece.

use super::*;
use crate::simulation::network::roadbed::ArmLayout;
use crate::simulation::network::roadbed::fixtures::finalize;
use crate::simulation::network::types::{EdgeClass, TransitFlags};

// What one fixture arm is: a car road with sidewalks, without them (shoulders), a bridge or a
// tunnel (with sidewalks).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Arm {
    Sidewalks,
    Rural,
    Bridge,
    Tunnel,
}

// One fixture arm: its angle in degrees, its carriageway width and its kind.
type ArmSpec = (f32, f32, Arm);

// Bridges climb and tunnels descend by this much over their arm, from the hub out.
const STRUCTURE_RISE_M: f32 = 6.0;

// `star` with each arm made its kind; arm `i` is edge `i`, its far node `i + 1`. Then the clips
// and the profile solve, as an edit runs them.
fn family(arms: &[ArmSpec], sloped: bool) -> RegionGraph {
    let widths: Vec<_> = arms
        .iter()
        .map(|&(angle, width, _)| (angle, width))
        .collect();
    let mut graph = star((37.5, -12.25), &widths, sloped);
    for (edge, &(_, _, arm)) in arms.iter().enumerate() {
        make(&mut graph, edge, arm);
    }
    finalize(&mut graph);
    graph
}

fn make(graph: &mut RegionGraph, edge: usize, arm: Arm) {
    let rise_m = match arm {
        Arm::Sidewalks => return,
        Arm::Rural => {
            graph.edge_mut(edge).allowed_types = TransitFlags::CAR;
            return;
        }
        Arm::Bridge => STRUCTURE_RISE_M,
        Arm::Tunnel => -STRUCTURE_RISE_M,
    };
    let hub = graph.node(0).pos;
    let far = graph.edge(edge).start_node.max(graph.edge(edge).end_node);
    let raise = |point: Vector3| {
        let run = Vector3::new(point.x - hub.x, 0.0, point.z - hub.z).length();
        point + Vector3::UP * (rise_m * run / ARM_M)
    };
    graph.nodes[far as usize].pos = raise(graph.node(far).pos);
    let edge = graph.edge_mut(edge);
    edge.class = if arm == Arm::Bridge {
        EdgeClass::Bridge
    } else {
        EdgeClass::Tunnel
    };
    edge.geometry
        .iter_mut()
        .for_each(|point| *point = raise(*point));
    edge.physical_geometry = edge.geometry.clone();
}

/// The family fixtures by name; node 0 is the hub.
pub(super) fn family_fixtures(sloped: bool) -> Vec<(String, RegionGraph)> {
    use Arm::{Bridge, Rural, Sidewalks as Walk, Tunnel};
    let (two, four) = (TWO_LANE_M, FOUR_LANE_M);
    let mut set: Vec<(&str, Vec<ArmSpec>)> = vec![
        // Roads without sidewalks, and where they meet roads with sidewalks.
        (
            "rural_t",
            vec![(0.0, two, Rural), (180.0, two, Rural), (270.0, two, Rural)],
        ),
        (
            "rural_four_way",
            [0.0, 90.0, 180.0, 270.0]
                .map(|angle| (angle, two, Rural))
                .to_vec(),
        ),
        (
            "rural_y_40",
            vec![(270.0, two, Rural), (70.0, two, Rural), (110.0, two, Rural)],
        ),
        (
            "rural_bend_90",
            vec![(15.0, two, Rural), (105.0, two, Rural)],
        ),
        (
            "rural_bend_150",
            vec![(15.0, two, Rural), (165.0, two, Rural)],
        ),
        (
            "rural_side_road",
            vec![(0.0, two, Walk), (180.0, two, Walk), (270.0, two, Rural)],
        ),
        (
            "rural_through_road",
            vec![(0.0, two, Rural), (180.0, two, Rural), (270.0, two, Walk)],
        ),
        (
            "rural_wide_t",
            vec![(0.0, four, Rural), (180.0, four, Rural), (270.0, two, Walk)],
        ),
        (
            "rural_bend_mixed",
            vec![(15.0, two, Rural), (115.0, two, Walk)],
        ),
        // Mixed profiles across a straight sector: band-only and band-and-width tapers (D4).
        (
            "band_step",
            vec![(0.0, two, Rural), (180.0, two, Walk), (270.0, two, Walk)],
        ),
        (
            "band_width_step",
            vec![(0.0, four, Rural), (180.0, two, Walk), (270.0, two, Walk)],
        ),
        (
            "band_four_way",
            vec![
                (0.0, two, Rural),
                (90.0, two, Walk),
                (180.0, two, Walk),
                (270.0, two, Rural),
            ],
        ),
        // Straight degree-two width and band changes: no pass-through (their spans would share
        // no mouth row), so a layout with D4 tapers on both straight sectors.
        (
            "band_pass_through",
            vec![(0.0, two, Rural), (180.0, two, Walk)],
        ),
        (
            "width_pass_through",
            vec![(0.0, four, Walk), (180.0, two, Walk)],
        ),
        (
            "width_bridge_pass_through",
            vec![(0.0, two, Walk), (180.0, four, Bridge)],
        ),
        // Bridge decks and tunnel road pieces at a junction, a bend and a pass-through.
        (
            "bridge_t",
            vec![(0.0, two, Walk), (180.0, two, Walk), (270.0, two, Bridge)],
        ),
        ("bridge_bend", vec![(15.0, two, Walk), (135.0, two, Bridge)]),
        (
            "bridge_pass_through",
            vec![(0.0, two, Walk), (180.0, two, Bridge)],
        ),
        (
            "bridge_rural_t",
            vec![(0.0, two, Rural), (180.0, two, Rural), (270.0, two, Bridge)],
        ),
        (
            "tunnel_t",
            vec![(0.0, two, Walk), (180.0, two, Walk), (270.0, two, Tunnel)],
        ),
        ("tunnel_bend", vec![(15.0, two, Walk), (135.0, two, Tunnel)]),
        (
            "tunnel_pass_through",
            vec![(0.0, two, Walk), (180.0, two, Tunnel)],
        ),
    ];
    // The two straight-sector steps with their changing arm curving away (D4 on curved arms).
    set.push((
        "band_step_curved",
        vec![(0.0, two, Rural), (180.0, two, Walk), (270.0, two, Walk)],
    ));
    set.push((
        "band_width_step_curved",
        vec![(0.0, four, Rural), (180.0, two, Walk), (270.0, two, Walk)],
    ));
    set.into_iter()
        .map(|(name, arms)| {
            let mut graph = family(&arms, sloped);
            if name.ends_with("_curved") {
                curve_arm(&mut graph, 0, 120.0, sloped);
                finalize(&mut graph);
            }
            (format!("{name}/{sloped}"), graph)
        })
        .collect()
}

fn arm_kind(graph: &RegionGraph, edge: usize) -> Arm {
    let edge = graph.edge(edge);
    match edge.class {
        EdgeClass::Bridge => Arm::Bridge,
        EdgeClass::Tunnel => Arm::Tunnel,
        EdgeClass::Standard if edge.allowed_types & TransitFlags::FOOT == 0 => Arm::Rural,
        EdgeClass::Standard => Arm::Sidewalks,
    }
}

// Every edge of a span borders two triangles except its outer-chain edges and its two mouth
// rows' edges.
fn check_span_watertight(name: &str, span: &SpanPiece) {
    let mesh = span.mesh();
    let stride = span.start_mouth().vertices.len();
    let rows = span.sections().len();
    assert_eq!(mesh.positions().len(), rows * stride, "{name}");
    let uses = edge_uses(mesh);
    let chains = chain_edges(mesh);
    let mouths: BTreeSet<PointBits> = [span.start_mouth(), span.end_mouth()]
        .iter()
        .flat_map(|mouth| mouth.vertices.iter().map(|&point| bits(point)))
        .collect();
    for (&(a, b), &count) in &uses {
        match count {
            2 => assert!(!chains.contains(&(a, b)), "{name}: shared span chain edge"),
            1 => assert!(
                chains.contains(&(a, b)) || (mouths.contains(&a) && mouths.contains(&b)),
                "{name} span {}: open edge {a:?} {b:?}",
                span.edge_idx()
            ),
            _ => panic!("{name} span {}: edge borders {count}", span.edge_idx()),
        }
    }
}

#[test]
fn road_families_build_watertight_pieces_flat_and_sloped() {
    for sloped in [false, true] {
        for (name, graph) in family_fixtures(sloped) {
            let roadbed = compiled(&graph);
            let failed: Vec<_> = roadbed
                .failed_nodes()
                .iter()
                .map(|&node| (node, roadbed.node_error(node)))
                .collect();
            assert!(failed.is_empty(), "{name}: failed {failed:?}");
            let kinds: Vec<_> = (0..graph.edge_count())
                .map(|edge| arm_kind(&graph, edge))
                .collect();
            for (edge, &kind) in kinds.iter().enumerate() {
                let span = roadbed.span_piece(edge);
                let span = span.unwrap_or_else(|| panic!("{name}: span {edge}"));
                check_span_watertight(&name, span);
                // A structure's far end owns no node piece (its abutment or portal is
                // `TERRAIN-07`'s); every other road end has its round cap (D2).
                let far = edge as u32 + 1;
                let structure = matches!(kind, Arm::Bridge | Arm::Tunnel);
                assert_eq!(
                    roadbed.node_piece(far).map(NodePiece::kind),
                    (!structure).then_some(NodeKind::Terminal),
                    "{name}: node {far}"
                );
            }
            for piece in node_pieces(&roadbed, &graph) {
                check_watertight(&name, &roadbed, piece);
                check_strips(&name, piece);
                check_on_plane(&name, piece);
            }
            match roadbed.node_piece(0) {
                Some(hub) => {
                    // Only car roads with sidewalks cross (D8): every such arm of a junction,
                    // one at a degree-two node.
                    let crossing = |arm: &ArmLayout| arm.crosswalk.is_some();
                    for arm in &hub.layout().arms {
                        if kinds[arm.key.edge] == Arm::Rural {
                            assert!(!crossing(arm), "{name}: a rural arm crosses");
                            assert_eq!(hub.crosswalk_kerbs(arm.key), None, "{name}");
                        }
                    }
                    let crossings = hub.layout().arms.iter().filter(|arm| crossing(arm)).count();
                    let walkable = kinds.iter().filter(|&&kind| kind != Arm::Rural).count();
                    let expected = match kinds.len() {
                        2 => walkable.min(1),
                        _ => walkable,
                    };
                    assert_eq!(crossings, expected, "{name}: crossings");
                }
                None => {
                    // A pass-through pair shares one bisected mouth row.
                    let [a, b] = [0, 1].map(|edge| {
                        let span = roadbed.span_piece(edge).unwrap();
                        let end = if graph.edge(edge).start_node == 0 {
                            EdgeEnd::Start
                        } else {
                            EdgeEnd::End
                        };
                        span_mouth(span, end).vertices.clone()
                    });
                    assert_eq!(a, b, "{name}: pass-through spans share their mouth row");
                }
            }
        }
    }
}

// The one triangle walk of every overlap query (`PieceMesh::any_top_triangle_in_bounds`, which
// zoning's cell test, the field test and the roadbed's own queries take) visits every top
// triangle whose XZ bounds meet the query rectangle, on every family piece: 4 m squares tiled
// over each piece, so zoning cells and fields are tested against every triangle they can overlap.
#[test]
fn overlap_walk_visits_every_triangle_a_rectangle_meets() {
    for sloped in [false, true] {
        for (name, graph) in family_fixtures(sloped) {
            let roadbed = compiled(&graph);
            let world = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
            for (id, _) in roadbed.pieces_in_bounds(world) {
                let mesh = match id {
                    PieceId::Span(edge) => roadbed.span_piece(edge).unwrap().mesh(),
                    PieceId::Node(node) => roadbed.node_piece(node).unwrap().mesh(),
                };
                let [min_x, min_z, max_x, max_z] = mesh.bounds_xz();
                let (columns, rows) = (
                    ((max_x - min_x) / 4.0).ceil(),
                    ((max_z - min_z) / 4.0).ceil(),
                );
                for (column, row) in (0..columns as usize)
                    .flat_map(|column| (0..rows as usize).map(move |row| (column, row)))
                {
                    let (x, z) = (min_x + 4.0 * column as f64, min_z + 4.0 * row as f64);
                    let rect = [x, z, x + 4.0, z + 4.0];
                    let mut visited = BTreeSet::new();
                    mesh.any_top_triangle_in_bounds(rect, |triangle| {
                        visited.insert(triangle);
                        false
                    });
                    for triangle in 0..mesh.top_triangle_count() {
                        let points = mesh.triangle_points(triangle);
                        let meets = points.iter().any(|p| p.x >= rect[0])
                            && points.iter().any(|p| p.x <= rect[2])
                            && points.iter().any(|p| p.z >= rect[1])
                            && points.iter().any(|p| p.z <= rect[3]);
                        assert!(
                            !meets || visited.contains(&triangle),
                            "{name} {id:?}: triangle {triangle} meets {rect:?} but is not visited"
                        );
                    }
                }
            }
        }
    }
}
