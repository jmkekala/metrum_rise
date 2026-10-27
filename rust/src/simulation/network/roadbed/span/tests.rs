// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68-B` span acceptance: watertight pieces, heights from the section evaluation, queries
//! from the rendered triangles, determinism and local rebuilds.

use super::model::test_support::{knot_stations, sample_with_mouth_frame};
use super::model::{SpanModel, SpanScratch};
use super::*;
use crate::simulation::network::roadbed::fixtures::{
    SLOPE, connect, mixed_profiles, point, road_edge, t_junction,
};
use crate::simulation::network::roadbed::geometry::triangle_height_at_xz;
use crate::simulation::network::roadbed::layout::EdgeEnd;
use crate::simulation::network::roadbed::mesh::PieceMesh;
use crate::simulation::network::roadbed::profile::Level;
use crate::simulation::network::roadbed::{BandKind, PieceId, RoadVec2, RoadbedSystem};
use crate::simulation::network::types::{EdgeClass, NodeType};
use godot::prelude::Vector3;
use std::collections::BTreeMap;

fn compiled(graph: &RegionGraph, chunk_span_m: f32) -> RoadbedSystem {
    let mut roadbed = RoadbedSystem::new(chunk_span_m);
    roadbed.compile_dirty(graph);
    assert!(roadbed.is_current());
    roadbed
}

fn spans<'a>(roadbed: &'a RoadbedSystem, graph: &RegionGraph) -> Vec<&'a SpanPiece> {
    (0..graph.edge_count())
        .filter_map(|edge| roadbed.span_piece(edge))
        .collect()
}

// Every published piece's mesh with its id: spans, then node pieces.
fn meshes<'a>(roadbed: &'a RoadbedSystem, graph: &RegionGraph) -> Vec<(PieceId, &'a PieceMesh)> {
    let spans = (0..graph.edge_count())
        .filter_map(|edge| Some((PieceId::Span(edge), roadbed.span_piece(edge)?.mesh())));
    let nodes = (0..graph.node_count() as u32)
        .filter_map(|node| Some((PieceId::Node(node), roadbed.node_piece(node)?.mesh())));
    spans.chain(nodes).collect()
}

// Expected section row: (lateral offset, height) per vertex, left to right.
fn expected_row(section: &RoadSection, profile: &RoadProfile) -> Vec<(f32, f32)> {
    let bands = &section.bands;
    let levels: Vec<Level> = profile.bands().iter().map(|band| band.level).collect();
    let mut row = vec![(bands[0].lateral_start_m, bands[0].height_start_m)];
    for j in 1..bands.len() {
        if levels[j - 1] != levels[j] {
            row.push((bands[j - 1].lateral_end_m, bands[j - 1].height_end_m));
        }
        row.push((bands[j].lateral_start_m, bands[j].height_start_m));
    }
    let last = &bands[bands.len() - 1];
    row.push((last.lateral_end_m, last.height_end_m));
    row
}

