// SPDX-License-Identifier: GPL-2.0-only

//! The remaining road families on the reference harness (`ROAD-68-F` part 2), flat and on its
//! 2.5% slope: bridge decks and tunnel road pieces where they meet grounded roads at a junction's
//! node piece or a pass-through's span mouth, and roads without sidewalks (shoulders) at a T. The
//! published road chunks, skirts and bridge concrete included, are watertight as `terrain_seams`
//! measures the reference junctions: a bridge's fascias (`TERRAIN-07-B`) close its deck's sides
//! against the deck edge and the slab, a structure's free end is capped and a bridge's mouth
//! shared with a grounded piece is closed below its slab by the abutment face (`TERRAIN-07-C`). The
//! terrain side of structures (abutments and portals) is `TERRAIN-07`'s, so the terrain is not
//! checked here.

use super::road_edit_order::{ARM_M, Stroke, committed};
use super::terrain_seams::open_road_edges_where;
use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::simulation::network::roadbed::PieceId;
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
    graph.finalize_junction_endpoint_profiles(&nodes, Some(&core.heightmap), &mut HashSet::new());
    core.transit_network
        .mark_surface_dirty_for_nodes(&core.region_graph, &nodes);
    core.transit_network
        .roadbed
        .compile_dirty(&core.region_graph);
}

// The XZ points of structure `edge`'s mouth rows where it meets another piece, without its free
// end's (a road end without a piece, which its cap closes), and the XZ segments of its two outer
// chains.
type StructureEnds = (Vec<Vec<(f64, f64)>>, Vec<[(f64, f64); 2]>);

fn structure_ends(core: &SimCore, edge: usize) -> StructureEnds {
    let graph = &core.region_graph;
    let roadbed = &core.transit_network.roadbed;
    let span = roadbed.span_piece(edge).expect("the structure's span");
    let record = graph.edge(edge);
    let free = |node: u32| {
        let node = graph.get_valid_node(node);
        graph.node_adjacency(node).len() == 1 && roadbed.node_piece(node).is_none()
    };
    let rows = [
        (record.start_node, span.start_mouth()),
        (record.end_node, span.end_mouth()),
    ]
    .iter()
    .filter(|(node, _)| !free(*node))
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
    (rows, chains)
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
            // A structure that ends without a continuation is capped there (`TERRAIN-07-C`). At
            // a mouth row it shares with a grounded piece, a bridge's slab end edge is closed by
            // its abutment face, whose bottom edge is a skirt bottom across the row that the
            // terrain's abutment wall continues to the graded ground; the fascias, the abutment
            // face and the grounded piece's skirts end in vertical edges on the row's corner
            // lines, of different depths (T-vertices on one line, no gap). A tunnel's sides have
            // no strips, since its bore is its portals'. Everything else is watertight.
            let (rows, chains) = match class {
                Some(_) => structure_ends(&core, edges[0]),
                None => Default::default(),
            };
            let tunnel = class == Some(2);
            let problems = open_road_edges_where(&mut core, |skirt_bottom, a, b| {
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
                ((skirt_bottom || vertical) && rows.iter().any(on_both))
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

// Every published piece's vertices by owner: what an undo must put back bitwise.
fn piece_positions(core: &SimCore) -> Vec<(PieceId, Vec<[u64; 3]>)> {
    let roadbed = &core.transit_network.roadbed;
    let graph = &core.region_graph;
    let spans = (0..graph.edge_count()).map(PieceId::Span);
    let nodes = (0..graph.node_count() as u32).map(PieceId::Node);
    spans
        .chain(nodes)
        .filter_map(|id| {
            let mesh = roadbed.piece_mesh(id)?;
            let bits = mesh
                .positions()
                .iter()
                .map(|p| [p.x, p.y, p.z].map(f64::to_bits));
            Some((id, bits.collect()))
        })
        .collect()
}

// `TERRAIN-07-C` structure-end acceptance (T22 3 (a)): a bridge and a tunnel drawn in play from a
// road, at a T's side road end (a node piece's mouth) and straight on from the through road's end
// (a pass-through's shared mouth row), flat and sloped, commit through the player's path: the
// grounded piece closes its chains across the structure's mouth with a wall run, so the exact
// cut's loops join and question 19's `roadbed_chain_open_end` rejection is gone. Undo puts the
// graph and every piece back.
#[test]
fn a_bridge_and_a_tunnel_drawn_from_a_road_commit_and_undo() {
    let cases = [
        (
            "side_road_end",
            vec![THROUGH, SIDE],
            SIDE.0[1],
            Vector2::new(0.0, 2.0 * ARM_M),
        ),
        ("straight_on", vec![THROUGH], THROUGH.0[1], ON.0[1]),
    ];
    for sloped in [false, true] {
        for (rise, class) in [(6.0, EdgeClass::Bridge), (-6.0, EdgeClass::Tunnel)] {
            for (name, strokes, from, to) in &cases {
                let case = format!("{name}/{class:?}/{sloped}");
                let order: Vec<usize> = (0..strokes.len()).collect();
                let mut core = committed(strokes, &order, sloped);
                core.benchmark_mode = false;
                core.precompute_road_mesh_data();
                let (edges, pieces) = (core.region_graph.edge_count(), piece_positions(&core));
                let at = |core: &SimCore, point: Vector2, rise: f32| {
                    Vector3::new(
                        point.x,
                        core.get_world_surface_height_internal(point) + rise,
                        point.y,
                    )
                };
                let points = vec![at(&core, *from, 0.0), at(&core, *to, rise)];
                let outcome = core.commit_road(
                    RoadCommitRequest {
                        points,
                        fwd_lanes: 1,
                        bkw_lanes: 1,
                        snap_to_existing_roads: true,
                        preview: None,
                    },
                    std::time::Instant::now(),
                    Default::default(),
                );
                assert!(outcome.committed, "{case}: {}", outcome.rejection);
                let structure = edge_ending_at(&core, *to);
                assert_eq!(core.region_graph.edge(structure).class, class, "{case}");
                core.precompute_road_mesh_data();
                let roadbed = &core.transit_network.roadbed;
                assert!(
                    roadbed.is_current() && roadbed.failed_nodes().is_empty(),
                    "{case}"
                );
                let walls = (0..core.region_graph.edge_count())
                    .map(PieceId::Span)
                    .chain((0..core.region_graph.node_count() as u32).map(PieceId::Node))
                    .filter_map(|id| roadbed.piece_mesh(id))
                    .map(|mesh| mesh.wall_runs().len())
                    .sum::<usize>();
                assert_eq!(
                    walls, 1,
                    "{case}: one wall run closes the structure's mouth"
                );
                assert!(core.undo_action_internal(), "{case}");
                core.precompute_road_mesh_data();
                assert_eq!(core.region_graph.edge_count(), edges, "{case}");
                assert!(structure >= edges || core.region_graph.edge(structure).deleted);
                assert!(
                    piece_positions(&core) == pieces,
                    "{case}: undo restores every piece"
                );
            }
        }
    }
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
