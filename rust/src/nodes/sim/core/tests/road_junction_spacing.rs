// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-118` round JS1 on the user's network of `dump-2026-11-15_11-27-06` (15 nodes, 16 1 + 1
//! roads, laid flat): a road end reserves only its cap, so a T beside one builds (the dump's
//! attempts 6 and the end of 5); the preview is the commit and undo restores the graph. Making
//! the road end a bend brings the T back into the edit, which then refuses with the place named.
//! Round JS2: an empty joint beside a junction that does not fit dissolves, its two roads joined
//! into one (the dump's attempts 1 to 4 and the start of 5); an occupied joint keeps its place.

use super::road_edit_order::{Stroke, committed_on};
use super::road_preview::{assert_preview_composes_committed, context, preview};
use super::*;
use crate::nodes::sim::core::scenario::save_cases::assert_same_network;
use crate::nodes::sim::core::road_commit::{RoadCommitOutcome, RoadCommitRequest};
use crate::nodes::sim::core::road_preview::RoadPreview;
use crate::nodes::sim::core::road_rejection::RoadRejection;
use crate::nodes::sim::road_tool::wording::{ReasonFacts, reason_text};
use crate::simulation::network::road_edit::RoadPreviewValidation;
use crate::simulation::network::roadbed::{NodeArms, NodeRole};
use godot::prelude::Vector2;
use std::sync::Arc;

// The dump's node positions (XZ, rounded to the millimetre) and its edges, in edge order.
const NODES: [(f32, f32); 15] = [
    (-39.403, 1.762),
    (88.580, 10.611),
    (5.490, 4.866),
    (9.628, -54.991),
    (79.461, -50.163),
    (1.351, 64.723),
    (-58.506, 60.585),
    (-94.272, -2.032),
    (-64.344, 0.038),
    (-71.241, 99.800),
    (84.442, 70.468),
    (-68.482, 59.895),
    (-98.411, 57.826),
    (-96.342, 27.897),
    (-156.199, 23.758),
];
const EDGES: [(usize, usize); 16] = [
    (0, 2),
    (2, 3),
    (2, 1),
    (3, 4),
    (2, 5),
    (5, 6),
    (0, 8),
    (8, 11),
    (8, 7),
    (1, 10),
    (5, 10),
    (11, 12),
    (11, 9),
    (12, 13),
    (13, 14),
    (13, 7),
];
// The dead end of edge 5 and the joint of edges 0 and 6.
const DEAD_END: usize = 6;
const JOINT: usize = 0;

fn at(node: usize) -> Vector2 {
    Vector2::new(NODES[node].0, NODES[node].1)
}

fn dump_city() -> SimCore {
    let strokes: Vec<Stroke> = EDGES.iter().map(|&(a, b)| ([at(a), at(b)], 1)).collect();
    let order: Vec<usize> = (0..strokes.len()).collect();
    let mut core = committed_on(512.0, &strokes, &order, false);
    // Edits record their undo.
    core.benchmark_mode = false;
    assert_eq!(live_edges(&core), 16, "the dump's roads");
    core
}

fn live_edges(core: &SimCore) -> usize {
    core.region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted)
        .count()
}

// The point on edge 5 `from_end_m` from its dead end.
fn on_edge_5(from_end_m: f32) -> Vector2 {
    at(DEAD_END) + (at(5) - at(DEAD_END)).normalized() * from_end_m
}

fn stroke(core: &SimCore, ends: [Vector2; 2]) -> Vec<Vector3> {
    ends.map(|point| {
        Vector3::new(
            point.x,
            core.get_world_surface_height_internal(point),
            point.y,
        )
    })
    .to_vec()
}

fn commit(
    core: &mut SimCore,
    points: &[Vector3],
    preview: Option<Arc<RoadPreview>>,
) -> RoadCommitOutcome {
    core.commit_road(
        RoadCommitRequest {
            parking: Default::default(),
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
            preview,
        },
        std::time::Instant::now(),
        Default::default(),
    )
}

