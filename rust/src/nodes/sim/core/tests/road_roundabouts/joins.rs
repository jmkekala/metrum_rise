// SPDX-License-Identifier: GPL-2.0-only

//! Strokes joined to a bare ring (`ROAD-62`, D9, round R7b): the player's main workflow, a bare
//! ring placed and an ordinary crossroads drawn onto it. A join near a bare node takes that node's
//! place, so only two arms closer than their mouths refuse.

use super::*;
use crate::nodes::sim::core::RoadCommitRequest;
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::roadbed::NodeArms;

const FAR_M: f32 = 90.0;

// The end of a stroke onto a ring at `angle_deg`, and its start `FAR_M` beyond.
fn stroke_at(radius_m: f32, angle_deg: f32) -> Vec<Vector3> {
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let at = |r: f32| Vector3::new(cos * r, 0.0, sin * r);
    vec![at(radius_m + FAR_M), at(radius_m)]
}

fn bare(diameter_m: f32, lanes: u8) -> (SimCore, f32) {
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e7;
    assert_eq!(
        ring_edit(&mut core, ground(Vector3::ZERO, lanes, diameter_m)),
        ""
    );
    let radius_m = 0.5 * (diameter_m - f32::from(lanes) * crate::config::LANE_WIDTH);
    (core, radius_m)
}

// Draws one stroke the way the road tool commits it, the preview's plan being the commit's.
fn draw(core: &mut SimCore, points: Vec<Vector3>, lanes: i32) -> Result<(), String> {
    core.precompute_road_mesh_data();
    let outcome = core.commit_road(
        RoadCommitRequest {
            points,
            fwd_lanes: lanes,
            bkw_lanes: lanes,
            snap_to_existing_roads: true,
            tunnel: false,
            median: None,
            attributes: RoadAttributes::DEFAULT,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    if outcome.committed {
        Ok(())
    } else {
        Err(outcome.rejection_reason)
    }
}

// Every ring edge under a half turn and level, its nodes laid out, its pieces compiled.
fn assert_ring_sound(core: &mut SimCore) {
    core.precompute_road_mesh_data();
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
    let graph = &core.region_graph;
    for id in ring_edges(core) {
        let edge = graph.edge(id);
        let ring = edge.roundabout.unwrap();
        let (a, b) = (
            graph.node(edge.start_node).pos,
            graph.node(edge.end_node).pos,
        );
        let angle = |p: Vector3| (p.z - ring.centre.y).atan2(p.x - ring.centre.x);
        let turn = (angle(a) - angle(b)).rem_euclid(std::f32::consts::TAU);
        assert!(turn < std::f32::consts::PI - 1e-3, "edge {id} turns {turn}");
        assert_eq!(
            crate::simulation::network::roundabouts::validate_ring_edge(graph, id),
            Ok(())
        );
        for node in [edge.start_node, edge.end_node] {
            if let Some(mut arms) = NodeArms::collect(graph, node) {
                arms.layout().unwrap();
            }
        }
    }
}

fn permutations(items: &[f32]) -> Vec<Vec<f32>> {
    if items.len() < 2 {
        return vec![items.to_vec()];
    }
    let mut all = Vec::new();
    for (i, &first) in items.iter().enumerate() {
        let rest: Vec<f32> = items
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, &x)| x)
            .collect();
        for mut tail in permutations(&rest) {
            tail.insert(0, first);
            all.push(tail);
        }
    }
    all
}

