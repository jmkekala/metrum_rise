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
use crate::simulation::network::roadbed::span::model::NOSE_CLEARANCE_M;
use crate::simulation::network::roadbed::{BandKind, RoadSection, RoadbedSystem};
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

// An undivided 2+2 road as wide as a 2+2 road with `median`.
fn as_wide_as(median: Option<RoadMedian>) -> impl Fn(Edge) -> Edge {
    move |edge: Edge| {
        let mut edge = divided(edge, 2, 2, None);
        edge.width = Edge::cross_section_width_m(2, 2, median);
        edge
    }
}

// A 2+2 road with `median` from its free west end into an undivided road of its width: noses at
// both ends of edge 0, the east one past the crossing strip of the pass-through node.
fn noses_into_undivided(median: Option<RoadMedian>, sloped: bool) -> RegionGraph {
    straight_pair(
        move |edge: Edge| divided(edge, 2, 2, median),
        as_wide_as(median),
        false,
        sloped,
    )
}

// The fixtures with a raised median: a continuation through a pass-through node, a nose pair at
// a pass-through node into an undivided road of the same width (also with the widest grass and
// a narrow paving median), a T whose through road is divided (noses at a junction), a bend, a
// divided road onto a bridge and into a tunnel, and a lone divided bridge and tunnel (free
// ends).
fn divided_fixtures(sloped: bool) -> Vec<(&'static str, RegionGraph)> {
    let wide = as_wide_as(grass(4.0));
    let paving = RoadMedian::from_tool(3, 2.0).unwrap();
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
        ("12 m grass noses", noses_into_undivided(grass(12.0), sloped)),
        ("2 m paving noses", noses_into_undivided(paving, sloped)),
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
        let bridge = compiled(&divided_fixtures(sloped)[8].1);
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
    // width the nose's radius (2 m) further on, the rows between at the angles of 7 equal arcs.
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
        assert_eq!(sections[full].s_m, sections[tip].s_m + 2.0);
        assert_eq!(full - tip, 7, "a row per arc");
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

// The median's outline at `section`, a body row: the outer edges of its two kerbs, laterally.
fn median_outline(section: &RoadSection) -> Option<(f32, f32)> {
    let median = section.bands.iter().position(|band| band.kind == BandKind::Median)?;
    Some((
        section.bands[median - 1].lateral_start_m,
        section.bands[median + 1].lateral_end_m,
    ))
}

// Checks each nose of `span`, whose raised median is `width_m` wide, and returns how many it
// has (`ROAD-80`, R3d): the tip row closes the outline onto the line between the lane groups
// `NOSE_CLEARANCE_M` past the mouth or past the crossing strip the span holds there; every
// outline vertex up to the first full-width row lies on the semicircle of radius `r = width_m /
// 2` centred `r` past the tip on that line (in station and lateral metres, within 0.1 mm), each
// of the `segments` arcs at most 0.5 m long; the first full-width row stands `r` past the tip.
fn assert_round_noses(name: &str, span: &SpanPiece, width_m: f32, segments: usize) -> usize {
    let sections = span.sections();
    let radius_m = 0.5 * width_m;
    let Some(first) = sections.iter().position(|section| median_outline(section).is_some())
    else {
        return 0;
    };
    let last = sections
        .iter()
        .rposition(|section| median_outline(section).is_some())
        .unwrap();
    let ends = [
        (first, 1.0, EdgeEnd::Start, sections[0].s_m),
        (last, -1.0, EdgeEnd::End, sections[sections.len() - 1].s_m),
    ];
    let mut noses = 0;
    for (tip, sign, end, mouth_m) in ends {
        let (low, high) = median_outline(&sections[tip]).unwrap();
        if (high - low - width_m).abs() < 1.0e-5 {
            // The median continues through this end: its row is a full-width body row.
            continue;
        }
        noses += 1;
        assert_eq!(low, high, "{name}: the tip row closes the outline");
        let from_m = span.crossing_strip_m(end).map_or(mouth_m, |(near, far)| {
            if sign > 0.0 { far } else { near }
        });
        let tip_m = sections[tip].s_m;
        assert!(
            ((tip_m - from_m) * sign - NOSE_CLEARANCE_M).abs() < 1.0e-4,
            "{name}: the tip {tip_m} lies the clearance past {from_m}"
        );
        let centre = (radius_m, low);
        let mut previous = [(0.0, low); 2];
        let mut row = tip;
        for arc in 1.. {
            row = if sign > 0.0 { row + 1 } else { row - 1 };
            let section = &sections[row];
            let d_m = (section.s_m - tip_m) * sign;
            let (low, high) = median_outline(section).expect("body rows run from tip to tip");
            for (side, lateral_m) in [low, high].into_iter().enumerate() {
                let point = (d_m, lateral_m);
                let radial = (point.0 - centre.0).hypot(point.1 - centre.1);
                assert!(
                    (radial - radius_m).abs() < 1.0e-4,
                    "{name}: outline {point:?} lies {radial} from the centre, not {radius_m}"
                );
                let chord = (point.0 - previous[side].0).hypot(point.1 - previous[side].1);
                assert!(chord <= 0.5 + 1.0e-4, "{name}: chord {chord} m");
                previous[side] = point;
            }
            if (high - low - width_m).abs() < 1.0e-5 {
                assert_eq!(arc, segments, "{name}: a row per arc");
                break;
            }
            assert!(high - low < width_m - 1.0e-5, "{name}: inside the nose");
        }
        assert!(
            ((sections[row].s_m - tip_m) * sign - radius_m).abs() < 1.0e-4,
            "{name}: the full width the radius past the tip"
        );
    }
    noses
}

#[test]
fn raised_median_noses_are_semicircles() {
    // (fixture, median width, arcs per quarter circle, noses on the divided edges): 2+2 roads
    // nosing at their free end and at a pass-through node into an undivided road of their width
    // (whose crossing strip the span holds), a T, a bend and the structures.
    let paving = RoadMedian::from_tool(3, 2.0).unwrap();
    for sloped in [false, true] {
        let mut cases = vec![
            ("3 m grass", noses_into_undivided(grass(3.0), sloped), 3.0, 5, 2),
            ("12 m grass", noses_into_undivided(grass(12.0), sloped), 12.0, 19, 2),
            ("2 m paving", noses_into_undivided(paving, sloped), 2.0, 4, 2),
        ];
        cases.extend(divided_fixtures(sloped).into_iter().filter_map(|(name, graph)| {
            let median = graph.edge(0).median?;
            let noses = match name {
                "continuous" => 2,
                "divided t" => 4,
                "bend" => 4,
                _ => 2,
            };
            let segments = match median.width_m {
                2.0 => 4,
                3.0 => 5,
                4.0 => 7,
                _ => 19,
            };
            Some((name, graph, median.width_m, segments, noses))
        }));
        for (name, graph, width_m, segments, noses) in cases {
            let roadbed = compiled(&graph);
            let found: usize = (0..graph.edge_count())
                .filter(|&edge| graph.edge(edge).median.is_some())
                .map(|edge| {
                    let span = roadbed.span_piece(edge).unwrap();
                    assert_round_noses(name, span, width_m, segments)
                })
                .sum();
            assert_eq!(found, noses, "{name} (sloped {sloped}): noses");
        }
    }
}

// Two noses that do not fit leave the span undivided (`ROAD-80`): a lone road between two road
// ends is divided exactly when its tips lie at least the median's width (twice the radius)
// apart, for a narrow and the widest median.
#[test]
fn a_span_too_short_for_its_two_noses_stays_undivided() {
    let mut outcomes = [false; 2];
    for (width_m, lengths) in [(3.0, [6.0, 8.0, 10.0]), (12.0, [16.0, 20.0, 26.0])] {
        for length_m in lengths {
            let mut graph = RegionGraph::new();
            let a = graph.add_node(point(0.0, 0.0, true), NodeType::Junction);
            let b = graph.add_node(point(length_m, 0.0, true), NodeType::Junction);
            let points = polyline((0.0, 0.0), (length_m, 0.0), length_m, true);
            graph.add_edge(divided(road_edge(a, b, points, 7.0), 2, 2, grass(width_m)));
            graph.rebuild_intersection_clips();
            let roadbed = compiled(&graph);
            let span = roadbed.span_piece(0).unwrap();
            let sections = span.sections();
            let tips_m = (
                sections[0].s_m + NOSE_CLEARANCE_M,
                sections[sections.len() - 1].s_m - NOSE_CLEARANCE_M,
            );
            let fits = tips_m.1 - tips_m.0 >= 2.0 * (0.5 * width_m);
            let name = format!("{width_m} m on {length_m} m");
            assert_eq!(sections.iter().any(|s| median_outline(s).is_some()), fits, "{name}");
            assert_eq!(span.mesh().band_triangles(BandKind::Median).is_empty(), !fits, "{name}");
            assert_watertight(&name, span);
            if fits {
                let segments = if width_m == 3.0 { 5 } else { 19 };
                assert_eq!(assert_round_noses(&name, span, width_m, segments), 2, "{name}");
            }
            outcomes[usize::from(fits)] = true;
        }
    }
    assert_eq!(outcomes, [true, true], "both outcomes occur");
}

// Spans without a raised median are as before the round nose (`ROAD-80`, R3d): an undivided
// pair and a painted one hash as recorded on a40e6f6b, flat and sloped. Re-recorded by `ROAD-94`
// (default junction lane use): the lanes of all six change, nothing else; the 2 + 2 straight
// pair's node now continues each lane on the lane of its own rank instead of on every lane.
#[test]
fn spans_without_a_raised_median_keep_their_products() {
    use crate::simulation::network::roadbed::fixtures::{ProductProbe, assert_probes};
    let painted = |edge: Edge| divided(edge, 2, 2, RoadMedian::from_tool(1, 1.0).unwrap());
    let undivided = |edge: Edge| divided(edge, 2, 2, None);
    let mut actual = Vec::new();
    for sloped in [false, true] {
        for (name, graph) in [
            ("undivided", straight_pair(undivided, undivided, true, sloped)),
            ("painted", straight_pair(painted, painted, false, sloped)),
            ("painted noses", straight_pair(painted, undivided, false, sloped)),
        ] {
            actual.push((format!("{name}/{sloped}"), ProductProbe::of_graph(&graph)));
        }
    }
    assert_probes(&actual, WITHOUT_RAISED_MEDIAN);
}

// Recorded on a40e6f6b, before the round nose.
const WITHOUT_RAISED_MEDIAN: &[(&str, [u64; 7])] = &[
    (
        "undivided/false",
        [
            0x61b3d4f0d4ff91db,
            0x048f321e01a44fc7,
            0x1df747977cee582f,
            0xb20f8d217aacc3ed,
            0x4f34b93cafbcd86f,
            0x39f6662732d2b2aa,
            0x5a21f467a126f0fb,
        ],
    ),
    (
        "painted/false",
        [
            0x2db64f429ef54d7b,
            0x465f0d00904e4c89,
            0x269195d876d35aae,
            0x34fe874c8b286229,
            0x3cacf181a32e379d,
            0x57f4bed4c7a2b44a,
            0xd6bcc8bb8e89b9e0,
        ],
    ),
    (
        "painted noses/false",
        [
            0xf6c43650403a61a8,
            0xc3f0d0cfc97ead3d,
            0xbc694f0548b165c1,
            0xdeadb3c3e00cc2e2,
            0xd2ed3c1674a9d386,
            0x361de04d9e399dcd,
            0x98429a1e56766213,
        ],
    ),
    (
        "undivided/true",
        [
            0xea51d6fc9e784d97,
            0xc2675bf5257a4b40,
            0x1df747977cee582f,
            0x216c5eaa3658c1c0,
            0x3fece4cee385943e,
            0xf36c284401562c04,
            0x9c9a4599ede68974,
        ],
    ),
    (
        "painted/true",
        [
            0xc1eb93265ac3ec77,
            0xbf8353d7781f14e1,
            0x269195d876d35aae,
            0x0bb7f0e00b63bf37,
            0x5d4d723eb4aeb8bd,
            0x107acfce25675428,
            0xbe038eddc7637f62,
        ],
    ),
    (
        "painted noses/true",
        [
            0xf52386d0f265c4e4,
            0xba39b6a427ec2330,
            0xbc694f0548b165c1,
            0xbc2422ecfdb352c4,
            0xb10cb0f4379cb9da,
            0x2c5d937149b68db7,
            0xedd4619dcf5d5ab2,
        ],
    ),
];

// `ROAD-95` round R5b1 (D3): a left turn pocket on a divided 2+2 road with a 5 m grass median
// (`fixtures::four_way_2x2_left_pocket`: edge 0 the base road into the transition node 1, 60 m
// west of the junction, edge 1 the pocket edge with a 1.5 m median). The transition passes
// through: the median continues without a nose, and on the pocket span its right edge (the
// pocket side) runs linearly from the base road's 2.5 m to its own -1.0 m over 12 m, with rows
// at the mouth, every 2 m and the taper's end; the left edge stays the base road's bit for bit.
// Both spans are watertight.
#[test]
fn a_left_pockets_median_narrows_linearly_over_its_first_twelve_metres() {
    use crate::simulation::network::roadbed::fixtures::four_way_2x2_left_pocket;
    let graph = four_way_2x2_left_pocket();
    assert!(
        median_continues(&graph, 1),
        "no nose either side of the transition"
    );
    assert!(pass_through_crossings(&graph, 1).is_some_and(|arms| arms.iter().all(|(_, c)| !c)));
    let roadbed = compiled(&graph);
    assert!(roadbed.node_piece(1).is_none(), "a pass-through");
    let (base, pocket) = (
        roadbed.span_piece(0).unwrap(),
        roadbed.span_piece(1).unwrap(),
    );
    // The shared row: the base span's end row is the pocket span's start row.
    assert_eq!(&*base.end_mouth().vertices, &*pocket.start_mouth().vertices);
    assert_eq!(
        pocket.start_mouth().profile.bands().len(),
        9,
        "a body row through the node"
    );
    let sections = pocket.sections();
    let mouth_m = sections[0].s_m;
    let base_outline = median_outline(base.sections().last().unwrap()).unwrap();
    assert_eq!(base_outline, (-2.5, 2.5));
    let mut rows = Vec::new();
    for section in sections {
        let d_m = section.s_m - mouth_m;
        let Some((left, right)) = median_outline(section) else {
            continue;
        };
        if d_m <= 12.0 + 1e-4 {
            rows.push(d_m);
            assert_eq!(
                left.to_bits(),
                (-2.5f32).to_bits(),
                "{d_m}: the far edge stays"
            );
            let expected = 2.5 + (-1.0 - 2.5) * (d_m / 12.0);
            assert!(
                (right - expected).abs() < 1e-5,
                "{d_m}: {right} on the line {expected}"
            );
        } else if d_m < 20.0 {
            assert_eq!(
                (left, right),
                (-2.5, -1.0),
                "{d_m}: the pocket edge's own median"
            );
        }
    }
    assert_eq!(rows.len(), 7, "rows at 0, 2, .., 12 m: {rows:?}");
    for (row, d_m) in rows.iter().enumerate() {
        assert!((d_m - 2.0 * row as f32).abs() < 1e-3, "{rows:?}");
    }
    for edge in 0..graph.edge_count() {
        assert_watertight(
            "four_way_2x2_left_pocket",
            roadbed.span_piece(edge).unwrap(),
        );
    }
    // The pocket edge still noses out at the junction: the last body row is a tip.
    let (low, high) = sections.iter().rev().find_map(median_outline).unwrap();
    assert_eq!(low, high, "the tip row closes the outline");
}
