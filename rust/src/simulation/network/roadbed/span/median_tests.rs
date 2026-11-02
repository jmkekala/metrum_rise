// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-80` divided spans: where a raised median continues through a pass-through node, where
//! it noses out, and the span meshes it builds.

use crate::simulation::network::graph::{Edge, RegionGraph, RoadMedian};
use crate::simulation::network::roadbed::arms::{
    edges_pass_through, median_continues, pass_through_crossings,
};
use crate::simulation::network::roadbed::fixtures::{divided, grass, straight_pair};
use crate::simulation::network::roadbed::layout::EdgeEnd;
use crate::simulation::network::roadbed::span::incident_direction;

// Whether the pair's two edges pass through at the centre node 0 by the one pass-through rule.
fn pass_through(graph: &RegionGraph) -> bool {
    let arm = |edge: usize| {
        let edge = graph.edge(edge);
        let end = if edge.start_node == 0 {
            EdgeEnd::Start
        } else {
            EdgeEnd::End
        };
        (edge, incident_direction(edge, end).unwrap())
    };
    edges_pass_through(arm(0), arm(1))
}

// Whether node 0 crosses on either arm.
fn crosses(graph: &RegionGraph) -> bool {
    pass_through_crossings(graph, 0).is_some_and(|arms| arms.iter().any(|(_, crosses)| *crosses))
}

#[test]
fn a_raised_median_continues_only_where_both_arms_carry_it_in_line() {
    let paving = RoadMedian::from_tool(3, 2.0).unwrap();
    let painted = RoadMedian::from_tool(1, 2.0).unwrap();
    let with = |fwd, bkw, median: Option<RoadMedian>| {
        move |edge: Edge| divided(edge, fwd, bkw, median)
    };
    // (west, east, east reversed, continues). Lane groups line up looking along the road: the
    // reversed east edge must carry the mirrored counts.
    let cases = [
        ("grass both ways", with(2, 2, grass(4.0)), with(2, 2, grass(4.0)), false, true),
        ("reversed", with(2, 2, grass(4.0)), with(2, 2, grass(4.0)), true, true),
        ("uneven, in line", with(2, 1, grass(4.0)), with(2, 1, grass(4.0)), false, true),
        ("uneven, mirrored", with(2, 1, grass(4.0)), with(1, 2, grass(4.0)), true, true),
        ("uneven, offset", with(2, 1, grass(4.0)), with(2, 1, grass(4.0)), true, false),
        ("grass and paving", with(2, 2, grass(2.0)), with(2, 2, paving), false, false),
        ("painted", with(2, 2, painted), with(2, 2, painted), false, false),
    ];
    for (name, west, east, reversed, continues) in cases {
        let graph = straight_pair(west, east, reversed, false);
        assert!(pass_through(&graph), "{name}: one mouth profile passes through");
        assert_eq!(median_continues(&graph, 0), continues, "{name}");
        // Z3: no default crossing where the median continues; elsewhere the D8 default.
        assert_eq!(crosses(&graph), !continues, "{name}: crossing");
    }

    // A divided road and an undivided one of its width pass through (one mouth profile); the
    // median noses out at the node. Of another width they are laid out.
    let wide = |edge: Edge| {
        let mut edge = divided(edge, 2, 2, None);
        edge.width = Edge::cross_section_width_m(2, 2, grass(4.0));
        edge
    };
    let graph = straight_pair(with(2, 2, grass(4.0)), wide, false, false);
    assert!(pass_through(&graph) && !median_continues(&graph, 0) && crosses(&graph));
    let graph = straight_pair(with(2, 2, grass(4.0)), with(2, 2, None), false, false);
    assert!(!pass_through(&graph));

    // An explicit crossing opens a continuing median; turning it off again closes it.
    let mut graph = straight_pair(with(2, 2, grass(4.0)), with(2, 2, grass(4.0)), false, false);
    graph.set_crosswalk_override(0, 1, true);
    assert!(crosses(&graph) && !median_continues(&graph, 0));
    graph.set_crosswalk_override(0, 1, false);
    assert!(!crosses(&graph) && median_continues(&graph, 0));
}

use crate::simulation::network::roadbed::fixtures::{bend, point, polyline, road_edge, t_junction};
use crate::simulation::network::roadbed::mesh::{PieceMesh, StripKind};
use crate::simulation::network::roadbed::span::SpanPiece;
use crate::simulation::network::roadbed::span::model::{NOSE_CLEARANCE_M, NOSE_LENGTH_M};
use crate::simulation::network::roadbed::{BandKind, RoadbedSystem};
use crate::simulation::network::types::{EdgeClass, NodeType};
use std::collections::BTreeMap;

fn compiled(graph: &RegionGraph) -> RoadbedSystem {
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(graph);
    assert!(roadbed.is_current() && roadbed.failed_nodes().is_empty());
    roadbed
}

fn grass_road(edge: Edge) -> Edge {
    divided(edge, 2, 2, grass(4.0))
}