// The main workflow: four compass arms joined to a bare ring in every order, at the default sizes
// and the smallest. Each join commits, and the ring holds exactly its four arms at the end.
#[test]
fn four_compass_arms_join_a_bare_ring_in_every_order() {
    for (diameter_m, lanes, arm) in [(36.0, 1, 1), (30.0, 1, 1), (50.0, 2, 2), (45.0, 2, 2)] {
        for order in permutations(&[0.0, 90.0, 180.0, 270.0]) {
            let (mut core, radius_m) = bare(diameter_m, lanes);
            for &angle in &order {
                draw(&mut core, stroke_at(radius_m, angle), arm)
                    .unwrap_or_else(|why| panic!("D{diameter_m} {order:?} at {angle}: {why}"));
                assert_ring_sound(&mut core);
            }
            let ring_nodes: std::collections::BTreeSet<u32> = ring_edges(&core)
                .into_iter()
                .map(|id| core.region_graph.edge(id).start_node)
                .collect();
            let with_arm = ring_nodes
                .iter()
                .filter(|&&node| live_arms(&core, node).len() == 3)
                .count();
            assert_eq!(with_arm, 4, "D{diameter_m} {order:?}");
        }
    }
}

// A join near a bare node keeps the node, moved, and undo puts the ring back bitwise.
#[test]
fn a_join_near_a_bare_node_slides_it_and_undo_restores_the_ring() {
    let (mut core, radius_m) = bare(36.0, 1);
    let (state, before) = (graph_state(&core), products(&mut core));
    let count = counts(&core);
    draw(&mut core, stroke_at(radius_m, 0.0), 1).unwrap();
    assert_eq!(
        ring_edges(&core).len(),
        3,
        "the bare node moved, nothing split"
    );
    assert_eq!(
        core.region_graph.node_count(),
        count.0 + 1,
        "the stroke's far end"
    );
    assert_ring_sound(&mut core);
    assert!(core.undo_action_internal());
    assert_eq!(graph_state(&core), state);
    assert_eq!(products(&mut core), before);
}

// Opposite arms on a small ring: the second takes the bare node between them, which would leave a
// half turn, so a bare node goes in mid-arc, and comes out on undo.
#[test]
fn a_half_turn_takes_a_bare_node_in_the_middle_of_its_arc() {
    let (mut core, radius_m) = bare(30.0, 1);
    draw(&mut core, stroke_at(radius_m, 0.0), 1).unwrap();
    let (state, before) = (graph_state(&core), products(&mut core));
    draw(&mut core, stroke_at(radius_m, 180.0), 1).unwrap();
    assert_ring_sound(&mut core);
    assert_eq!(ring_edges(&core).len(), 4);
    assert!(core.undo_action_internal());
    assert_eq!(graph_state(&core), state);
    assert_eq!(products(&mut core), before);
}

// The limits of D9 still stand on a ring whose bare nodes moved: a second arm at an arm node, and
// two arms closer than their mouths, with the tool's reason. Nothing changes.
#[test]
fn only_true_crowding_refuses() {
    for (diameter_m, lanes, arm, refused_m, fits_m) in
        [(36.0, 1, 1, 9.0, 11.0), (50.0, 2, 2, 14.0, 16.0)]
    {
        let (mut core, radius_m) = bare(diameter_m, lanes);
        draw(&mut core, stroke_at(radius_m, 0.0), arm).unwrap();
        let state = graph_state(&core);
        let degrees = |arc_m: f32| (arc_m / radius_m).to_degrees();
        let refused = draw(&mut core, stroke_at(radius_m, degrees(refused_m)), arm);
        assert_eq!(refused.unwrap_err(), "roundabout_arms_too_close");
        assert_eq!(graph_state(&core), state, "a refusal changes nothing");
        let mut second = stroke_at(radius_m, 0.0);
        second[0] = stroke_at(radius_m, 30.0)[0];
        let taken = draw(&mut core, second, arm);
        assert_eq!(taken.unwrap_err(), "roundabout_arm_node_taken");
        draw(&mut core, stroke_at(radius_m, degrees(fits_m)), arm).unwrap();
        assert_ring_sound(&mut core);
    }
}