// Every live node and edge record, bit for bit: positions, aliases, ends, both polylines, clips.
fn graph_hash(core: &SimCore) -> u64 {
    use std::hash::{Hash, Hasher};
    let graph = &core.region_graph;
    let mut hasher = std::hash::DefaultHasher::new();
    let bits = |point: &Vector3| point.to_array().map(f32::to_bits);
    for id in 0..graph.node_count() as u32 {
        (graph.get_valid_node(id), bits(&graph.node(id).pos)).hash(&mut hasher);
    }
    for edge in graph.edges().iter().filter(|edge| !edge.deleted) {
        (edge.start_node, edge.end_node).hash(&mut hasher);
        edge.geometry
            .iter()
            .map(bits)
            .for_each(|point| point.hash(&mut hasher));
        edge.physical_geometry
            .iter()
            .map(bits)
            .for_each(|point| point.hash(&mut hasher));
        [edge.start_clip, edge.end_clip]
            .map(f32::to_bits)
            .hash(&mut hasher);
    }
    hasher.finish()
}

// The node at `point`, and its smallest corner radius.
fn node_at(core: &SimCore, point: Vector2) -> (u32, f64) {
    let node = core
        .region_graph
        .find_node_within(Vector3::new(point.x, 0.0, point.y), 0.5)
        .or_else(|| {
            let height = core.get_world_surface_height_internal(point);
            core.region_graph
                .find_node_within(Vector3::new(point.x, height, point.y), 0.5)
        })
        .expect("a node there");
    let layout = NodeArms::collect(&core.region_graph, node)
        .expect("the node has arms")
        .layout()
        .expect("the node lays out");
    let radius = layout
        .sectors
        .iter()
        .filter_map(|sector| sector.corner.map(|corner| corner.radius_m))
        .fold(f64::INFINITY, f64::min);
    (node, radius)
}

fn degree(core: &SimCore, node: u32) -> usize {
    core.region_graph
        .node_adjacency(node)
        .iter()
        .filter(|&&edge| !core.region_graph.edge(edge).deleted)
        .count()
}

// The dump's attempt 6: from the joint at node 0 to the point on edge 5 15 m from its dead end.
// Refused before (a 7.5 m budget for an 8.5 m mouth), it builds a T with a 15 m stub at its full
// radius; node 0 becomes a T too. The preview is the commit, and undo restores the graph.
#[test]
fn the_dumps_attempt_6_builds_a_t_with_its_15_m_stub() {
    let mut core = dump_city();
    let before = graph_hash(&core);
    let points = stroke(&core, [at(JOINT), on_edge_5(15.0)]);
    let mut context = context(&mut core);
    let view_chunks = context.view.road_mesh_chunks.clone();
    let preview = Arc::new(preview(&mut context, &points));
    assert!(
        preview.validation.is_valid,
        "{}",
        preview.validation.invalid_reason
    );
    let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
    let outcome = commit(&mut core, &points, Some(Arc::clone(&preview)));
    assert!(
        outcome.committed,
        "{} {}",
        outcome.rejection_reason, outcome.rejection
    );
    assert_preview_composes_committed(&core, &view_chunks, pieces, "attempt 6");
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
    let (t, radius) = node_at(&core, on_edge_5(15.0));
    assert_eq!(degree(&core, t), 3, "a T on edge 5");
    assert_eq!(radius, 3.5, "the 15 m stub leaves the full radius");
    assert_eq!(degree(&core, JOINT as u32), 3, "the joint became a T");
    let stub = core
        .region_graph
        .node_adjacency(t)
        .iter()
        .map(|&edge| core.region_graph.edge(edge))
        .find(|edge| [edge.start_node, edge.end_node].contains(&(DEAD_END as u32)))
        .expect("the stub to the dead end");
    let length_m = core.region_graph.calculate_length(&stub.geometry);
    assert!((length_m - 15.0).abs() < 0.05, "{length_m}");
    assert!(core.undo_action_internal());
    assert_eq!(graph_hash(&core), before, "undo restores the graph");
}

