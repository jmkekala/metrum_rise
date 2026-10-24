// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68-E` acceptance on the road side: a current preview's pieces are the pieces the commit
//! installs (the same `Arc`s), a stale preview's commit equals a fresh commit bitwise, undo swaps
//! the pre-edit pieces back, and the road tool's replica follows every publication.

use super::road_edit_order::{ARM_M, Stroke, committed};
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

// A lane's planar shape: its XZ bits. Heights differ by design between lanes the renderer
// already lifted onto the surface and freshly built ones.
fn planar_bits(geometry: &[Vector3]) -> Vec<[u32; 2]> {
    geometry
        .iter()
        .map(|p| [p.x.to_bits(), p.z.to_bits()])
        .collect()
}

// The lane network independent of lane ids: every live edge's lanes by (edge, direction,
// index), and every live node's connection lanes, each with its successors named by their
// shapes.
fn lane_network(
    lanes: &crate::simulation::network::lanes::LaneSystem,
    graph: &crate::simulation::network::graph::RegionGraph,
) -> BTreeMap<String, Vec<String>> {
    let shape = |id: usize| format!("{:?}", planar_bits(&lanes.lanes[id].geometry));
    let describe = |id: usize| {
        let lane = &lanes.lanes[id];
        let mut next: Vec<_> = lane.next_lanes.iter().map(|&next| shape(next)).collect();
        next.sort();
        format!("{:?} {} -> {next:?}", lane.lane_type, shape(id))
    };
    let mut network = BTreeMap::new();
    for (&edge, ids) in &lanes.edge_lanes {
        if edge >= graph.edge_count() || graph.edge(edge).deleted {
            continue;
        }
        for &id in ids {
            let lane = &lanes.lanes[id];
            network.insert(
                format!("edge {edge} {} {}", lane.is_fwd, lane.lane_idx),
                vec![describe(id)],
            );
        }
    }
    for (&node, ids) in &lanes.node_lanes {
        let mut connections: Vec<_> = ids.iter().map(|&id| describe(id)).collect();
        connections.sort();
        if !connections.is_empty() {
            network.insert(format!("node {node}"), connections);
        }
    }
    network
}

#[test]
fn undo_rebuilds_lanes_locally_as_a_full_rebuild_would() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        core.benchmark_mode = false;
        let points = stroke(&core, side);
        commit(&mut core, &points, None);
        let lanes_before_undo = core.transit_network.lane_system.lanes.len();

        assert!(core.undo_action_internal());
        let lanes = &core.transit_network.lane_system;
        assert!(
            lanes.lanes.len() > lanes_before_undo,
            "sloped {sloped}: undo appends the closure's lanes instead of renumbering all"
        );
        let mut graph = core.region_graph.clone();
        let mut full = crate::simulation::network::lanes::LaneSystem::new();
        full.rebuild(&mut graph);
        assert_eq!(
            lane_network(lanes, &core.region_graph),
            lane_network(&full, &graph),
            "sloped {sloped}: the local undo's lanes equal a full rebuild's"
        );
    }
}

#[test]
fn preview_chunks_are_the_committed_chunks_without_lane_layers() {
    // A road apart from the through road: its closure has no crossing and no sign, so the
    // lane-free preview chunks equal the commit's publication; the through road's published
    // triangles (with their crossings) are spliced in unchanged.
    let apart = [Vector2::new(-ARM_M, 120.0), Vector2::new(ARM_M, 120.0)];
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        let points = stroke(&core, apart);
        let mut context = context(&core);
        let preview = Arc::new(preview(&mut context, &points));
        let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
        let chunks = pieces.chunks.as_ref().expect("the view's chunks splice");
        assert!(!pieces.changed_chunks.is_empty());
        commit(&mut core, &points, Some(Arc::clone(&preview)));
        for chunk in &pieces.changed_chunks {
            assert_eq!(
                chunks.get(chunk).map(|mesh| format!("{mesh:?}")),
                core.published_road_mesh_chunks
                    .get(chunk)
                    .map(|mesh| format!("{mesh:?}")),
                "sloped {sloped}: chunk {chunk:?} is published as previewed"
            );
        }
    }
}

#[test]
fn save_and_load_reproduce_the_pieces_bitwise() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        let points = stroke(&core, side);
        let mut context = context(&core);
        let previewed = Arc::new(preview(&mut context, &points));
        commit(&mut core, &points, Some(previewed));
        let before = pieces(&core);
        let path = temp_save_path("road_preview_pieces");
        core.save_game_settled(path.to_str().unwrap(), None)
            .expect("the fixture saves");
        let mut loaded = test_core();
        let loaded_result = loaded.load_game_internal(path.to_str().unwrap());
        std::fs::remove_file(&path).unwrap();
        loaded_result.expect("the fixture loads");
        assert!(loaded.transit_network.roadbed.is_current());
        assert_eq!(
            loaded.transit_network.roadbed.failed_nodes(),
            core.transit_network.roadbed.failed_nodes()
        );
        assert_eq!(
            bits_only(&pieces(&loaded)),
            bits_only(&before),
            "sloped {sloped}: a loaded city's pieces equal the saved city's bit for bit"
        );
    }
}