// Any point of a bare ring takes a road: every 7.5 degrees, each width of road, at every size.
// Only a road as wide as the ring refuses: at 30 m on one lane, 3+3 where a join within the snap of
// a bare node comes in oblique, and 4+4 anywhere.
#[test]
fn a_bare_ring_takes_a_road_anywhere() {
    for (diameter_m, lanes) in [
        (30.0, 1),
        (36.0, 1),
        (60.0, 1),
        (45.0, 2),
        (50.0, 2),
        (80.0, 2),
    ] {
        for arm in [1, 2, 3, 4] {
            let mut refused = 0;
            for step in 0..48 {
                let (mut core, radius_m) = bare(diameter_m, lanes);
                match draw(&mut core, stroke_at(radius_m, step as f32 * 7.5), arm) {
                    Ok(()) => assert_ring_sound(&mut core),
                    Err(why) => {
                        assert_eq!(why, "roundabout_arms_too_close");
                        refused += 1;
                    }
                }
            }
            let expected = match (diameter_m, arm) {
                (30.0, 3) => 6,
                (30.0, 4) => 48,
                _ => 0,
            };
            assert_eq!(refused, expected, "D{diameter_m} {arm}+{arm}");
        }
    }
}

// Six arms in any rotation fit a bare ring at the floors, as six equal arms of a conversion do,
// and each undoes back to the ring before it.
#[test]
fn six_arms_join_a_bare_ring_at_the_smallest_size() {
    for (diameter_m, lanes, arm) in [(30.0, 1, 1), (45.0, 2, 2)] {
        for offset in [0.0, 17.0, 30.0, 45.0] {
            let (mut core, radius_m) = bare(diameter_m, lanes);
            for i in 0..6 {
                let angle = offset + 60.0 * i as f32;
                let before = (graph_state(&core), products(&mut core));
                draw(&mut core, stroke_at(radius_m, angle), arm)
                    .unwrap_or_else(|why| panic!("D{diameter_m} +{offset} arm {i}: {why}"));
                assert_ring_sound(&mut core);
                assert!(core.undo_action_internal());
                assert_eq!(graph_state(&core), before.0, "D{diameter_m} +{offset} {i}");
                assert_eq!(products(&mut core), before.1, "D{diameter_m} +{offset} {i}");
                draw(&mut core, stroke_at(radius_m, angle), arm).unwrap();
            }
            assert_eq!(ring_edges(&core).len(), 6, "only the arms' nodes are left");
        }
    }
}

// A stroke may start on a ring as well as end on one, or join two rings: each end takes its own
// node.
#[test]
fn both_ends_of_a_stroke_can_join() {
    let (mut core, radius_m) = bare(36.0, 1);
    let mut outward = stroke_at(radius_m, 0.0);
    outward.reverse();
    draw(&mut core, outward, 1).unwrap();
    assert_ring_sound(&mut core);

    let (mut core, radius_m) = bare(36.0, 1);
    assert_eq!(
        ring_edit(&mut core, ground(Vector3::new(150.0, 0.0, 0.0), 1, 36.0)),
        ""
    );
    let between = vec![
        Vector3::new(radius_m, 0.0, 0.0),
        Vector3::new(150.0 - radius_m, 0.0, 0.0),
    ];
    draw(&mut core, between, 1).unwrap();
    assert_ring_sound(&mut core);
    assert_eq!(ring_edges(&core).len(), 6, "both rings kept their nodes");

    let (mut core, radius_m) = bare(36.0, 1);
    let at = |angle: f32, r: f32| {
        let (sin, cos) = angle.to_radians().sin_cos();
        Vector3::new(cos * r, 0.0, sin * r)
    };
    let around = vec![
        at(0.0, radius_m),
        at(0.0, 70.0),
        at(60.0, 80.0),
        at(120.0, 70.0),
        at(120.0, radius_m),
    ];
    // Every two ring nodes of a bare ring are neighbours, and two roads between the same two
    // nodes are refused anywhere; nothing changes. The ring's refusal says why, a plain road's
    // keeps its words.
    let state = graph_state(&core);
    assert_eq!(
        draw(&mut core, around, 1).unwrap_err(),
        "roundabout_neighbour_nodes"
    );
    assert_eq!(graph_state(&core), state);

    let (a, b) = (Vector3::new(200.0, 0.0, 0.0), Vector3::new(300.0, 0.0, 0.0));
    draw(&mut core, vec![a, b], 1).unwrap();
    let doubled = vec![a, Vector3::new(250.0, 0.0, 40.0), b];
    assert_eq!(
        draw(&mut core, doubled, 1).unwrap_err(),
        "surface_geometry_invalid"
    );
}