#[test]
fn spans_are_watertight_with_shared_curb_and_mouth_edges() {
    for sloped in [false, true] {
        let graph = mixed_profiles(sloped);
        let roadbed = compiled(&graph, 64.0);
        let pieces = spans(&roadbed, &graph);
        assert_eq!(pieces.len(), graph.edge_count(), "every edge has a span");
        for span in pieces {
            let mesh = span.mesh();
            let stride = span.start_mouth().vertices.len();
            let rows = span.sections().len();
            assert_eq!(mesh.positions().len(), rows * stride);
            let mut uses: BTreeMap<(u32, u32), usize> = BTreeMap::new();
            for triangle in mesh.triangles() {
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
            let row_of = |vertex: u32| vertex as usize / stride;
            for (&(a, b), &count) in &uses {
                let mouth = row_of(a) == row_of(b) && (row_of(a) == 0 || row_of(a) == rows - 1);
                let chain = chain_edges.contains(&(a, b));
                match count {
                    2 => assert!(
                        !chain,
                        "edge {}: chain edge ({a},{b}) shared",
                        span.edge_idx()
                    ),
                    1 => assert!(
                        mouth || chain,
                        "edge {}: open edge ({a},{b}) is neither mouth nor chain",
                        span.edge_idx()
                    ),
                    _ => panic!("edge {}: ({a},{b}) borders {count}", span.edge_idx()),
                }
            }
            for &(a, b) in &chain_edges {
                assert_eq!(
                    uses.get(&(a, b)),
                    Some(&1),
                    "chain edge ({a},{b}) borders one"
                );
            }
            // Curb faces pair a raised and a ground vertex at bit-identical XZ.
            for triangle in mesh.curb_face_triangles() {
                let points = triangle.map(|vertex| mesh.positions()[vertex as usize]);
                let distinct_xz = points
                    .iter()
                    .filter(|p| points.iter().filter(|q| q.x == p.x && q.z == p.z).count() == 1)
                    .count();
                assert_eq!(
                    distinct_xz, 1,
                    "a curb-face triangle spans one vertical edge"
                );
            }
        }
    }
}

#[test]
fn span_vertices_are_the_section_evaluation() {
    let graph = mixed_profiles(true);
    let roadbed = compiled(&graph, 64.0);
    for span in spans(&roadbed, &graph) {
        let mut scratch = SpanScratch::default();
        let SpanScratch {
            points, cumulative, ..
        } = &mut scratch;
        let mouths = [Some(span.start_mouth()), Some(span.end_mouth())];
        let model = SpanModel::new(&graph, span.edge_idx(), points, cumulative, mouths).unwrap();
        assert_eq!(span.sections()[0], span.start_mouth().section);
        assert_eq!(span.sections().last(), Some(&span.end_mouth().section));
        let stride = span.start_mouth().vertices.len();
        let rows = span.sections().len();
        for (row, section) in span.sections().iter().enumerate() {
            // Mouth rows are the node pieces' mouth sections (`node::tests`).
            if row == 0 || row + 1 == rows {
                continue;
            }
            assert_eq!(
                *section,
                model.section_at(section.s_m),
                "section {row} re-evaluates"
            );
            let expected = expected_row(section, &span.start_mouth().profile);
            assert_eq!(expected.len(), stride);
            for (slot, &(lateral_m, height_m)) in expected.iter().enumerate() {
                let vertex = span.mesh().positions()[row * stride + slot];
                let x = section.center_xz.x + section.lateral_xz.x * f64::from(lateral_m);
                let z = section.center_xz.y + section.lateral_xz.y * f64::from(lateral_m);
                assert_eq!(
                    (vertex.x.to_bits(), vertex.y.to_bits(), vertex.z.to_bits()),
                    (x.to_bits(), f64::from(height_m).to_bits(), z.to_bits()),
                    "edge {} row {row} slot {slot}",
                    span.edge_idx()
                );
            }
            if row > 0 {
                assert!(
                    section.s_m > span.sections()[row - 1].s_m,
                    "stations increase"
                );
            }
        }
        for (row, vertex) in span.start_mouth().vertices.iter().enumerate() {
            assert_eq!(*vertex, span.mesh().positions()[row], "start mouth copied");
        }
    }
}

#[test]
fn span_sections_run_between_the_node_mouths() {
    let graph = t_junction(false);
    let roadbed = compiled(&graph, 64.0);
    for edge_idx in 0..graph.edge_count() {
        let edge = graph.edge(edge_idx);
        let span = roadbed.span_piece(edge_idx).unwrap();
        let sections = span.sections();
        // Both mouths are the node pieces' own sections, shared by pointer.
        let node_mouth = |node: u32, end: EdgeEnd| {
            let key = crate::simulation::network::roadbed::ArmKey {
                edge: edge_idx,
                end,
            };
            Arc::clone(roadbed.node_piece(node).unwrap().mouth(key).unwrap())
        };
        assert!(Arc::ptr_eq(
            &node_mouth(edge.start_node, EdgeEnd::Start),
            &span.start_mouth
        ));
        assert!(Arc::ptr_eq(
            &node_mouth(edge.end_node, EdgeEnd::End),
            &span.end_mouth
        ));
        assert_eq!(sections[0], span.start_mouth().section);
        assert_eq!(sections.last(), Some(&span.end_mouth().section));
        // Standard roads keep the 8 m class step between knots.
        assert!(
            sections
                .windows(2)
                .all(|pair| pair[1].s_m - pair[0].s_m <= 8.0 + 1e-3),
            "edge {edge_idx} keeps the class step"
        );
    }
    // Junction arms clip at their layout mouths; terminals keep no clip, and their mouths sit
    // 1 m in from the end (`s_min`).
    assert!(graph.edge(0).end_clip > 0.0 && graph.edge(0).start_clip == 0.0);
    let west = roadbed.span_piece(0).unwrap();
    assert!((west.sections()[0].s_m - 1.0).abs() < 1e-6);
}

#[test]
fn sloped_span_rows_never_fall_millimetres_apart() {
    // Each interval between fixed stations is divided evenly at one step (`ROAD-68-D`): profile
    // cadence rows and class rows no longer start from different origins, which put the sloped
    // T's rows 3-6 mm apart before its knots.
    let graph = t_junction(true);
    let roadbed = compiled(&graph, 64.0);
    for edge_idx in 0..graph.edge_count() {
        let sections = roadbed.span_piece(edge_idx).unwrap().sections();
        for pair in sections.windows(2) {
            assert!(
                pair[1].s_m - pair[0].s_m >= 0.1,
                "edge {edge_idx}: rows at {} and {} m",
                pair[0].s_m,
                pair[1].s_m
            );
        }
    }
}

#[test]
fn query_heights_and_bands_equal_the_rendered_triangles() {
    let graph = mixed_profiles(true);
    let roadbed = compiled(&graph, 64.0);
    let all: Vec<[RoadVec3; 3]> = meshes(&roadbed, &graph)
        .into_iter()
        .flat_map(|(_, mesh)| (0..mesh.top_triangle_count()).map(move |i| mesh.triangle_points(i)))
        .collect();
    let mut checked = 0;
    for (id, mesh) in meshes(&roadbed, &graph) {
        for triangle in 0..mesh.top_triangle_count() {
            let points = mesh.triangle_points(triangle);
            let centroid = (points[0] + points[1] + points[2]) / 3.0;
            let (x, z) = (centroid.x as f32, centroid.z as f32);
            let point = RoadVec2::new(f64::from(x), f64::from(z));
            let expected = all
                .iter()
                .filter_map(|candidate| triangle_height_at_xz(candidate, point))
                .fold(None, |best: Option<f32>, h| {
                    Some(best.map_or(h, |b| b.max(h)))
                });
            assert_eq!(roadbed.height_at(x, z), expected, "{id:?}");
            let (surface_height, normal) = roadbed.height_and_normal_at(x, z).unwrap();
            assert_eq!(Some(surface_height), expected, "{id:?}");
            assert!(normal.is_finite() && normal.y > 0.0);
            assert!((normal.length() - 1.0).abs() < 1e-5);
            let kind = mesh.triangle_band(triangle).unwrap();
            if kind == BandKind::Carriageway {
                assert_eq!(roadbed.carriageway_height_at(x, z), expected);
            }
            checked += 1;
        }
    }
    assert!(checked > 1000, "checked {checked} triangles");

    // The centreline of the east arm is carriageway; past the curb it is sidewalk.
    let east = roadbed.span_piece(1).unwrap();
    let section = &east.sections()[east.sections().len() / 2];
    let at = |lateral_m: f64| {
        let p = section.center_xz + section.lateral_xz * lateral_m;
        (p.x as f32, p.y as f32)
    };
    let (cx, cz) = at(1.0);
    assert_eq!(roadbed.band_at(cx, cz), Some(BandKind::Carriageway));
    let (sx, sz) = at(4.5);
    assert_eq!(roadbed.band_at(sx, sz), Some(BandKind::Sidewalk));
    assert_eq!(roadbed.carriageway_height_at(sx, sz), None);
    assert!(roadbed.height_at(sx, sz).unwrap() > roadbed.height_at(cx, cz).unwrap());
    assert_eq!(roadbed.height_at(cx, cz + 300.0), None);

    // A ray straight down and a slanted ray hit the triangle the height query answers from.
    let height = roadbed.height_at(cx, cz).unwrap();
    let down = roadbed
        .raycast(Vector3::new(cx, 50.0, cz), Vector3::new(0.0, -1.0, 0.0))
        .unwrap();
    assert!((down.y - height).abs() < 1e-4 && down.x == cx && down.z == cz);
    let origin = Vector3::new(cx - 40.0, height + 40.0, cz);
    let slanted = roadbed
        .raycast(origin, Vector3::new(cx, height, cz) - origin)
        .unwrap();
    assert!(
        (slanted - Vector3::new(cx, height, cz)).length() < 0.05,
        "slanted hit {slanted:?}"
    );
    assert!(
        roadbed
            .raycast(
                Vector3::new(cx, 50.0, cz + 300.0),
                Vector3::new(0.0, -1.0, 0.0)
            )
            .is_none()
    );

    // Owner-local lane lookups answer from the owner's span only.
    let lane = roadbed.lane_query(1, usize::MAX, true);
    assert_eq!(
        lane.height_at(cx, cz),
        roadbed.carriageway_height_at(cx, cz)
    );
    assert_eq!(lane.height_at(sx, sz), None);
    assert_eq!(
        roadbed.lane_query(0, usize::MAX, true).height_at(cx, cz),
        None
    );

    // Overlap queries see the same triangles.
    let near = |radius| {
        roadbed.any_band_near_disc(
            RoadVec2::new(f64::from(sx), f64::from(sz)),
            radius,
            |kind| kind == BandKind::Carriageway,
        )
    };
    assert!(near(1.5) && !near(0.5));
    let mut kinds = Vec::new();
    roadbed.any_top_triangle_in_bounds(
        [
            f64::from(cx) - 0.1,
            f64::from(cz) - 0.1,
            f64::from(cx) + 0.1,
            f64::from(cz) + 0.1,
        ],
        |_, kind| {
            kinds.push(kind);
            false
        },
    );
    assert!(kinds.contains(&BandKind::Carriageway));
    let world = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
    let ids: Vec<_> = roadbed.pieces_in_bounds(world).map(|(id, _)| id).collect();
    let mut expected: Vec<_> = meshes(&roadbed, &graph)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    expected.sort_unstable();
    assert_eq!(ids, expected);
}

// Every published bit of every span, for exact comparison.
fn fingerprint(roadbed: &RoadbedSystem, graph: &RegionGraph) -> Vec<u64> {
    let mut bits = Vec::new();
    for span in spans(roadbed, graph) {
        bits.push(span.edge_idx() as u64);
        for section in span.sections() {
            bits.push(u64::from(section.s_m.to_bits()));
            bits.push(u64::from(section.center_height_m.to_bits()));
            for band in &section.bands {
                bits.push(u64::from(band.height_start_m.to_bits()));
                bits.push(u64::from(band.height_end_m.to_bits()));
            }
        }
        let mesh = span.mesh();
        for p in mesh.positions() {
            bits.extend([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]);
        }
        for t in mesh.triangles() {
            bits.extend(t.map(u64::from));
        }
    }
    bits.extend(
        roadbed
            .occupied_chunks()
            .flat_map(|(x, z)| [x as u64, z as u64]),
    );
    bits
}

#[test]
fn pieces_are_bitwise_identical_across_runs_and_worker_counts() {
    let graph = mixed_profiles(true);
    let reference = fingerprint(&compiled(&graph, 64.0), &graph);
    for threads in [1, 4, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        for _ in 0..2 {
            let roadbed = pool.install(|| compiled(&graph, 64.0));
            assert_eq!(
                fingerprint(&roadbed, &graph),
                reference,
                "{threads} workers"
            );
        }
    }
    // Incremental rebuilds converge on the cold compile.
    let mut roadbed = compiled(&graph, 64.0);
    for edge in 0..graph.edge_count() {
        roadbed.mark_edge_dirty(edge);
    }
    roadbed.compile_dirty(&graph);
    assert_eq!(fingerprint(&roadbed, &graph), reference);
}

#[test]
fn an_edit_rebuilds_only_its_closure_and_chunks_accumulate() {
    let mut graph = mixed_profiles(false);
    let mut roadbed = compiled(&graph, 64.0);
    let world = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
    let before: BTreeMap<_, _> = roadbed.pieces_in_bounds(world).collect();
    let first_changed: Vec<_> = roadbed.changed_chunks().collect();
    assert_eq!(first_changed, roadbed.occupied_chunks().collect::<Vec<_>>());
    roadbed.clear_changed_chunks();

    // Dirty edge 4 only: its span and its two terminal node pieces rebuild, nothing else.
    roadbed.mark_edge_dirty(4);
    let changed = roadbed.compile_dirty(&graph);
    let after: BTreeMap<_, _> = roadbed.pieces_in_bounds(world).collect();
    let ends = [graph.edge(4).start_node, graph.edge(4).end_node];
    for (id, generation) in &after {
        let rebuilt = *id == PieceId::Span(4) || matches!(id, PieceId::Node(n) if ends.contains(n));
        assert_eq!(generation != &before[id], rebuilt, "{id:?}");
    }
    let mut closure_chunks: Vec<_> = roadbed
        .span_piece(4)
        .unwrap()
        .footprint()
        .render_chunks()
        .to_vec();
    // Edge 4 is the footpath: its ends have no node pieces until `ROAD-68-F`.
    for node in ends {
        if let Some(piece) = roadbed.node_piece(node) {
            closure_chunks.extend(piece.footprint().render_chunks());
        }
    }
    closure_chunks.sort_unstable();
    closure_chunks.dedup();
    assert_eq!(changed, closure_chunks);

    // A dirty node rebuilds its incident edges; chunks accumulate until taken.
    roadbed.mark_node_dirty(0);
    let second = roadbed.compile_dirty(&graph);
    let after_node: BTreeMap<_, _> = roadbed.pieces_in_bounds(world).collect();
    for (id, generation) in &after_node {
        let rebuilt = matches!(id, PieceId::Span(0..=2) | PieceId::Node(0));
        assert_eq!(generation != &after[id], rebuilt, "{id:?}");
    }
    let mut pending: Vec<_> = changed.iter().chain(&second).copied().collect();
    pending.sort_unstable();
    pending.dedup();
    assert_eq!(roadbed.changed_chunks().collect::<Vec<_>>(), pending);

    // Deleting an edge removes its piece and frees chunks no other piece covers.
    graph.edge_mut(4).deleted = true;
    roadbed.mark_edge_dirty(4);
    roadbed.compile_dirty(&graph);
    assert!(roadbed.span_piece(4).is_none() && roadbed.sections(4).is_none());
    let occupied: Vec<_> = roadbed.occupied_chunks().collect();
    let mut covered: Vec<_> = spans(&roadbed, &graph)
        .iter()
        .flat_map(|span| span.footprint().render_chunks().to_vec())
        .collect();
    for node in 0..graph.node_count() as u32 {
        if let Some(piece) = roadbed.node_piece(node) {
            covered.extend(piece.footprint().render_chunks());
        }
    }
    covered.sort_unstable();
    covered.dedup();
    assert_eq!(occupied, covered);
}

#[test]
fn shallow_pass_through_spans_share_one_cross_section_axis() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-40.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let east = graph.add_node(Vector3::new(40.0, 0.0, 4.0), NodeType::Junction);
    let mut bridge = road_edge(
        west,
        center,
        vec![Vector3::new(-40.0, 0.0, 0.0), Vector3::ZERO],
        7.0,
    );
    bridge.class = EdgeClass::Bridge;
    let bridge = graph.add_edge(bridge);
    let approach = graph.add_edge(road_edge(
        center,
        east,
        vec![Vector3::ZERO, Vector3::new(40.0, 0.0, 4.0)],
        7.0,
    ));
    let roadbed = compiled(&graph, 64.0);
    let bridge_mouth = roadbed.span_piece(bridge).unwrap().end_mouth();
    let approach_mouth = roadbed.span_piece(approach).unwrap().start_mouth();
    let lateral_dot = bridge_mouth
        .section
        .lateral_xz
        .dot(approach_mouth.section.lateral_xz)
        .abs();
    assert!(
        (1.0 - lateral_dot) <= 1.0e-9,
        "pass-through mouths share one axis; abs_dot={lateral_dot:.12}"
    );
    assert_eq!(bridge_mouth.vertices, approach_mouth.vertices);
}

