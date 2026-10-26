// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68-D` patch loops: chain orientation, closed boundaries, holes, rectangle clipping and
//! open ends.

use super::*;
use crate::simulation::network::roadbed::RoadVec2;
use crate::simulation::network::roadbed::fixtures::{
    bend, connect, footpath_fixtures, point, road_edge, t_junction,
};
use crate::simulation::network::types::NodeType;
use crate::simulation::terrain::rect_loops::{inside, signed_area_xz};

fn compiled(graph: &RegionGraph) -> RoadbedSystem {
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(graph);
    assert!(roadbed.failed_nodes().is_empty(), "fixture nodes compile");
    roadbed
}

fn everything(roadbed: &RoadbedSystem) -> [f64; 4] {
    let mut rect = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (id, _) in roadbed.pieces_in_bounds([-1.0e6, -1.0e6, 1.0e6, 1.0e6]) {
        let bounds = roadbed.pieces[&id].mesh().bounds_xz();
        rect = [
            rect[0].min(bounds[0]),
            rect[1].min(bounds[1]),
            rect[2].max(bounds[2]),
            rect[3].max(bounds[3]),
        ];
    }
    [rect[0] - 5.0, rect[1] - 5.0, rect[2] + 5.0, rect[3] + 5.0]
}

// A straight road through a pass-through node, a lone road between two road ends, and a
// square ring of four bends around a closed block.
fn straight_and_ring() -> Vec<(String, RegionGraph)> {
    let mut straight = RegionGraph::new();
    let a = straight.add_node(point(-80.0, 3.0, true), NodeType::Junction);
    let b = straight.add_node(point(0.0, 0.0, true), NodeType::Junction);
    let c = straight.add_node(point(90.0, -2.0, true), NodeType::Junction);
    connect(&mut straight, a, b, 7.0, true);
    connect(&mut straight, b, c, 7.0, true);
    straight.rebuild_intersection_clips();
    let mut ring = RegionGraph::new();
    let corners = [(-60.0, -60.0), (60.0, -60.0), (60.0, 60.0), (-60.0, 60.0)]
        .map(|(x, z)| ring.add_node(point(x, z, false), NodeType::Junction));
    for index in 0..4 {
        connect(
            &mut ring,
            corners[index],
            corners[(index + 1) % 4],
            7.0,
            false,
        );
    }
    ring.rebuild_intersection_clips();
    vec![("straight".to_owned(), straight), ("ring".to_owned(), ring)]
}

fn fixtures() -> Vec<(String, RegionGraph)> {
    let mut set = vec![
        ("t".to_owned(), t_junction(false)),
        ("t_sloped".to_owned(), t_junction(true)),
        ("bend".to_owned(), bend(60.0, 70.0, true)),
    ];
    set.extend(footpath_fixtures(true));
    set.extend(straight_and_ring());
    set
}

#[test]
fn every_chain_keeps_the_road_on_its_left_and_closes_its_boundary() {
    for (name, graph) in fixtures() {
        let roadbed = compiled(&graph);
        let loops = terrain_loops(&roadbed, &graph, everything(&roadbed))
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let mut segments = 0;
        for terrain_loop in &loops {
            assert!(
                terrain_loop.edges.iter().all(Option::is_some),
                "{name}: a loop inside the rectangle is chains only"
            );
            for (index, edge) in terrain_loop.edges.iter().enumerate() {
                let a = terrain_loop.points[index];
                let b = terrain_loop.points[(index + 1) % terrain_loop.points.len()];
                let direction = RoadVec2::new(b.x - a.x, b.z - a.z);
                if direction.length() < 0.05 {
                    continue;
                }
                // Left is the counter-clockwise quarter turn from +x towards +z.
                let left = direction.normalize().perp() * 0.02;
                let middle = RoadVec2::new(a.x + b.x, a.z + b.z) * 0.5;
                let probe = |side: RoadVec2| {
                    let at = middle + side;
                    roadbed.band_at(at.x as f32, at.y as f32).is_some()
                };
                assert!(probe(left), "{name}: road left of {edge:?}");
                assert!(!probe(-left), "{name}: no road right of {edge:?}");
                segments += 1;
            }
        }
        assert!(segments > 0, "{name}: chains were checked");
        let outer = loops.iter().filter(|terrain_loop| !terrain_loop.is_hole());
        assert_eq!(outer.count(), 1, "{name}: one connected footprint");
        let holes = loops.iter().filter(|terrain_loop| terrain_loop.is_hole());
        assert_eq!(holes.count(), usize::from(name == "ring"), "{name}: holes");
        // Every chain segment of every grounded piece appears exactly once.
        let mut seen: Vec<_> = loops
            .iter()
            .flat_map(|l| l.edges.iter().flatten())
            .collect();
        let count = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), count, "{name}: segments used once");
        let expected: usize = roadbed
            .pieces_in_bounds(everything(&roadbed))
            .map(|(id, _)| {
                let mesh = roadbed.pieces[&id].mesh();
                mesh.outer_chains()
                    .iter()
                    .map(|chain| chain.vertices().len() - 1)
                    .sum::<usize>()
            })
            .sum();
        assert_eq!(count, expected, "{name}: every segment joined");
    }
}