// Joins in any order at any angle leave a sound ring or refuse with a ring's reason, and a refusal
// changes nothing. A fixed sequence of pseudo-random angles over every size.
#[test]
fn joins_in_any_sequence_leave_a_sound_ring() {
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed >> 8
    };
    for (diameter_m, lanes, arm) in [
        (30.0, 1, 1),
        (36.0, 1, 1),
        (60.0, 1, 2),
        (45.0, 2, 2),
        (80.0, 2, 1),
    ] {
        for round in 0..6 {
            let (mut core, radius_m) = bare(diameter_m, lanes);
            for _ in 0..7 {
                let angle = (next() % 3600) as f32 * 0.1;
                let before = graph_state(&core);
                match draw(&mut core, stroke_at(radius_m, angle), arm) {
                    Ok(()) => assert_ring_sound(&mut core),
                    Err(why) => {
                        assert!(
                            matches!(
                                why.as_str(),
                                "roundabout_arms_too_close"
                                    | "roundabout_arm_node_taken"
                                    | "surface_geometry_invalid"
                                    | "junction_arm_angle"
                            ),
                            "D{diameter_m} round {round} at {angle}: {why}"
                        );
                        assert_eq!(graph_state(&core), before);
                    }
                }
            }
        }
    }
}

// `ROAD-68-E` on a join: the road tool's replica previews the stroke onto a bare ring, the node's
// slide and the half turn's new node included, and the commit installs that preview's own pieces;
// a fresh commit gives the same pieces bitwise. The replica follows the ring's edits and undo.
#[test]
fn a_preview_near_a_bare_node_is_what_the_commit_installs() {
    use crate::nodes::sim::core::road_preview::{RoadPreviewRequest, road_tool_snapshot_from_core};
    use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
    use crate::simulation::network::roadbed::PieceId;
    use std::sync::Arc;
    let (mut core, radius_m) = bare(36.0, 1);
    let (mut control, _) = bare(36.0, 1);
    for core in [&mut core, &mut control] {
        draw(core, stroke_at(radius_m, 0.0), 1).unwrap();
        core.precompute_road_mesh_data();
        core.refresh_road_terrain_inputs();
    }
    let mut context =
        RoadPreviewContext::capture(road_tool_snapshot_from_core(&core).expect("current"));
    let points = stroke_at(radius_m, 180.0);
    let surface_generation = context.view.surface_generation;
    let request = RoadPreviewRequest {
        surface_generation,
        points: points.clone(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        tunnel: false,
        median: None,
        attributes: RoadAttributes::DEFAULT,
    };
    let preview = Arc::new(context.preview_road(request));
    assert!(
        preview.validation.is_valid,
        "{}",
        preview.validation.invalid_reason
    );
    let compiled = preview
        .pieces
        .as_ref()
        .expect("a valid stroke compiles pieces");
    let commit = |core: &mut SimCore, preview| {
        let outcome = core.commit_road(
            RoadCommitRequest {
                points: points.clone(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
                tunnel: false,
                median: None,
                attributes: RoadAttributes::DEFAULT,
                preview,
            },
            std::time::Instant::now(),
            Default::default(),
        );
        assert!(outcome.committed, "{}", outcome.rejection);
    };
    commit(&mut core, Some(Arc::clone(&preview)));
    assert!(core.last_road_edit_metrics.preview_plan_reused);
    let mesh = |core: &SimCore, id: PieceId| {
        let roadbed = &core.transit_network.roadbed;
        match id {
            PieceId::Span(edge) => roadbed.span_piece(edge).map(|span| span.mesh()),
            PieceId::Node(node) => roadbed.node_piece(node).map(|piece| piece.mesh()),
        }
        .map(std::ptr::from_ref)
    };
    for (id, piece) in compiled.delta.pieces() {
        assert_eq!(mesh(&core, id), piece.map(std::ptr::from_ref), "{id:?}");
    }
    commit(&mut control, None);
    assert_eq!(graph_state(&core), graph_state(&control));
    assert_eq!(products(&mut core), products(&mut control));
    assert_ring_sound(&mut core);
}

// The replica's follow is compared with the live graph at each step in debug builds: a ring on
// empty ground, a conversion, their undos and a bulldoze.
#[test]
fn the_road_tool_replica_follows_ring_edits_and_undo() {
    use crate::nodes::sim::core::road_preview::road_tool_snapshot_from_core;
    use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
    let (mut core, node) = crossing(1);
    core.benchmark_mode = false;
    core.refresh_road_terrain_inputs();
    let mut context =
        RoadPreviewContext::capture(road_tool_snapshot_from_core(&core).expect("current"));
    let first = core.publish_road_tool_view().expect("current roadbed");
    assert!(context.apply(&first));
    let follow = |core: &mut SimCore, context: &mut RoadPreviewContext, step: &str| {
        let publication = core.finalize_network_render_products().expect(step);
        assert!(context.apply(&publication), "{step}");
    };
    assert!(core.place_roundabout_internal(ground(Vector3::new(300.0, 0.0, 300.0), 1, 36.0)));
    follow(&mut core, &mut context, "ground");
    assert!(core.place_roundabout_internal(junction(node, 36.0)));
    follow(&mut core, &mut context, "conversion");
    let ring =
        core.region_graph.edges().iter().position(|edge| {
            edge.roundabout.is_some_and(|ring| ring.centre.x == 0.0) && !edge.deleted
        });
    assert!(core.bulldoze_road_edge_for_test(ring.unwrap()));
    follow(&mut core, &mut context, "bulldoze");
    for step in ["undo bulldoze", "undo conversion", "undo ground"] {
        assert!(core.undo_action_internal());
        follow(&mut core, &mut context, step);
    }
}

// A large ring with more edges than the stroke's excerpt reaches by incidence: the join walks the
// whole ring all the same, here 7 edges, and takes the bare node it lands near.
#[test]
fn a_join_walks_a_ring_beyond_the_excerpts_reach() {
    let (mut core, radius_m) = bare(80.0, 2);
    for angle in [30.0, 150.0, 270.0] {
        draw(&mut core, stroke_at(radius_m, angle), 1).unwrap();
    }
    draw(&mut core, stroke_at(radius_m, 180.0), 1).unwrap();
    let edges = ring_edges(&core).len();
    assert_eq!(edges, 7, "three arms and three bare nodes, then a split");
    draw(&mut core, stroke_at(radius_m, 82.0), 1).unwrap();
    assert_eq!(
        ring_edges(&core).len(),
        edges,
        "the bare node at 90 degrees moved"
    );
    assert_ring_sound(&mut core);
}

// A caller without a plan near a ring plans all the same: only a plan moves a bare node.
#[test]
fn a_plan_less_stroke_near_a_ring_takes_the_bare_node_too() {
    let (mut core, radius_m) = bare(36.0, 1);
    core.precompute_road_mesh_data();
    assert!(
        core.add_road_internal(stroke_at(radius_m, 0.0), 1, 1)
            .committed
    );
    assert_eq!(ring_edges(&core).len(), 3);
}

// The plan's R7b row: a stroke joined to a bare ring in the middle of a chord lays out at 60 m,
// and two of them, either side of a bare node, leave it where it is.
#[test]
fn a_mid_chord_join_lays_out_at_60_m() {
    let (mut core, radius_m) = bare(60.0, 1);
    draw(&mut core, stroke_at(radius_m, 30.0), 1).unwrap();
    assert_eq!(
        ring_edges(&core).len(),
        4,
        "far from both bare nodes: a split"
    );
    assert_ring_sound(&mut core);
}