// Native per-stage preview timings on the `ROAD-30` preview fixtures (`docs/roads.md` "Stage
// budgets"): `sloped_t` sweeps a T onto a 220 m trunk, `sloped_branch` orbits a fourth leg of
// the T at (-65, 0). Eight poses each, as `road_preview_replay.gd` moves them; stages are
// medians and p95 over the measured repetitions. The context refresh after a commit (the
// publication and the replica's records) and the commit's graph copy are measured apart.
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_road_preview_stages -- --ignored --nocapture"]
fn benchmark_road_preview_stages() {
    use std::time::Instant;
    let main = ([Vector2::new(-110.0, 0.0), Vector2::new(110.0, 0.0)], 1);
    let leg = ([Vector2::new(-65.0, 0.0), Vector2::new(-65.0, 60.0)], 1);
    let sweep = |u: f32| Vector2::new(-40.0, 0.0).lerp(Vector2::new(90.0, 0.0), u);
    let orbit = |u: f32| Vector2::new(-60.0, -55.0) + Vector2::from_angle(std::f32::consts::TAU * u) * 15.0;
    // Name, committed roads, stroke start and the end pose at u in [0, 1).
    type Case<'a> = (&'a str, Vec<Stroke>, Vector2, &'a dyn Fn(f32) -> Vector2);
    let cases: [Case; 2] = [
        ("sloped_t", vec![main], Vector2::new(0.0, -80.0), &sweep),
        ("sloped_branch", vec![main, leg], Vector2::new(-65.0, 0.0), &orbit),
    ];
    let percentile = |values: &mut Vec<f64>, q: f64| {
        values.sort_by(f64::total_cmp);
        values[((values.len() - 1) as f64 * q).round() as usize]
    };
    for (name, roads, start, pose) in cases {
        let order: Vec<usize> = (0..roads.len()).collect();
        let mut core = committed(&roads, &order, true);
        let mut context = context(&core);
        // Follow the publisher, so the commit below refreshes the context by records.
        let first = core.publish_road_tool_view(None).expect("current roadbed");
        assert!(context.apply(&first));
        let strokes: Vec<Vec<Vector3>> = (0..8)
            .map(|index| stroke(&core, [start, pose(index as f32 / 8.0)]))
            .collect();
        let mut stages = [(); 4].map(|()| Vec::new());
        for repetition in 0..60 {
            for points in &strokes {
                let start = Instant::now();
                let preview = preview(&mut context, points);
                let total_ms = start.elapsed().as_secs_f64() * 1000.0;
                assert!(
                    preview.pieces.as_ref().is_some_and(|pieces| pieces.chunks.is_some()),
                    "{name}: every pose previews its chunks: {}",
                    preview.validation.invalid_reason
                );
                if repetition >= 10 {
                    let times = preview.times;
                    for (samples, ms) in stages
                        .iter_mut()
                        .zip([times.plan_ms, times.roadbed_ms, times.payload_ms, total_ms])
                    {
                        samples.push(ms);
                    }
                }
            }
        }
        let [plan, roadbed, payload, total] = stages.map(|mut samples| {
            (percentile(&mut samples, 0.5), percentile(&mut samples, 0.95))
        });
        // One commit of the first pose: its graph copy and the context refresh it publishes.
        core.benchmark_mode = false;
        let copy_start = Instant::now();
        let copy = std::hint::black_box(core.region_graph.clone());
        let graph_copy_ms = copy_start.elapsed().as_secs_f64() * 1000.0;
        drop(copy);
        let previewed = Arc::new(preview(&mut context, &strokes[0]));
        let outcome = commit(&mut core, &strokes[0], Some(previewed));
        let publication = outcome.road_snapshot.expect("a commit publishes");
        let refresh_start = Instant::now();
        assert!(context.apply(&publication));
        let apply_ms = refresh_start.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "road_preview_stages fixture={name} workers={} poses=8 samples={} plan_ms_p50={:.3} plan_ms_p95={:.3} roadbed_ms_p50={:.3} roadbed_ms_p95={:.3} payload_ms_p50={:.3} payload_ms_p95={:.3} total_ms_p50={:.3} total_ms_p95={:.3} commit_publication_ms={:.3} context_apply_ms={:.3} graph_copy_ms={:.3} edges={}",
            crate::thread_pools::install(rayon::current_num_threads),
            8 * 50,
            plan.0,
            plan.1,
            roadbed.0,
            roadbed.1,
            payload.0,
            payload.1,
            total.0,
            total.1,
            outcome.snapshot_ms,
            apply_ms,
            graph_copy_ms,
            core.region_graph.edge_count(),
        );
    }
}
