// SPDX-License-Identifier: GPL-2.0-only

//! Road-type pockets: stroke ownership, refusal independence and undo (`ROAD-119`).

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