// The fixtures with a raised median: a continuation through a pass-through node, a nose pair at
// a pass-through node into an undivided road of the same width, a T whose through road is
// divided (noses at a junction), a bend, a divided road onto a bridge and into a tunnel, and a
// lone divided bridge and tunnel (free ends).
fn divided_fixtures(sloped: bool) -> Vec<(&'static str, RegionGraph)> {
    let wide = |edge: Edge| {
        let mut edge = divided(edge, 2, 2, None);
        edge.width = Edge::cross_section_width_m(2, 2, grass(4.0));
        edge
    };
    let class = |class: EdgeClass| {
        move |edge: Edge| Edge {
            class,
            ..grass_road(edge)
        }
    };
    let mut t = t_junction(sloped);
    for edge in [0, 1] {
        let divided = grass_road(t.edge(edge).clone());
        *t.edge_mut(edge) = divided;
    }
    t.rebuild_intersection_clips();
    let mut bent = bend(80.0, 80.0, sloped);
    for edge in [0, 1] {
        let divided = divided(bent.edge(edge).clone(), 1, 1, grass(3.0));
        *bent.edge_mut(edge) = divided;
    }
    bent.rebuild_intersection_clips();
    let lone = |class: EdgeClass| {
        let mut graph = RegionGraph::new();
        let a = graph.add_node(point(-80.0, 0.0, sloped), NodeType::Junction);
        let b = graph.add_node(point(80.0, 0.0, sloped), NodeType::Junction);
        let points = polyline((-80.0, 0.0), (80.0, 0.0), 20.0, sloped);
        graph.add_edge(Edge {
            class,
            ..grass_road(road_edge(a, b, points, 7.0))
        });
        graph.rebuild_intersection_clips();
        graph
    };
    vec![
        ("continuous", straight_pair(grass_road, grass_road, true, sloped)),
        ("noses at a pass-through", straight_pair(grass_road, wide, false, sloped)),
        ("divided t", t),
        ("bend", bent),
        ("onto a bridge", straight_pair(grass_road, class(EdgeClass::Bridge), false, sloped)),
        ("into a tunnel", straight_pair(grass_road, class(EdgeClass::Tunnel), false, sloped)),
        ("lone bridge", lone(EdgeClass::Bridge)),
        ("lone tunnel", lone(EdgeClass::Tunnel)),
    ]
}

// The first vertex of each section row: outer chain 0 runs along them.
fn row_starts(mesh: &PieceMesh) -> &[u32] {
    mesh.outer_chains()[0].vertices()
}