#[test]
fn inserted_samples_keep_short_profile_interval_bands_unfolded() {
    // Logged junction profile: a 13 cm vertical interval has only 1.6 cm of plan run.
    let points = [
        RoadVec3::new(2654.783, 96.00182, -8441.406),
        RoadVec3::new(2649.0454, 96.112976, -8443.134),
        RoadVec3::new(2649.03, 95.98347, -8443.139),
        RoadVec3::new(2647.115, 96.19704, -8443.715),
    ];
    let cumulative = knot_stations(&points);
    let section = |fraction: f32| {
        let s = cumulative[1] + (cumulative[2] - cumulative[1]) * fraction;
        let (center, tangent) = sample_with_mouth_frame(&points, s);
        let lateral = RoadVec3::new(-tangent.y, 0.0, tangent.x) * 3.5;
        [center - lateral, center, center + lateral]
    };
    let winding = |a: RoadVec3, b: RoadVec3, c: RoadVec3| {
        (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)
    };
    let mut previous = section(0.0);
    for i in 1..=16 {
        let next = section(i as f32 / 16.0);
        for band in 0..2 {
            let (sl, el, er, sr) = (
                previous[band],
                next[band],
                next[band + 1],
                previous[band + 1],
            );
            assert!(
                winding(sl, el, er) > 0.0 && winding(sl, er, sr) > 0.0,
                "inserted sample {i} folds band {band}"
            );
        }
        previous = next;
    }
}

