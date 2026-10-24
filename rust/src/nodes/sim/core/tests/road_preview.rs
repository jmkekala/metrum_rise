// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68-E` acceptance on the road side: a current preview's pieces are the pieces the commit
//! installs (the same `Arc`s), a stale preview's commit equals a fresh commit bitwise, undo swaps
//! the pre-edit pieces back, and the road tool's replica follows every publication.

use super::road_edit_order::{ARM_M, committed};
use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::nodes::sim::core::road_preview::{RoadPreview, RoadPreviewRequest};
use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
use crate::simulation::network::roadbed::{PieceId, PieceMesh};
use godot::prelude::Vector2;
use std::collections::BTreeMap;
use std::sync::Arc;

const THROUGH: ([Vector2; 2], i32) = ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1);

// A piece's vertex position bits and triangles, in mesh order.
type MeshBits = (Vec<[u64; 3]>, Vec<[u32; 3]>);

fn mesh_bits(mesh: &PieceMesh) -> MeshBits {
    let positions = mesh
        .positions()
        .iter()
        .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
        .collect();
    (positions, mesh.triangles().to_vec())
}

fn piece_mesh(core: &SimCore, id: PieceId) -> Option<&PieceMesh> {
    let roadbed = &core.transit_network.roadbed;
    match id {
        PieceId::Span(edge) => roadbed.span_piece(edge).map(|span| span.mesh()),
        PieceId::Node(node) => roadbed.node_piece(node).map(|piece| piece.mesh()),
    }
}

// Every published piece, with its mesh address and bits.
fn pieces(core: &SimCore) -> BTreeMap<PieceId, (*const PieceMesh, MeshBits)> {
    let everywhere = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
    core.transit_network
        .roadbed
        .pieces_in_bounds(everywhere)
        .map(|(id, _)| {
            let mesh = piece_mesh(core, id).expect("listed pieces are published");
            (id, (std::ptr::from_ref(mesh), mesh_bits(mesh)))
        })
        .collect()
}

fn bits_only(
    pieces: &BTreeMap<PieceId, (*const PieceMesh, MeshBits)>,
) -> BTreeMap<PieceId, &MeshBits> {
    pieces.iter().map(|(id, (_, bits))| (*id, bits)).collect()
}

fn stroke(core: &SimCore, ends: [Vector2; 2]) -> Vec<Vector3> {
    ends.map(|point| {
        let height = core.get_world_surface_height_internal(point);
        Vector3::new(point.x, height, point.y)
    })
    .to_vec()
}

fn context(core: &SimCore) -> RoadPreviewContext {
    RoadPreviewContext::capture(road_tool_snapshot_from_core(core).expect("current roadbed"))
}

fn preview(context: &mut RoadPreviewContext, points: &[Vector3]) -> RoadPreview {
    let surface_generation = context.view.surface_generation;
    context.preview_road(RoadPreviewRequest {
        include_terrain: true,
        request_id: 0,
        surface_generation,
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
    })
}

fn commit(
    core: &mut SimCore,
    points: &[Vector3],
    preview: Option<Arc<RoadPreview>>,
) -> crate::nodes::sim::core::road_commit::RoadCommitOutcome {
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    outcome
}

#[test]
fn a_current_preview_is_what_the_commit_installs() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        let mut control = committed(&[THROUGH], &[0], sloped);
        let points = stroke(&core, side);
        let mut context = context(&core);
        let preview = Arc::new(preview(&mut context, &points));
        assert!(
            preview.validation.is_valid,
            "{}",
            preview.validation.invalid_reason
        );
        let compiled = preview
            .pieces
            .as_ref()
            .expect("a valid stroke compiles pieces");
        assert!(compiled.delta.pieces().count() > 0);

        commit(&mut core, &points, Some(Arc::clone(&preview)));
        assert!(
            core.last_road_edit_metrics.preview_plan_reused,
            "sloped {sloped}: a current preview is the commit's plan"
        );
        for (id, mesh) in compiled.delta.pieces() {
            let live = piece_mesh(&core, id);
            assert_eq!(
                live.map(std::ptr::from_ref),
                mesh.map(std::ptr::from_ref),
                "sloped {sloped}: {id:?} is the preview's own piece"
            );
        }
        assert_eq!(
            core.transit_network.roadbed.failed_nodes(),
            compiled.roadbed.failed_nodes()
        );

        commit(&mut control, &points, None);
        assert!(!control.last_road_edit_metrics.preview_plan_reused);
        assert_eq!(
            bits_only(&pieces(&core)),
            bits_only(&pieces(&control)),
            "sloped {sloped}: installed pieces equal a fresh commit's"
        );
    }
}

#[test]
fn a_stale_preview_recompiles_to_a_fresh_commit() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    let elsewhere = ([Vector2::new(-ARM_M, 120.0), Vector2::new(ARM_M, 120.0)], 1);
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        let side_points = stroke(&core, side);
        let mut context = context(&core);
        let stale = Arc::new(preview(&mut context, &side_points));
        let other = stroke(&core, elsewhere.0);
        commit(&mut core, &other, None);
        commit(&mut core, &side_points, Some(stale));
        assert!(
            !core.last_road_edit_metrics.preview_plan_reused,
            "sloped {sloped}: a stale fingerprint recompiles"
        );
        let control = committed(&[THROUGH, elsewhere], &[0, 1], sloped);
        let mut control = control;
        commit(&mut control, &side_points, None);
        assert_eq!(
            bits_only(&pieces(&core)),
            bits_only(&pieces(&control)),
            "sloped {sloped}: the stale commit equals a fresh one"
        );
    }
}

#[test]
fn undo_swaps_the_pre_edit_pieces_back() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        core.benchmark_mode = false;
        let before = pieces(&core);
        let points = stroke(&core, side);
        let mut context = context(&core);
        let preview = Arc::new(preview(&mut context, &points));
        commit(&mut core, &points, Some(preview));
        assert_ne!(bits_only(&pieces(&core)), bits_only(&before));

        assert!(core.undo_action_internal());
        assert!(core.transit_network.roadbed.is_current());
        assert_eq!(
            pieces(&core),
            before,
            "sloped {sloped}: undo restores the pre-edit pieces themselves"
        );
    }
}

#[test]
fn the_replica_follows_commits_and_undo() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    let mut core = committed(&[THROUGH], &[0], true);
    core.benchmark_mode = false;
    let mut context = context(&core);
    // The initial capture was made outside the publisher; its first publication replaces it.
    let first = core.publish_road_tool_view(None).expect("current roadbed");
    assert!(context.apply(&first));
    let points = stroke(&core, side);
    let previewed = Arc::new(preview(&mut context, &points));
    let outcome = commit(&mut core, &points, Some(previewed));
    let publication = outcome.road_snapshot.expect("a commit publishes");
    assert!(matches!(
        publication.replica,
        crate::nodes::sim::core::road_tool_context::ReplicaSync::Records(_)
    ));
    // Debug builds compare the replica with the live graph copy here.
    assert!(context.apply(&publication));
    assert!(
        !context.apply(&publication),
        "an old publication is ignored"
    );

    assert!(core.undo_action_internal());
    core.rebuild_network_surface_terrain_internal();
    core.precompute_road_mesh_data();
    let publication = core
        .publish_road_tool_view(None)
        .expect("undo leaves it current");
    assert!(context.apply(&publication));
    assert_eq!(context.replica.edge_count(), core.region_graph.edge_count());

    // The followed context previews the same stroke again, as it did before the commit.
    let again = preview(&mut context, &points);
    assert!(again.validation.is_valid && again.pieces.is_some());
}
