// SPDX-License-Identifier: GPL-2.0-only

//! The remaining road families on the reference harness (`ROAD-68-F` part 2), flat and on its
//! 2.5% slope: bridge decks and tunnel road pieces where they meet grounded roads at a junction's
//! node piece or a pass-through's span mouth, and roads without sidewalks (shoulders) at a T. The
//! published road chunks, skirts and bridge concrete included, are watertight as `terrain_seams`
//! measures the reference junctions: a bridge's fascias (`TERRAIN-07-B`) close its deck's sides
//! against the deck edge and the slab. The terrain side of structures (abutments and portals) is
//! `TERRAIN-07`'s, so the terrain is not checked here.

use super::road_edit_order::{ARM_M, Stroke, committed};
use super::terrain_seams::open_road_edges_where;
use super::*;
use crate::simulation::network::types::{EdgeClass, TransitFlags};
use godot::prelude::Vector2;
use std::collections::HashSet;

const THROUGH: Stroke = ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1);
const SIDE: Stroke = ([Vector2::ZERO, Vector2::new(0.0, ARM_M)], 1);
const ON: Stroke = (
    [Vector2::new(ARM_M, 0.0), Vector2::new(2.0 * ARM_M, 0.0)],
    1,
);

// The live edge whose far end, away from the origin, lies at `end`.
fn edge_ending_at(core: &SimCore, end: Vector2) -> usize {
    let graph = &core.region_graph;
    (0..graph.edge_count())
        .find(|&edge| {
            let edge = graph.edge(edge);
            !edge.deleted
                && [edge.start_node, edge.end_node].iter().any(|&node| {
                    let pos = graph.node(graph.get_valid_node(node)).pos;
                    Vector2::new(pos.x, pos.z) == end
                })
        })
        .expect("the stroke's edge")
}

// Makes `edges` roads without sidewalks. No tool authors one (the road tool's strokes always
// carry sidewalks; saves may hold them), so the test changes the graph and recompiles the
// closure as an edit would.
fn without_sidewalks(core: &mut SimCore, edges: &[usize]) {
    let mut nodes = HashSet::new();
    for &edge in edges {
        let edge = core.region_graph.edge_mut(edge);
        edge.allowed_types = TransitFlags::CAR;
        nodes.extend([edge.start_node, edge.end_node]);
    }
    let graph = &mut core.region_graph;
    graph.rebuild_intersection_clips();
    graph.finalize_junction_endpoint_profiles(&nodes);
    core.transit_network
        .mark_surface_dirty_for_nodes(&core.region_graph, &nodes);
    core.transit_network
        .roadbed
        .compile_dirty(&core.region_graph);
}

// The XZ points of structure `edge`'s two mouth rows, the free end's (a road end without a
// piece) first if it has one, and the XZ segments of its two outer chains.
type StructureEnds = (Vec<Vec<(f64, f64)>>, bool, Vec<[(f64, f64); 2]>);

fn structure_ends(core: &SimCore, edge: usize) -> StructureEnds {
    let graph = &core.region_graph;
    let roadbed = &core.transit_network.roadbed;
    let span = roadbed.span_piece(edge).expect("the structure's span");
    let record = graph.edge(edge);
    let mut ends = [
        (record.start_node, span.start_mouth()),
        (record.end_node, span.end_mouth()),
    ];
    let free = |node: u32| {
        let node = graph.get_valid_node(node);
        graph.node_adjacency(node).len() == 1 && roadbed.node_piece(node).is_none()
    };
    ends.sort_by_key(|&(node, _)| !free(node));
    let has_free_end = free(ends[0].0);
    let rows = ends
        .iter()
        .map(|(_, mouth)| mouth.vertices.iter().map(|p| (p.x, p.z)).collect())
        .collect();
    let mesh = span.mesh();
    let chains = mesh
        .outer_chains()
        .iter()
        .flat_map(|chain| chain.vertices().windows(2))
        .map(|pair| {
            [pair[0], pair[1]].map(|v| {
                let p = mesh.positions()[v as usize];
                (p.x, p.z)
            })
        })
        .collect();
    (rows, has_free_end, chains)
}