#[test]
fn clipped_rectangles_reuse_chain_vertices_and_tile_the_footprint() {
    for (name, graph) in fixtures() {
        let roadbed = compiled(&graph);
        let whole = everything(&roadbed);
        let area = |rect: [f64; 4]| -> f64 {
            terrain_loops(&roadbed, &graph, rect)
                .unwrap_or_else(|error| panic!("{name}: {error:?}"))
                .iter()
                .map(|terrain_loop| signed_area_xz(&terrain_loop.points))
                .sum()
        };
        let total = area(whole);
        // Cut through node and span pieces on both axes, on and off vertices.
        for (cut_x, cut_z) in [(0.0, 0.0), (1.37, -2.5), (-30.0, 13.25), (37.5, -12.25)] {
            let quarters = [
                [whole[0], whole[1], cut_x, cut_z],
                [cut_x, whole[1], whole[2], cut_z],
                [whole[0], cut_z, cut_x, whole[3]],
                [cut_x, cut_z, whole[2], whole[3]],
            ];
            let sum: f64 = quarters.into_iter().map(area).sum();
            assert!(
                (sum - total).abs() <= 1.0e-6 * total.abs().max(1.0),
                "{name}: quarters at ({cut_x}, {cut_z}) sum to {sum}, whole {total}"
            );
            for rect in quarters {
                for terrain_loop in terrain_loops(&roadbed, &graph, rect).unwrap() {
                    assert!(
                        !terrain_loop.is_hole() || name == "ring",
                        "{name}: no false holes"
                    );
                    for (index, edge) in terrain_loop.edges.iter().enumerate() {
                        let a = terrain_loop.points[index];
                        assert!(within(rect, a), "{name}: vertex inside the rectangle");
                        if let Some(edge) = edge
                            && inside(rect, a)
                        {
                            let mesh = roadbed.pieces[&edge.piece].mesh();
                            let chain = mesh.outer_chains()[edge.chain as usize].vertices();
                            let start = mesh.positions()[chain[edge.segment as usize] as usize];
                            assert_eq!(start, a, "{name}: interior seam vertex is the chain's");
                        }
                    }
                }
            }
        }
    }
}

// `TERRAIN-07-C` (T22 3 (a)): a road continued straight on by a bridge closes its chains across
// the shared mouth row with its span's wall run, whose segments bound the loop as structure
// sources.
#[test]
fn a_chain_ending_at_a_structure_span_closes_across_its_mouth() {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(point(-80.0, 0.0, false), NodeType::Junction);
    let b = graph.add_node(point(0.0, 0.0, false), NodeType::Junction);
    let c = graph.add_node(point(80.0, 0.0, false), NodeType::Junction);
    connect(&mut graph, a, b, 7.0, false);
    let mut bridge = road_edge(
        b,
        c,
        vec![point(0.0, 0.0, false), point(80.0, 0.0, false)],
        7.0,
    );
    bridge.class = EdgeClass::Bridge;
    graph.add_edge(bridge);
    graph.rebuild_intersection_clips();
    let roadbed = compiled(&graph);
    let runs = roadbed
        .span_piece(0)
        .expect("the road's span")
        .mesh()
        .wall_runs();
    assert_eq!(runs.len(), 1, "one wall run across the shared mouth row");
    let loops = terrain_loops(&roadbed, &graph, [-20.0, -20.0, 20.0, 20.0]).unwrap();
    let walls: Vec<ChainSegment> = loops
        .iter()
        .flat_map(|terrain_loop| terrain_loop.edges.iter().flatten().copied())
        .filter(|segment| segment.wall)
        .collect();
    assert_eq!(walls.len(), runs[0].vertices().len() - 1);
    assert!(
        walls
            .iter()
            .all(|segment| segment.piece == PieceId::Span(0))
    );
    // Away from the bridge the road's own loops close.
    assert!(terrain_loops(&roadbed, &graph, [-100.0, -20.0, -20.0, 20.0]).is_ok());
}