// Every top and curb-face edge borders two triangles, apart from the outer chains' and the two
// mouth rows' (one each); no triangle repeats a vertex or lies flat in XZ, but curb faces, which
// each span one vertical edge.
fn assert_watertight(name: &str, span: &SpanPiece) {
    let mesh = span.mesh();
    let starts = row_starts(mesh);
    let rows = starts.len();
    let row_of = |vertex: u32| starts.partition_point(|&first| first <= vertex) - 1;
    let mut uses: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for triangle in mesh.triangles() {
        let [a, b, c] = *triangle;
        assert!(a != b && b != c && a != c, "{name}: {triangle:?} repeats a vertex");
        for i in 0..3 {
            let (a, b) = (triangle[i], triangle[(i + 1) % 3]);
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let chain_edges: Vec<(u32, u32)> = mesh
        .outer_chains()
        .iter()
        .flat_map(|chain| {
            chain
                .vertices()
                .windows(2)
                .map(|pair| (pair[0].min(pair[1]), pair[0].max(pair[1])))
        })
        .collect();
    for (&(a, b), &count) in &uses {
        let mouth = row_of(a) == row_of(b) && (row_of(a) == 0 || row_of(a) == rows - 1);
        let chain = chain_edges.contains(&(a, b));
        match count {
            2 => assert!(!chain, "{name}: chain edge ({a},{b}) shared"),
            1 => assert!(mouth || chain, "{name}: open edge ({a},{b}), rows {}", row_of(a)),
            _ => panic!("{name}: ({a},{b}) borders {count}"),
        }
    }
    for triangle in 0..mesh.top_triangle_count() {
        let [p, q, r] = mesh.triangle_points(triangle);
        let area = (q.x - p.x) * (r.z - p.z) - (q.z - p.z) * (r.x - p.x);
        assert!(area.abs() > 1e-9, "{name}: flat top triangle {triangle}");
    }
    for triangle in mesh.curb_face_triangles() {
        let points = triangle.map(|vertex| mesh.positions()[vertex as usize]);
        let distinct_xz = points
            .iter()
            .filter(|p| points.iter().filter(|q| q.x == p.x && q.z == p.z).count() == 1)
            .count();
        assert_eq!(distinct_xz, 1, "{name}: a curb face spans one vertical edge");
    }
    // The two end rows are the mouth sections' vertices, bit for bit.
    let row = |row: usize| {
        let end = starts.get(row + 1).map_or(mesh.positions().len(), |&next| next as usize);
        &mesh.positions()[starts[row] as usize..end]
    };
    assert_eq!(row(0), &*span.start_mouth().vertices, "{name}: start mouth row");
    assert_eq!(row(rows - 1), &*span.end_mouth().vertices, "{name}: end mouth row");
}

#[test]
fn divided_spans_are_watertight_and_close_their_structures() {
    for sloped in [false, true] {
        for (name, graph) in divided_fixtures(sloped) {
            let roadbed = compiled(&graph);
            let mut divided = 0;
            for edge in 0..graph.edge_count() {
                let Some(span) = roadbed.span_piece(edge) else {
                    continue;
                };
                assert_watertight(name, span);
                let mesh = span.mesh();
                if graph.edge(edge).median.is_some() {
                    divided += 1;
                    assert!(
                        !mesh.band_triangles(BandKind::Median).is_empty(),
                        "{name}: edge {edge} has a median"
                    );
                }
                match graph.edge(edge).class {
                    EdgeClass::Bridge => {
                        assert_eq!(mesh.strip_kind(), Some(StripKind::Fascia));
                        let mut strips = 0;
                        mesh.for_each_strip_triangle(|_, _| strips += 1);
                        assert!(strips >= 4 * (row_starts(mesh).len() - 1), "{name}: fascias");
                    }
                    EdgeClass::Tunnel => assert!(mesh.buried() && !mesh.lining_triangles().is_empty()),
                    EdgeClass::Standard => {}
                }
            }
            assert!(divided > 0, "{name}");
            // Node pieces stay watertight with their arms' mouth rows (the mouths are shared).
            for node in 0..graph.node_count() as u32 {
                if let Some(piece) = roadbed.node_piece(node) {
                    for (key, mouth) in piece.mouths() {
                        let span = roadbed.span_piece(key.edge).unwrap();
                        let shared = match key.end {
                            EdgeEnd::Start => span.start_mouth(),
                            EdgeEnd::End => span.end_mouth(),
                        };
                        assert_eq!(&**mouth, shared, "{name}: node {node} mouth");
                    }
                }
            }
        }
        // A lone bridge's free ends are capped; a lone tunnel's lining closes them.
        let bridge = compiled(&divided_fixtures(sloped)[6].1);
        assert_eq!(bridge.span_piece(0).unwrap().mesh().cap_kind(), Some(StripKind::Fascia));
    }
}

#[test]
fn a_continuing_median_shares_its_row_and_noses_take_their_stations() {
    let graph = straight_pair(grass_road, grass_road, true, true);
    let roadbed = compiled(&graph);
    let (west, east) = (roadbed.span_piece(0).unwrap(), roadbed.span_piece(1).unwrap());
    // Edge 1 runs from the east end into the node, so its end row is edge 0's end row mirrored.
    let mut mirrored = east.end_mouth().vertices.to_vec();
    mirrored.reverse();
    assert_eq!(&*west.end_mouth().vertices, &mirrored[..]);
    assert_eq!(west.end_mouth().profile.bands().len(), 9, "a body row through the node");

    // Each free end noses out: the tip row at the clearance past its mouth, the median at its
    // width the nose length further on, the rows between at 1 m.
    let median_width = |section: &crate::simulation::network::roadbed::RoadSection| {
        section
            .bands
            .iter()
            .find(|band| band.kind == BandKind::Median)
            .map(|band| band.lateral_end_m - band.lateral_start_m)
    };
    for span in [west, east] {
        let sections = span.sections();
        let mouth_m = sections[0].s_m;
        let tip = sections
            .iter()
            .position(|section| section.bands.len() == 9)
            .unwrap();
        assert_eq!(sections[tip].s_m, mouth_m + NOSE_CLEARANCE_M);
        assert_eq!(median_width(&sections[tip]), Some(0.0));
        assert!(sections[..tip].iter().all(|section| section.bands.len() == 6));
        let full = sections
            .iter()
            .position(|section| median_width(section) == Some(4.0 - 0.3))
            .unwrap();
        assert_eq!(sections[full].s_m, sections[tip].s_m + NOSE_LENGTH_M);
        assert_eq!(full - tip, NOSE_LENGTH_M as usize, "a row every metre");
        let at = |row: usize, lateral_m: f64| {
            let section = &sections[row];
            let p = section.center_xz + section.lateral_xz * lateral_m;
            (p.x as f32, p.y as f32)
        };
        let middle = (full + sections.len() - 1) / 2;
        let (x, z) = at(middle, 0.0);
        assert_eq!(roadbed.band_at(x, z), Some(BandKind::Median));
        let (x, z) = at(middle, 5.0);
        assert_eq!(roadbed.band_at(x, z), Some(BandKind::Carriageway));
        let (x, z) = at(tip - 1, 0.0);
        assert_eq!(roadbed.band_at(x, z), Some(BandKind::Carriageway));
    }
}