// The family cases: strokes, the stroke ends whose edges change, and the class they take
// (`set_edge_class_internal`'s code), or `None` for roads without sidewalks (with no end, the
// strokes alone).
type Case = (&'static str, Vec<Stroke>, Vec<Vector2>, Option<u8>);

fn cases() -> Vec<Case> {
    let (west, east) = (THROUGH.0[0], THROUGH.0[1]);
    let (side, on) = (SIDE.0[1], ON.0[1]);
    vec![
        ("bridge_t", vec![THROUGH, SIDE], vec![side], Some(1)),
        ("bridge_continuation", vec![THROUGH, ON], vec![on], Some(1)),
        ("tunnel_t", vec![THROUGH, SIDE], vec![side], Some(2)),
        ("tunnel_continuation", vec![THROUGH, ON], vec![on], Some(2)),
        ("rural_side_road", vec![THROUGH, SIDE], vec![side], None),
        ("rural_t", vec![THROUGH, SIDE], vec![west, east, side], None),
        // A two-lane road end continued straight by a four-lane road: a layout with D4 tapers.
        ("widening", vec![THROUGH, (ON.0, 2)], vec![], None),
    ]
}

#[test]
fn structure_and_rural_road_chunks_are_watertight() {
    let mut failures = Vec::new();
    for sloped in [false, true] {
        for (name, strokes, ends, class) in cases() {
            let order: Vec<usize> = (0..strokes.len()).collect();
            let mut core = committed(&strokes, &order, sloped);
            let edges: Vec<_> = ends.iter().map(|&end| edge_ending_at(&core, end)).collect();
            match class {
                Some(class) => {
                    core.set_edge_class_internal(edges[0] as i32, class);
                    let want = if class == 1 {
                        EdgeClass::Bridge
                    } else {
                        EdgeClass::Tunnel
                    };
                    assert_eq!(core.region_graph.edge(edges[0]).class, want, "{name}");
                }
                None if !edges.is_empty() => without_sidewalks(&mut core, &edges),
                None => {}
            }
            // Publish the edit's pieces, as the frame after it does.
            core.precompute_road_mesh_data();
            let roadbed = &core.transit_network.roadbed;
            assert!(roadbed.is_current(), "{name}/{sloped}");
            assert!(roadbed.failed_nodes().is_empty(), "{name}/{sloped}");
            // A structure that ends without a continuation ends on its mouth row there. At both
            // of its mouth rows its slab and fascias end, and so do the skirts of the grounded
            // pieces it meets; a tunnel's sides have no strips, since its bore is its portals'.
            // Abutments and portals close them (`TERRAIN-07-C`). Everything else is watertight.
            let (rows, has_free_end, chains) = match class {
                Some(_) => structure_ends(&core, edges[0]),
                None => Default::default(),
            };
            let tunnel = class == Some(2);
            let problems = open_road_edges_where(&mut core, |layer, a, b| {
                const CONCRETE: i32 = 4;
                // Chunk vertices are the pieces' f64 positions rounded to f32 in the chunk frame.
                let near = |p: (f64, f64, f64), (x, z): (f64, f64)| {
                    (x - p.0).abs() < 1e-4 && (z - p.2).abs() < 1e-4
                };
                let on_row = |row: &Vec<(f64, f64)>, p| row.iter().any(|&q| near(p, q));
                let on_both = |row: &Vec<(f64, f64)>| on_row(row, a) && on_row(row, b);
                let vertical = (a.0, a.2) == (b.0, b.2);
                let on_chain = |p: (f64, f64, f64)| {
                    chains.iter().any(|&[s, t]| {
                        let (dx, dz) = (t.0 - s.0, t.1 - s.1);
                        let along = ((p.0 - s.0) * dx + (p.2 - s.1) * dz) / (dx * dx + dz * dz);
                        let closest = (
                            s.0 + dx * along.clamp(0.0, 1.0),
                            s.1 + dz * along.clamp(0.0, 1.0),
                        );
                        near(p, closest)
                    })
                };
                (has_free_end && on_both(&rows[0]))
                    || ((layer == CONCRETE || vertical) && rows.iter().any(on_both))
                    || (tunnel && on_chain(a) && on_chain(b))
            });
            if !problems.is_empty() {
                failures.push(format!(
                    "{name}/{sloped}: {} problems\n  {}",
                    problems.len(),
                    problems[..problems.len().min(12)].join("\n  ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// `ROAD-68` open question 17, roads without sidewalks: the walking planner's whole-network proof
// requires foot lanes both ways on every edge, so one road without sidewalks withdraws it
// whatever its junctions do. The T with sidewalks keeps it.
#[test]
fn a_road_without_sidewalks_withdraws_the_walking_proof() {
    use crate::simulation::economy::agents::tick::TripQueryScratch;
    for rural in [false, true] {
        let mut core = committed(&[THROUGH, SIDE], &[0, 1], false);
        if rural {
            let side = edge_ending_at(&core, SIDE.0[1]);
            without_sidewalks(&mut core, &[side]);
        }
        let mut graph = core.region_graph.clone();
        core.transit_network.lane_system.rebuild(&mut graph);
        let mut scratch = TripQueryScratch::default();
        scratch.prepare(&core.transit_network, &graph);
        assert_eq!(scratch.walking_proof_complete_for_test(), !rural);
    }
}