// The end of the dump's attempt 5 alone: a T 10 m from the dead end builds at a reduced radius.
// Then the dead end extended sideways is a bend 10 m from the T, which needs 17 m: the edit lays
// the T out again and refuses, naming the place; a T 6 m from the dead end is refused as a road
// end too close, and the report names the road the stroke ends on.
#[test]
fn a_t_10_m_from_a_road_end_builds_and_the_road_end_then_keeps_its_place() {
    let mut core = dump_city();
    // Along attempt 5's line, square to edge 5, from 30 m short of the road.
    let back = (Vector2::new(-44.391, 1.417) - Vector2::new(-48.530, 61.274)).normalized();
    let start = on_edge_5(10.0) + back * 30.0;
    let points = stroke(&core, [start, on_edge_5(10.0)]);
    let outcome = commit(&mut core, &points, None);
    assert!(
        outcome.committed,
        "{} {}",
        outcome.rejection_reason, outcome.rejection
    );
    let (t, radius) = node_at(&core, on_edge_5(10.0));
    assert_eq!(degree(&core, t), 3);
    assert!((2.0..3.5).contains(&radius), "a reduced radius: {radius}");

    // Sideways off the dead end, a quarter turn from edge 5.
    let along = (at(5) - at(DEAD_END)).normalized();
    let sideways = at(DEAD_END) + Vector2::new(along.y, -along.x) * 30.0;
    let sideways = if sideways.y < at(DEAD_END).y {
        at(DEAD_END) - Vector2::new(along.y, -along.x) * 30.0
    } else {
        sideways
    };
    let points = stroke(&core, [at(DEAD_END), sideways]);
    let mut view = context(&mut core);
    let refused = preview(&mut view, &points).validation;
    assert!(!refused.is_valid);
    assert!(
        matches!(
            refused.invalid_reason,
            "junction_corner_does_not_fit" | "junction_arm_too_short"
        ),
        "{}",
        refused.invalid_reason
    );
    let place = refused.junction_place.expect("the refusal names its place");
    let mut roles = place.roles;
    roles.sort_by_key(|role| role.code());
    assert_eq!(roles, [NodeRole::Junction, NodeRole::Bend]);
    assert!((refused.geometry_measured - 10.0).abs() < 0.05);
    assert_eq!(refused.geometry_limit, 17.0);
    let text = reason_text(refused.invalid_reason, &facts(&refused)).unwrap();
    assert!(
        text == "Junction only 10.0 m from the bend; needs 17.0 m"
            || text == "Bend only 10.0 m from the junction; needs 17.0 m",
        "{text}"
    );
    let outcome = commit(&mut core, &points, None);
    assert!(!outcome.committed, "the commit refuses it too");

    // A T 6 m from the dead end of edge 3 was a road end too close; since round JS3 its 6 m stub
    // goes and the dead end merges into the stroke's end, a bend (`stubs_and_joins`).
    let (near, far) = (at(4), at(3));
    let end = near + (far - near).normalized() * 6.0;
    // Square to edge 3, from the north.
    let along = (far - near).normalized();
    let north = Vector2::new(-along.y, along.x) * along.x.signum();
    let points = stroke(&core, [end + north * 30.0, end]);
    let outcome = commit(&mut core, &points, None);
    assert!(outcome.committed, "{}", outcome.rejection_reason);
    assert!(!node_stands_at(&core, near), "the dead end merged into the bend");
}

mod contacts;
mod lead_ins;
mod stubs_and_joins;

fn facts(validation: &RoadPreviewValidation) -> ReasonFacts {
    ReasonFacts {
        geometry_measured: Some(f64::from(validation.geometry_measured)),
        geometry_limit: f64::from(validation.geometry_limit),
        junction_roles: validation.junction_place.map(|place| place.roles),
        ..ReasonFacts::default()
    }
}

