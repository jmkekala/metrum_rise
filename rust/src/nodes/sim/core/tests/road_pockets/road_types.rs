// SPDX-License-Identifier: GPL-3.0-or-later

//! Road-type pockets: stroke ownership (under `dev`'s joins and growths, `ROAD-118`), refusal
//! independence, hover against commit, undo and its refund (`ROAD-128`).

use super::*;
use crate::nodes::sim::core::road_commit::{RoadCommitOutcome, RoadCommitRequest};
use crate::simulation::network::road_types::{RoadAttributes, RoadTypePockets};
use std::time::Instant;

fn draw(core: &mut SimCore, points: &[(f32, f32)], pockets: RoadTypePockets) -> RoadCommitOutcome {
    core.commit_road_with_pockets(
        RoadCommitRequest {
            points: points
                .iter()
                .map(|&(x, z)| Vector3::new(x, 0.0, z))
                .collect(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking: Default::default(),
            attributes: RoadAttributes::DEFAULT,
            preview: None,
        },
        pockets,
        Instant::now(),
        Default::default(),
    )
}

fn pockets(left: bool, right: bool, body_m: f32) -> RoadTypePockets {
    RoadTypePockets {
        left,
        right,
        body_m,
    }
}

#[test]
fn crossing_gives_only_the_new_road_two_pockets_and_undo_restores_it() {
    let mut core = super::super::road_edit_order::committed(&[], &[], false);
    core.benchmark_mode = false;
    assert!(
        draw(
            &mut core,
            &[(-180.0, 0.0), (180.0, 0.0)],
            Default::default()
        )
        .committed
    );
    core.adopt_pending_cell_lots();
    let before = products(&mut core);
    // Every node and edge slot bit for bit, holes included (`ROAD-124`): the stroke's and the
    // pockets' records fold into one undo, which must put back what both changed.
    let record = network_record(&core);
    let depth = core.undo_stack.len();
    let outcome = draw(
        &mut core,
        &[(0.0, -180.0), (0.0, 180.0)],
        pockets(true, true, 60.0),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    assert!(
        outcome.pocket_refusals.is_empty(),
        "{:?}",
        outcome
            .pocket_refusals
            .iter()
            .map(|r| &r.reason)
            .collect::<Vec<_>>()
    );
    let pocket_edges: Vec<_> = core
        .region_graph
        .edges()
        .iter()
        .filter(|e| !e.deleted && e.pocket.is_some())
        .collect();
    assert_eq!(pocket_edges.len(), 2);
    assert!(
        pocket_edges
            .iter()
            .all(|e| e.geometry.iter().all(|p| p.x == 0.0))
    );
    assert_eq!(core.undo_stack.len(), depth + 1);
    // The staged terrain belongs to the final pocketed stroke, not the intermediate base road.
    let stage = core.last_graded_tiles.as_ref().expect("graded stroke");
    assert!(!stage.tiles().is_empty());
    for tile in stage.tiles() {
        let mut fresh = Vec::new();
        crate::simulation::terrain::graded::bake_tiles(
            &core.heightmap,
            &core.transit_network.roadbed,
            &core.allocator,
            &[tile.tile],
            tile.level,
            &mut fresh,
        )
        .unwrap();
        assert_eq!(tile.payload.texels, fresh[0].texels);
        assert_eq!(tile.payload.seams, fresh[0].seams);
        assert_eq!(tile.payload.edges, fresh[0].edges);
    }
    assert!(core.undo_action_internal());
    core.precompute_road_mesh_data();
    assert_eq!(products(&mut core), before);
    assert_same_record(&network_record(&core), &record, "undo of the pocketed stroke");
}

// B1 of the icon UI merge: each pocket charges its own record, which the commit folds into the
// stroke's; the fold keeps those charges, so undo refunds the road and its pockets and the
// treasury reads as before the stroke.
#[test]
fn undo_refunds_a_pocketed_strokes_whole_cost() {
    let mut core = super::super::road_edit_order::committed(&[], &[], false);
    core.benchmark_mode = false;
    assert!(draw(&mut core, &[(-180.0, 0.0), (180.0, 0.0)], Default::default()).committed);
    core.adopt_pending_cell_lots();
    let balance = core.treasury.balance;
    let outcome = draw(
        &mut core,
        &[(0.0, -180.0), (0.0, 180.0)],
        pockets(true, true, 60.0),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    assert!(outcome.pocket_refusals.is_empty());
    let charged = core.undo_stack.back().expect("the stroke's record").charged;
    assert!(
        (balance - core.treasury.balance - charged).abs() < 1.0e-6,
        "the record holds all that was charged: {charged} of {}",
        balance - core.treasury.balance
    );
    assert!(core.undo_action_internal());
    assert!(
        (core.treasury.balance - balance).abs() < 1.0e-6,
        "{} after undo against {balance}",
        core.treasury.balance
    );
}

// Previews `points` with `setting` on the road tool's context of `core` (the hover), commits the
// same stroke and holds the hover to the commit: the replica comes back as it was and every
// piece the hover compiled is the committed piece bit for bit. Returns the commit's outcome.
fn hover_then_commit(
    core: &mut SimCore,
    points: &[(f32, f32)],
    setting: RoadTypePockets,
    label: &str,
) -> RoadCommitOutcome {
    use crate::nodes::sim::core::{RoadPreviewContext, RoadPreviewRequest};
    core.adopt_pending_cell_lots();
    core.refresh_road_terrain_inputs();
    let mut context = RoadPreviewContext::capture(road_tool_snapshot_from_core(core).unwrap());
    let counts = (context.replica.node_count(), context.replica.edge_count());
    let preview = context.preview_road_with_hue(
        RoadPreviewRequest {
            surface_generation: context.view.surface_generation,
            points: points
                .iter()
                .map(|&(x, z)| Vector3::new(x, 0.0, z))
                .collect(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking: Default::default(),
            attributes: RoadAttributes::DEFAULT,
        },
        setting,
        None,
    );
    assert!(
        preview.validation.is_valid,
        "{label}: {}",
        preview.validation.invalid_reason
    );
    assert_eq!(
        (context.replica.node_count(), context.replica.edge_count()),
        counts,
        "{label}: the replica is restored"
    );
    assert!(
        context.replica.edges().iter().all(|edge| edge.pocket.is_none()),
        "{label}: no pocket stays on the replica"
    );
    let outcome = draw(core, points, setting);
    assert!(outcome.committed, "{label}: {}", outcome.rejection);
    let pieces = preview.pieces.expect("a valid stroke compiles");
    for (id, expected) in pieces.delta.pieces() {
        let actual = core.transit_network.roadbed.piece_mesh(id);
        assert_eq!(expected.is_some(), actual.is_some(), "{label}: {id:?}");
        if let (Some(expected), Some(actual)) = (expected, actual) {
            let bits = |mesh: &crate::simulation::network::roadbed::PieceMesh| {
                (
                    mesh.positions()
                        .iter()
                        .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
                        .collect::<Vec<_>>(),
                    mesh.triangles().to_vec(),
                )
            };
            assert_eq!(bits(expected), bits(actual), "{label}: {id:?}");
        }
    }
    outcome
}

// The live pocket edges' centrelines.
fn pocket_lines(core: &SimCore) -> Vec<Vec<Vector3>> {
    core.region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.pocket.is_some())
        .map(|edge| edge.geometry.clone())
        .collect()
}

// The refusals' reasons, for messages.
fn reasons(outcome: &RoadCommitOutcome) -> Vec<&str> {
    outcome
        .pocket_refusals
        .iter()
        .map(|refusal| refusal.reason.as_str())
        .collect()
}

// B3 of the icon UI merge on `dev`'s join model (`ROAD-118` round JS2), decision D2: a stroke
// from a road's dead end across a road 10 m on. The 10 m piece cannot lay out beside the
// crossroads, so the joint dissolves and the old road keeps its id and runs on to the crossroads:
// that part of the stroke is now the old road's and asks for no pocket. The stroke's own rest,
// from the crossroads on, takes its pockets there. The hover is the commit, and undo restores the
// network and the roadbed bit for bit.
#[test]
fn a_stroke_merged_into_a_dead_end_pockets_only_its_own_rest() {
    let mut core = super::super::road_edit_order::committed(&[], &[], false);
    core.benchmark_mode = false;
    for road in [[(-180.0, 0.0), (0.0, 0.0)], [(10.0, -180.0), (10.0, 180.0)]] {
        assert!(draw(&mut core, &road, Default::default()).committed);
    }
    core.adopt_pending_cell_lots();
    let before = (network_record(&core), products(&mut core));
    let street = core
        .region_graph
        .edges()
        .iter()
        .position(|edge| !edge.deleted && edge.geometry.iter().any(|p| p.x < -100.0))
        .expect("the street");
    let outcome = hover_then_commit(
        &mut core,
        &[(0.0, 0.0), (180.0, 0.0)],
        pockets(true, true, 60.0),
        "dead end",
    );
    assert!(outcome.pocket_refusals.is_empty(), "{:?}", reasons(&outcome));
    // The joint dissolved: the street keeps its id and ends at the crossroads.
    let graph = &core.region_graph;
    let edge = graph.edge(street);
    assert!(!edge.deleted);
    let crossroads = [edge.start_node, edge.end_node]
        .into_iter()
        .find(|&node| (graph.node(node).pos.x - 10.0).abs() < 0.01)
        .expect("the street runs on to the crossroads");
    let arms = graph.node_adjacency(crossroads).iter();
    assert_eq!(arms.filter(|&&arm| !graph.edge(arm).deleted).count(), 4, "a crossroads");
    let lines = pocket_lines(&core);
    assert_eq!(lines.len(), 1, "one pocket, on the stroke's own approach");
    assert!(
        lines[0].iter().all(|p| p.z == 0.0 && p.x >= 10.0),
        "east of the crossroads: {:?}",
        lines[0]
    );
    assert!(core.undo_action_internal());
    let now = (network_record(&core), products(&mut core));
    assert_same_record(&now.0, &before.0, "dead end undo");
    assert_eq!(now.1, before.1, "dead end undo");
}

// B3 on round JS5c's growths: a stroke whose end makes a corner with a road end grows that road
// onto the corner, a bend that asks for no pocket; a stroke passing 6 m beyond a road end grows
// the road onto the stroke, a T whose two stroke approaches are asked for a left pocket (one has
// the left turn into the street and takes it, the other has none, `pocket_movement_missing`)
// and whose grown road, the old road's, is not asked. The hover is the commit, and undo restores
// the network bit for bit.
#[test]
fn grown_road_ends_take_no_pockets_and_the_stroke_keeps_its_own() {
    let (dx, dz) = (134f32.to_radians().cos(), 134f32.to_radians().sin());
    let (end_x, end_z) = (8.345, 0.0);
    for (label, stroke, expected) in [
        ("corner", [(end_x - dx * 80.0, end_z - dz * 80.0), (end_x, end_z)], (0, 0)),
        ("pass", [(6.0, -180.0), (6.0, 180.0)], (1, 1)),
    ] {
        let mut core = super::super::road_edit_order::committed(&[], &[], false);
        core.benchmark_mode = false;
        assert!(draw(&mut core, &[(-180.0, 0.0), (0.0, 0.0)], Default::default()).committed);
        core.adopt_pending_cell_lots();
        let before = (network_record(&core), products(&mut core));
        let outcome = hover_then_commit(&mut core, &stroke, pockets(true, false, 60.0), label);
        let graph = &core.region_graph;
        assert!(
            graph.edge(0).geometry.iter().any(|p| p.x > 1.0),
            "{label}: the street grew: {:?}",
            graph.edge(0).geometry
        );
        let lines = pocket_lines(&core);
        assert_eq!(
            (lines.len(), outcome.pocket_refusals.len()),
            expected,
            "{label}: {:?}",
            reasons(&outcome)
        );
        assert!(
            lines.iter().flatten().all(|p| p.x == 6.0),
            "{label}: on the stroke: {lines:?}"
        );
        for refusal in &outcome.pocket_refusals {
            assert_eq!(refusal.reason, "pocket_movement_missing", "{label}");
            assert!(
                (refusal.position.x - 6.0).abs() < 0.01 && refusal.position.z.abs() < 0.01,
                "{label}: at the T: {:?}",
                refusal.position
            );
            let edge = graph.edge(refusal.edge);
            assert!(
                edge.geometry.iter().all(|p| p.x == 6.0),
                "{label}: a stroke approach: {:?}",
                edge.geometry
            );
        }
        assert!(core.undo_action_internal(), "{label}");
        let now = (network_record(&core), products(&mut core));
        assert_same_record(&now.0, &before.0, label);
        assert_eq!(now.1, before.1, "{label}");
    }
}

// B3: a stroke crossing two roads has four approaches of its own, two at each crossroads, and
// takes a pocket on each; the hover is the commit, and undo restores the network bit for bit.
#[test]
fn a_stroke_crossing_two_roads_pockets_its_four_approaches() {
    let mut core = super::super::road_edit_order::committed(&[], &[], false);
    core.benchmark_mode = false;
    for z in [-120.0, 120.0] {
        assert!(draw(&mut core, &[(-200.0, z), (200.0, z)], Default::default()).committed);
    }
    core.adopt_pending_cell_lots();
    let before = (network_record(&core), products(&mut core));
    let outcome = hover_then_commit(
        &mut core,
        &[(0.0, -240.0), (0.0, 240.0)],
        pockets(true, true, 60.0),
        "two crossings",
    );
    assert!(outcome.pocket_refusals.is_empty(), "{:?}", reasons(&outcome));
    let lines = pocket_lines(&core);
    assert_eq!(lines.len(), 4);
    assert!(lines.iter().flatten().all(|p| p.x == 0.0), "{lines:?}");
    assert!(core.undo_action_internal());
    let now = (network_record(&core), products(&mut core));
    assert_same_record(&now.0, &before.0, "two crossings undo");
    assert_eq!(now.1, before.1, "two crossings undo");
}

#[test]
fn refused_short_pocket_keeps_the_stroke_and_has_a_reason() {
    let mut core = super::super::road_edit_order::committed(&[], &[], false);
    core.benchmark_mode = false;
    assert!(
        draw(
            &mut core,
            &[(-180.0, 0.0), (180.0, 0.0)],
            Default::default()
        )
        .committed
    );
    let outcome = draw(
        &mut core,
        &[(0.0, 0.0), (0.0, 70.0)],
        pockets(true, false, 60.0),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    assert_eq!(outcome.pocket_refusals.len(), 1);
    assert_eq!(outcome.pocket_refusals[0].reason, "pocket_road_too_short");
    assert!(core.region_graph.edges().iter().all(|e| e.pocket.is_none()));
    assert!(core.undo_action_internal());
    assert_eq!(
        core.region_graph
            .edges()
            .iter()
            .filter(|e| !e.deleted)
            .count(),
        1
    );
}

#[test]
fn missing_role_does_not_reject_an_end_join() {
    let mut core = super::super::road_edit_order::committed(&[], &[], false);
    core.benchmark_mode = false;
    assert!(draw(&mut core, &[(-180.0, 0.0), (0.0, 0.0)], Default::default()).committed);
    assert!(draw(&mut core, &[(0.0, 0.0), (0.0, 180.0)], Default::default()).committed);
    let outcome = draw(
        &mut core,
        &[(0.0, -180.0), (0.0, 0.0)],
        pockets(true, true, 60.0),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    assert_eq!(outcome.pocket_refusals.len(), 1);
    assert_eq!(outcome.pocket_refusals[0].reason, "pocket_movement_missing");
}

#[test]
fn both_junction_ends_of_one_stroke_keep_their_own_pockets() {
    let mut core = super::super::road_edit_order::committed(&[], &[], false);
    core.benchmark_mode = false;
    for z in [-180.0, 180.0] {
        assert!(draw(&mut core, &[(-180.0, z), (180.0, z)], Default::default()).committed);
    }
    let outcome = draw(
        &mut core,
        &[(0.0, -180.0), (0.0, 180.0)],
        pockets(true, true, 60.0),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    assert!(
        outcome.pocket_refusals.is_empty(),
        "{:?}",
        outcome
            .pocket_refusals
            .iter()
            .map(|r| &r.reason)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        core.region_graph
            .edges()
            .iter()
            .filter(|e| !e.deleted && e.pocket.is_some())
            .count(),
        2
    );
}

#[test]
fn pocket_hover_matches_committed_piece_bits_and_restores_its_replica() {
    use crate::nodes::sim::core::{RoadPreviewContext, RoadPreviewRequest};
    let mut core = super::super::road_edit_order::committed(&[], &[], false);
    core.benchmark_mode = false;
    assert!(
        draw(
            &mut core,
            &[(-180.0, 0.0), (180.0, 0.0)],
            Default::default()
        )
        .committed
    );
    core.adopt_pending_cell_lots();
    core.refresh_road_terrain_inputs();
    let mut context = RoadPreviewContext::capture(road_tool_snapshot_from_core(&core).unwrap());
    let counts = (context.replica.node_count(), context.replica.edge_count());
    let preview = context.preview_road_with_hue(
        RoadPreviewRequest {
            surface_generation: context.view.surface_generation,
            points: vec![
                Vector3::new(0.0, 0.0, -180.0),
                Vector3::new(0.0, 0.0, 180.0),
            ],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking: Default::default(),
            attributes: RoadAttributes::DEFAULT,
        },
        pockets(true, true, 60.0),
        None,
    );
    assert!(preview.validation.is_valid);
    assert_eq!(
        (context.replica.node_count(), context.replica.edge_count()),
        counts
    );
    assert!(
        context
            .replica
            .edges()
            .iter()
            .all(|edge| edge.pocket.is_none())
    );
    let outcome = draw(
        &mut core,
        &[(0.0, -180.0), (0.0, 180.0)],
        pockets(true, true, 60.0),
    );
    assert!(outcome.committed);
    let pieces = preview.pieces.unwrap();
    for (id, expected) in pieces.delta.pieces() {
        let actual = core.transit_network.roadbed.piece_mesh(id);
        assert_eq!(expected.is_some(), actual.is_some(), "{id:?}");
        if let (Some(expected), Some(actual)) = (expected, actual) {
            let bits = |mesh: &crate::simulation::network::roadbed::PieceMesh| {
                (
                    mesh.positions()
                        .iter()
                        .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
                        .collect::<Vec<_>>(),
                    mesh.triangles().to_vec(),
                )
            };
            assert_eq!(bits(expected), bits(actual), "{id:?}");
        }
    }
}