#[test]
fn a_single_edge_runs_between_its_terminal_mouths() {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(point(0.0, 0.0, true), NodeType::Junction);
    let b = graph.add_node(point(100.0, 0.0, true), NodeType::Junction);
    let edge = connect(&mut graph, a, b, 7.0, true);
    let roadbed = compiled(&graph, 64.0);
    let span = roadbed.span_piece(edge).unwrap();
    let sections = span.sections();
    // Terminal mouths sit 1 m (`s_min`) in from both ends; the round caps cover the rest.
    let slope_factor = (1.0 + f64::from(SLOPE).powi(2)).sqrt() as f32;
    assert!((sections[0].s_m - slope_factor).abs() < 1e-4);
    let length = graph.edge(edge).physical_length;
    assert!((sections.last().unwrap().s_m - (length - slope_factor)).abs() < 1e-3);
    // 1 (mouth), 8, 16, 20 (knot), ... 96, 99 (mouth): the class step restarts at every knot.
    assert_eq!(sections.len(), 5 * 3 + 1);
    assert_eq!(
        span.mesh().curb_face_triangles().len(),
        (sections.len() - 1) * 2 * 2
    );
    graph.edge_mut(edge).deleted = true;
    assert!(roadbed.build_span_piece(&graph, edge).is_none());
}