// The dump's junction on the line of roads 8-11-9, where edge 5 continued straight on from its
// dead end meets that line 10 m on.
const JUNCTION: usize = 11;

// The dump's strokes 1 to 5 (`ROAD-118` round JS2): from the dead end straight on to the junction
// 10 m away; the same 30 m on, across the line of roads; from the junction back to the dead end;
// from the joint's road 5 m from the joint to edge 5 10 m from its dead end.
fn attempts() -> [(&'static str, [Vector2; 2]); 4] {
    let along = (at(DEAD_END) - at(5)).normalized();
    [
        ("attempt 1", [at(DEAD_END), at(DEAD_END) + along * 10.0]),
        ("attempt 2", [at(DEAD_END), at(DEAD_END) + along * 30.0]),
        ("attempts 3 and 4", [at(JUNCTION), at(DEAD_END)]),
        (
            "attempt 5",
            [
                at(JOINT) + (at(8) - at(JOINT)).normalized() * 5.0,
                on_edge_5(10.0),
            ],
        ),
    ]
}

// Whether a live node stands within half a metre of `point`.
fn node_stands_at(core: &SimCore, point: Vector2) -> bool {
    let graph = &core.region_graph;
    (0..graph.node_count() as u32).any(|node| {
        let pos = graph.node(node).pos;
        graph.get_valid_node(node) == node
            && degree(core, node) > 0
            && Vector2::new(pos.x, pos.z).distance_to(point) < 0.5
    }) || graph
        .find_node_within(
            Vector3::new(point.x, core.get_world_surface_height_internal(point), point.y),
            0.5,
        )
        .is_some_and(|node| {
            let pos = graph.node(node).pos;
            Vector2::new(pos.x, pos.z).distance_to(point) < 0.5
        })
}

// The dump's attempts 1 to 5, each on a fresh copy of the network: refused before JS2, each now
// builds. The preview is the commit; the joint is gone (no node there, none found there); edge 5
// keeps its id and runs to the junction (1 to 4); the joint of edges 0 and 6 dissolves into the
// new T (5). Undo restores the graph and the roadbed bit for bit, and the stroke committed again
// builds the same graph and roadbed.
#[test]
fn the_dumps_attempts_1_to_5_dissolve_the_joint_beside_the_junction() {
    for (label, ends) in attempts() {
        let mut core = dump_city();
        let before = (graph_hash(&core), roadbed_digest(&core));
        let points = stroke(&core, ends);
        let mut context = context(&mut core);
        let view_chunks = context.view.road_mesh_chunks.clone();
        let preview = Arc::new(preview(&mut context, &points));
        assert!(
            preview.validation.is_valid,
            "{label}: {}",
            preview.validation.invalid_reason
        );
        let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
        let outcome = commit(&mut core, &points, Some(Arc::clone(&preview)));
        assert!(
            outcome.committed,
            "{label}: {} {}",
            outcome.rejection_reason, outcome.rejection
        );
        assert_preview_composes_committed(&core, &view_chunks, pieces, label);
        assert!(core.transit_network.roadbed.failed_nodes().is_empty(), "{label}");
        let graph = &core.region_graph;
        if label == "attempt 5" {
            assert!(!node_stands_at(&core, at(JOINT)), "{label}: the joint is gone");
            let (t, _) = node_at(&core, ends[0]);
            assert_eq!(degree(&core, t), 3, "{label}: a T on the joint's line");
            assert!(!graph.edge(0).deleted && !graph.edge(6).deleted, "{label}: ids kept");
            assert!(
                [graph.edge(0).start_node, graph.edge(0).end_node].contains(&t)
                    && [graph.edge(6).start_node, graph.edge(6).end_node].contains(&t),
                "{label}: edge 0 runs on to the T, edge 6 from it"
            );
        } else {
            assert!(!node_stands_at(&core, at(DEAD_END)), "{label}: the joint is gone");
            let (junction, _) = node_at(&core, at(JUNCTION));
            let edge = graph.edge(5);
            assert!(!edge.deleted, "{label}: edge 5 keeps its id");
            assert_eq!(edge.end_node, junction, "{label}: edge 5 runs to node 11");
            assert_eq!(degree(&core, junction), 4, "{label}: a crossroads");
        }
        let after = (graph_hash(&core), roadbed_digest(&core));
        assert!(core.undo_action_internal(), "{label}");
        core.precompute_road_mesh_data();
        assert_eq!(
            (graph_hash(&core), roadbed_digest(&core)),
            before,
            "{label}: undo restores the graph and the roadbed"
        );
        assert!(node_stands_at(&core, at(DEAD_END)) && node_stands_at(&core, at(JOINT)));
        let outcome = commit(&mut core, &points, None);
        assert!(outcome.committed, "{label}: again");
        core.precompute_road_mesh_data();
        assert_eq!(
            (graph_hash(&core), roadbed_digest(&core)),
            after,
            "{label}: committed again, the same graph and roadbed"
        );
    }
}

// `ROAD-124`: a dissolved joint is merged into its junction, a hole with its alias, so a save
// and a load find the running game's network (ids, holes, aliases, node grid): no node stands
// at the joint's place and none is left without a road, and a stroke drawn to that place builds
// (or is refused) alike in both games, finding the road, not the joint. `ROAD-125`: attempt 5
// prepends the joint's road, now running to the T, to the slanted line of edge 2; the game saves,
// its road grid alignments load as the edit left them and the joint keeps no connectors.
#[test]
fn a_dissolved_joint_is_a_hole_live_and_loaded() {
    for (label, ends) in attempts() {
        let mut core = dump_city();
        let joint = if label == "attempt 5" {
            JOINT
        } else {
            DEAD_END
        };
        let points = stroke(&core, ends);
        assert!(commit(&mut core, &points, None).committed, "{label}");
        let graph = &core.region_graph;
        assert_ne!(graph.get_valid_node(joint as u32), joint as u32, "{label}: merged");
        let mut loaded = saved_and_loaded(&mut core, "dissolved_joint");
        assert_eq!(
            loaded.zoning.cells.saved_road_alignments(),
            core.zoning.cells.saved_road_alignments(),
            "{label}"
        );
        let place = at(joint);
        let to = stroke(&core, [place + Vector2::new(0.0, -50.0), place]);
        let mut outcomes = Vec::new();
        for game in [&mut core, &mut loaded] {
            assert!(!node_stands_at(game, place), "{label}");
            assert!(roadless_nodes(game).is_empty(), "{label}");
            outcomes.push(commit(game, &to, None).committed);
        }
        assert_eq!(outcomes[0], outcomes[1], "{label}");
        assert_same_network(&core, &loaded, label);
    }
}

// Joints that are not empty keep their place: an authored crossing at the joint, or the joint's
// two roads at different speeds (attempt 5's start). The junction beside them was refused with
// its place named; round JS3 sheared the stroke onto the joint, a T. Since round JS5a nothing
// drawn moves: the end, on the road 5 m from the joint, could join it only through an arc whose
// arm meets the joint's road at 39 degrees or less, a corner that does not fit, so the drawn
// stroke's refusal stands with its place, and the joint keeps its two roads.
#[test]
fn occupied_joints_keep_their_place() {
    let start = at(JOINT) + (at(8) - at(JOINT)).normalized() * 5.0;
    let side = start + Vector2::new(0.0, -30.0);
    for occupied in ["crossing", "speed"] {
        let mut core = dump_city();
        let applied = match occupied {
            "crossing" => core.set_crosswalk_override_internal(JOINT as u32, 0, false),
            // The joint's road on one side at 40 km/h, in place.
            _ => core.set_edge_cross_section_internal(
                0,
                1,
                1,
                None,
                Default::default(),
                crate::simulation::network::road_types::RoadAttributes::new(
                    40,
                    crate::simulation::network::graph::SidewalkSides::BOTH,
                    true,
                ),
            ),
        };
        assert!(applied, "{occupied}");
        let _ = core.finalize_network_render_products();
        let points = stroke(&core, [side, start]);
        let mut view = context(&mut core);
        let refused = preview(&mut view, &points).validation;
        assert!(!refused.is_valid, "{occupied}");
        assert!(
            matches!(
                refused.invalid_reason,
                "junction_corner_does_not_fit" | "junction_arm_too_short"
            ),
            "{occupied}: {}",
            refused.invalid_reason
        );
        assert!(refused.junction_place.is_some(), "{occupied}: the place is named");
        assert_eq!(refused.joined_nodes, [None; 2], "{occupied}");
        assert!(!commit(&mut core, &points, None).committed, "{occupied}");
        assert_eq!(degree(&core, JOINT as u32), 2, "{occupied}: the joint keeps its place");
    }
}

// Every edge's and node's published piece.
fn roadbed_digest(core: &SimCore) -> u64 {
    use crate::simulation::network::roadbed::PieceId;
    let roadbed = &core.transit_network.roadbed;
    let spans: Vec<_> = (0..core.region_graph.edge_count())
        .filter(|&edge| !core.region_graph.edge(edge).deleted)
        .map(|edge| roadbed.piece_mesh(PieceId::Span(edge)))
        .collect();
    let nodes: Vec<_> = (0..core.region_graph.node_count() as u32)
        .filter(|&node| core.region_graph.get_valid_node(node) == node)
        .map(|node| roadbed.piece_mesh(PieceId::Node(node)))
        .collect();
    crate::debug::work::debug_hash(&(spans, nodes))
}

// Attempts 1 and 5 as one batch: both joints dissolve, the batch leaves the graph and roadbed as
// the two strokes committed one by one, its records share one undo group, and one undo restores
// the network bit for bit.
#[test]
fn a_batch_of_dissolving_strokes_is_one_undo_group() {
    use crate::nodes::sim::core::RoadBatchRequest;
    let [(_, first), _, _, (_, fifth)] = attempts();
    let mut single = dump_city();
    let strokes = [first, fifth].map(|ends| stroke(&single, ends));
    for points in &strokes {
        single.precompute_road_mesh_data();
        assert!(commit(&mut single, points, None).committed);
    }
    single.precompute_road_mesh_data();

    let mut core = dump_city();
    core.precompute_road_mesh_data();
    let before = (graph_hash(&core), roadbed_digest(&core));
    let depth = core.undo_stack.len();
    let request = |points: &Vec<Vector3>| RoadCommitRequest {
        parking: Default::default(),
        points: points.clone(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
        median: None,
        attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        preview: None,
    };
    let batch = core.commit_road_batch(
        RoadBatchRequest {
            strokes: strokes.iter().map(request).collect(),
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(batch.outcome.committed, "{}", batch.outcome.rejection_reason);
    assert!(!node_stands_at(&core, at(DEAD_END)) && !node_stands_at(&core, at(JOINT)));
    core.precompute_road_mesh_data();
    assert_eq!(
        (graph_hash(&core), roadbed_digest(&core)),
        (graph_hash(&single), roadbed_digest(&single)),
        "the batch is its strokes one by one"
    );
    let groups: Vec<u64> = core.undo_stack.iter().skip(depth).map(|record| record.group).collect();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0], groups[1], "one undo group");
    assert!(core.undo_action_internal());
    assert_eq!(core.undo_stack.len(), depth);
    core.precompute_road_mesh_data();
    assert_eq!((graph_hash(&core), roadbed_digest(&core)), before);
}
