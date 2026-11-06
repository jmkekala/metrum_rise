// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68-E` acceptance on the road side: a current preview's pieces are the pieces the commit
//! installs (the same `Arc`s), a stale preview's commit equals a fresh commit bitwise, undo swaps
//! the pre-edit pieces back, and the road tool's replica follows every publication.

use super::road_edit_order::{ARM_M, Stroke, committed, committed_on};
use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::nodes::sim::core::road_preview::{RoadPreview, RoadPreviewRequest};
use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
use crate::simulation::network::graph::RoadMedian;
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

// The road tool's context as a publication leaves it: terrain inputs rebuilt first.
fn context(core: &mut SimCore) -> RoadPreviewContext {
    core.refresh_road_terrain_inputs();
    RoadPreviewContext::capture(road_tool_snapshot_from_core(core).expect("current roadbed"))
}

fn preview(context: &mut RoadPreviewContext, points: &[Vector3]) -> RoadPreview {
    preview_with(context, points, None)
}

// A 1 + 1 stroke's preview with `median` (`ROAD-80`).
fn preview_with(
    context: &mut RoadPreviewContext,
    points: &[Vector3],
    median: Option<RoadMedian>,
) -> RoadPreview {
    let surface_generation = context.view.surface_generation;
    context.preview_road(RoadPreviewRequest {
        parking: Default::default(),
        surface_generation,
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        tunnel: false,
        median,
        attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
    })
}

fn commit(
    core: &mut SimCore,
    points: &[Vector3],
    preview: Option<Arc<RoadPreview>>,
) -> crate::nodes::sim::core::road_commit::RoadCommitOutcome {
    commit_with(core, points, None, preview)
}

// A 1 + 1 stroke's commit with `median` (`ROAD-80`).
fn commit_with(
    core: &mut SimCore,
    points: &[Vector3],
    median: Option<RoadMedian>,
    preview: Option<Arc<RoadPreview>>,
) -> crate::nodes::sim::core::road_commit::RoadCommitOutcome {
    let outcome = core.commit_road(
        RoadCommitRequest {
            parking: Default::default(),
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            tunnel: false,
            median,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
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
        let mut context = context(&mut core);
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

// Open question 7: an arm pinned at both ends keeps its first end's mouth knot when the other
// end's solve adds supports. The trunk link east of a side road gains a junction at its far end
// in a preview; the near junction is rebuilt and compiles there, and the commit installs the
// preview's pieces with no failed node, flat and sloped.
#[test]
fn a_far_junction_keeps_the_near_mouth_pinned_on_the_preview_path() {
    let near = ([Vector2::new(-30.0, 0.0), Vector2::new(-30.0, ARM_M)], 1);
    let far = [Vector2::new(20.0, 0.0), Vector2::new(20.0, -ARM_M)];
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH, near], &[0, 1], sloped);
        let graph = &core.region_graph;
        let near_node = (0..graph.node_count() as u32)
            .map(|node| graph.get_valid_node(node))
            .find(|&node| {
                let pos = graph.node(node).pos;
                (pos.x, pos.z) == (-30.0, 0.0)
            })
            .expect("the near junction");
        let points = stroke(&core, far);
        let mut context = context(&mut core);
        let preview = Arc::new(preview(&mut context, &points));
        assert!(
            preview.validation.is_valid,
            "{}",
            preview.validation.invalid_reason
        );
        let compiled = preview.pieces.as_ref().expect("a valid stroke compiles");
        assert_eq!(compiled.delta.failed_nodes().count(), 0, "sloped {sloped}");
        assert!(
            compiled
                .delta
                .pieces()
                .any(|(id, mesh)| id == PieceId::Node(near_node) && mesh.is_some()),
            "sloped {sloped}: the near junction is rebuilt with the link"
        );
        commit(&mut core, &points, Some(Arc::clone(&preview)));
        assert!(core.last_road_edit_metrics.preview_plan_reused);
        assert!(
            core.transit_network.roadbed.failed_nodes().is_empty(),
            "sloped {sloped}: {:?}",
            core.transit_network.roadbed.failed_nodes()
        );
        let installed = piece_mesh(&core, PieceId::Node(near_node)).map(std::ptr::from_ref);
        let previewed = compiled
            .delta
            .pieces()
            .find(|(id, _)| *id == PieceId::Node(near_node))
            .and_then(|(_, mesh)| mesh.map(std::ptr::from_ref));
        assert_eq!(installed, previewed, "sloped {sloped}");
    }
}

#[test]
fn a_stale_preview_recompiles_to_a_fresh_commit() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    let elsewhere = ([Vector2::new(-ARM_M, 120.0), Vector2::new(ARM_M, 120.0)], 1);
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        let side_points = stroke(&core, side);
        let mut context = context(&mut core);
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
        let mut context = context(&mut core);
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
    let mut context = context(&mut core);
    // The initial capture was made outside the publisher; its first publication replaces it.
    let first = core.publish_road_tool_view().expect("current roadbed");
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
        .publish_road_tool_view()
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

// The preview's chunks as Godot composes them over the view's published chunks (each one's hidden
// runs dropped, its overlay merged) against the chunks the commit then published: layers,
// crossing frames and owner runs bitwise (`ROAD-68-H` round 3). The overlays carry no lights or
// signs. Returns whether an overlay paints a crossing.
fn assert_preview_composes_committed(
    core: &SimCore,
    view_chunks: &crate::simulation::network::render::RoadChunkMap,
    pieces: &crate::nodes::sim::core::road_preview::RoadPreviewPieces,
    label: &str,
) -> bool {
    use crate::simulation::network::NetworkMeshData;
    let chunks = pieces.chunks.as_ref().expect("the view's chunks compose");
    assert!(!pieces.changed_chunks.is_empty());
    let mut painted = false;
    for chunk in &pieces.changed_chunks {
        let published = view_chunks.get(chunk).map(|mesh| &**mesh);
        let composed = match chunks.get(chunk) {
            Some(preview) => {
                assert!(
                    preview.overlay.street_lights.is_empty() && preview.overlay.road_signs.is_empty(),
                    "{label}: an overlay places no lights or signs"
                );
                let positions: usize = preview
                    .hidden
                    .iter()
                    .map(|&[_, start, end]| (end - start) as usize)
                    .sum();
                assert_eq!(positions, preview.hidden_positions.len());
                painted |= preview.overlay.road_crossings.iter().any(|paint| paint.z > 0.0);
                Some(NetworkMeshData::compose_preview(
                    published,
                    &preview.overlay,
                    &preview.hidden,
                ))
            }
            None => None,
        };
        let shown = composed.as_ref().or(published);
        match (shown, core.cached_road_mesh_chunks.get(chunk)) {
            (Some(shown), Some(committed)) => assert!(
                shown.same_surfaces(committed),
                "{label}: chunk {chunk:?} composes the committed chunk"
            ),
            (Some(shown), None) => {
                assert!(shown.is_empty(), "{label}: chunk {chunk:?} composes to nothing")
            }
            (None, committed) => assert!(
                committed.is_none_or(|mesh| mesh.is_empty()),
                "{label}: chunk {chunk:?} stays empty"
            ),
        }
    }
    painted
}

#[test]
fn preview_chunks_compose_the_committed_chunks() {
    // A road apart from the through road: its closure has no crossing and no sign; the through
    // road's published runs (with their crossings) stay resident.
    let apart = [Vector2::new(-ARM_M, 120.0), Vector2::new(ARM_M, 120.0)];
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        let points = stroke(&core, apart);
        let mut context = context(&mut core);
        let view_chunks = context.view.road_mesh_chunks.clone();
        let preview = Arc::new(preview(&mut context, &points));
        let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
        commit(&mut core, &points, Some(Arc::clone(&preview)));
        assert_preview_composes_committed(&core, &view_chunks, pieces, &format!("sloped {sloped}"));
    }
}

#[test]
fn a_preview_after_an_acknowledged_publication_composes_over_every_published_chunk() {
    // Godot acknowledges each publication, which empties the upserts it had not yet installed. A
    // view published afterwards still holds every published chunk (`ROAD-68-H` round 3), so a
    // T on the through road composes over that road's published runs. Before, the view held only
    // the unacknowledged upserts, and the preview spliced its pieces into nothing.
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    let mut core = committed(&[THROUGH], &[0], false);
    let generation = core.road_tool_surface_generation;
    assert!(core.acknowledge_network_render_generation(generation));
    assert!(core.published_road_mesh_chunks.is_empty());
    let points = stroke(&core, side);
    let mut context = context(&mut core);
    let view_chunks = context.view.road_mesh_chunks.clone();
    assert!(!view_chunks.is_empty(), "the view holds the acknowledged chunks");
    let preview = Arc::new(preview(&mut context, &points));
    let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
    let chunks = pieces.chunks.as_ref().expect("the view's chunks compose");
    assert!(
        chunks.values().any(|chunk| !chunk.hidden.is_empty()),
        "the T replaces runs of the resident through road"
    );
    commit(&mut core, &points, Some(Arc::clone(&preview)));
    assert_preview_composes_committed(&core, &view_chunks, pieces, "acknowledged");
}

#[test]
fn preview_chunks_with_crossings_and_signs_are_the_committed_chunks() {
    // The T's side road: its closure holds the T's three crossings, their signs and the lights
    // of its arms. A straight continuation of the through road: the pass-through node's
    // span-owned crossing (D8). Paint, lights and signs come from the pieces, so the preview's
    // chunks are published bitwise (`ROAD-68-F`, open question 26).
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    let on = [Vector2::new(ARM_M, 0.0), Vector2::new(2.0 * ARM_M, 0.0)];
    for sloped in [false, true] {
        for (name, ends) in [("t", side), ("continuation", on)] {
            let mut core = committed(&[THROUGH], &[0], sloped);
            let points = stroke(&core, ends);
            let mut context = context(&mut core);
            let view_chunks = context.view.road_mesh_chunks.clone();
            let preview = Arc::new(preview(&mut context, &points));
            let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
            commit(&mut core, &points, Some(Arc::clone(&preview)));
            assert!(core.last_road_edit_metrics.preview_plan_reused);
            let label = format!("{name} sloped {sloped}");
            let painted = assert_preview_composes_committed(&core, &view_chunks, pieces, &label);
            let committed = |kind: fn(&crate::simulation::network::NetworkMeshData) -> bool| {
                pieces.changed_chunks.iter().any(|chunk| {
                    core.cached_road_mesh_chunks.get(chunk).is_some_and(|mesh| kind(mesh))
                })
            };
            let signed = committed(|mesh| !mesh.road_signs.is_empty());
            let lit = committed(|mesh| !mesh.street_lights.is_empty());
            assert!(
                painted && signed && lit,
                "{label}: the preview paints zebras ({painted}); the commit signs ({signed}) and \
                 lights ({lit}) them"
            );
        }
    }
}

// `ROAD-68-F` part 2: a two-lane road end continued straight by a four-lane road is a layout
// with D4 tapers (two profiles share no pass-through mouth row): the stroke's preview, with the
// node's one crossing, publishes its chunks bitwise.
#[test]
fn a_widening_continuation_previews_its_committed_chunks() {
    let on = [Vector2::new(ARM_M, 0.0), Vector2::new(2.0 * ARM_M, 0.0)];
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        let points = stroke(&core, on);
        let mut context = context(&mut core);
        let surface_generation = context.view.surface_generation;
        let preview = Arc::new(context.preview_road(RoadPreviewRequest {
            surface_generation,
            points: points.clone(),
            fwd_lanes: 2,
            bkw_lanes: 2,
            snap_to_existing_roads: true,
            tunnel: false,
            median: None,
            parking: crate::simulation::network::graph::RoadParking::NONE,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        }));
        let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
        let view_chunks = context.view.road_mesh_chunks.clone();
        let outcome = core.commit_road(
            RoadCommitRequest {
                points,
                fwd_lanes: 2,
                bkw_lanes: 2,
                snap_to_existing_roads: true,
                tunnel: false,
                median: None,
                parking: crate::simulation::network::graph::RoadParking::NONE,
                attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
                preview: Some(Arc::clone(&preview)),
            },
            std::time::Instant::now(),
            Default::default(),
        );
        assert!(outcome.committed, "{}", outcome.rejection);
        assert!(core.last_road_edit_metrics.preview_plan_reused);
        let graph = &core.region_graph;
        let node = (0..graph.node_count() as u32)
            .find(|&node| {
                let pos = graph.node(node).pos;
                graph.get_valid_node(node) == node && (pos.x, pos.z) == (ARM_M, 0.0)
            })
            .expect("the joined road end");
        let piece = core.transit_network.roadbed.node_piece(node);
        assert!(
            piece.is_some_and(|piece| piece.layout().boundaries.iter().any(|b| b.taper.is_some())),
            "sloped {sloped}: the widening is a node piece with a D4 taper"
        );
        assert_preview_composes_committed(&core, &view_chunks, pieces, &format!("sloped {sloped}"));
    }
}

#[test]
fn save_and_load_reproduce_the_pieces_bitwise() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        let points = stroke(&core, side);
        let mut context = context(&mut core);
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
// the T at (-65, 0), `divided_t` is `sloped_t` with a 3 m kerbed grass median on the stroke
// (`ROAD-80`). Eight poses each, as `road_preview_replay.gd` moves them; stages are
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
    let grass = RoadMedian::from_tool(2, 3.0).unwrap();
    // Name, committed roads, stroke start, the end pose at u in [0, 1) and the stroke's median.
    type Case<'a> = (
        &'a str,
        Vec<Stroke>,
        Vector2,
        &'a dyn Fn(f32) -> Vector2,
        Option<RoadMedian>,
    );
    let cases: [Case; 3] = [
        ("sloped_t", vec![main], Vector2::new(0.0, -80.0), &sweep, None),
        ("sloped_branch", vec![main, leg], Vector2::new(-65.0, 0.0), &orbit, None),
        ("divided_t", vec![main], Vector2::new(0.0, -80.0), &sweep, grass),
    ];
    let percentile = |values: &mut Vec<f64>, q: f64| {
        values.sort_by(f64::total_cmp);
        values[((values.len() - 1) as f64 * q).round() as usize]
    };
    for (name, roads, start, pose, median) in cases {
        let order: Vec<usize> = (0..roads.len()).collect();
        let mut core = committed(&roads, &order, true);
        let mut context = context(&mut core);
        // Follow the publisher, so the commit below refreshes the context by records.
        let first = core.publish_road_tool_view().expect("current roadbed");
        assert!(context.apply(&first));
        let strokes: Vec<Vec<Vector3>> = (0..8)
            .map(|index| stroke(&core, [start, pose(index as f32 / 8.0)]))
            .collect();
        let mut stages = [(); 5].map(|()| Vec::new());
        let (mut graded_tiles, mut graded_adopted, mut graded_baked) = (0, 0, 0);
        // Payload vertices over the first repetition's poses: the overlays' (all layers and the
        // skirts') and the hidden runs' positions.
        let (mut payload_vertices, mut skirt_vertices, mut hidden_vertices) = (0, 0, 0);
        for repetition in 0..60 {
            for points in &strokes {
                let start = Instant::now();
                let preview = preview_with(&mut context, points, median);
                let total_ms = start.elapsed().as_secs_f64() * 1000.0;
                assert!(
                    preview.pieces.as_ref().is_some_and(|pieces| pieces.chunks.is_some()),
                    "{name}: every pose previews its chunks: {}",
                    preview.validation.invalid_reason
                );
                if repetition == 0 {
                    let chunks = preview
                        .pieces
                        .as_ref()
                        .and_then(|pieces| pieces.chunks.as_ref());
                    for chunk in chunks.into_iter().flat_map(|chunks| chunks.values()) {
                        payload_vertices += chunk.overlay.vertex_count();
                        skirt_vertices += chunk.overlay.skirt_vertices.len();
                        hidden_vertices += chunk.hidden_positions.len();
                    }
                }
                let graded = &preview.pieces.as_ref().unwrap().graded;
                assert!(
                    graded.stage.is_ok(),
                    "{name}: every pose bakes its graded tiles"
                );
                if repetition >= 10 {
                    let times = preview.times;
                    graded_tiles += graded.stats.wanted;
                    graded_adopted += graded.stats.adopted;
                    graded_baked += graded.stats.baked;
                    for (samples, ms) in stages.iter_mut().zip([
                        times.plan_ms,
                        times.roadbed_ms,
                        times.payload_ms,
                        times.graded_ms,
                        total_ms,
                    ]) {
                        samples.push(ms);
                    }
                }
            }
        }
        let [plan, roadbed, payload, graded, total] = stages.map(|mut samples| {
            (percentile(&mut samples, 0.5), percentile(&mut samples, 0.95))
        });
        // One commit of the first pose: its graph copy and the context refresh it publishes.
        core.benchmark_mode = false;
        let copy_start = Instant::now();
        let copy = std::hint::black_box(core.region_graph.clone());
        let graph_copy_ms = copy_start.elapsed().as_secs_f64() * 1000.0;
        drop(copy);
        let previewed = Arc::new(preview_with(&mut context, &strokes[0], median));
        let outcome = commit_with(&mut core, &strokes[0], median, Some(previewed));
        let publication = outcome.road_snapshot.expect("a commit publishes");
        let refresh_start = Instant::now();
        assert!(context.apply(&publication));
        let apply_ms = refresh_start.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "road_preview_stages fixture={name} workers={} poses=8 samples={} payload_vertices_per_pose={} skirt_vertices_per_pose={} hidden_vertices_per_pose={} plan_ms_p50={:.3} plan_ms_p95={:.3} roadbed_ms_p50={:.3} roadbed_ms_p95={:.3} payload_ms_p50={:.3} payload_ms_p95={:.3} graded_ms_p50={:.3} graded_ms_p95={:.3} graded_tiles_per_pose={:.2} graded_adopted_per_pose={:.2} graded_baked_per_pose={:.2} commit_graded_ms={:.3} commit_graded_adopted={} commit_graded_baked={} total_ms_p50={:.3} total_ms_p95={:.3} commit_publication_ms={:.3} context_apply_ms={:.3} graph_copy_ms={:.3} edges={}",
            crate::thread_pools::install(rayon::current_num_threads),
            8 * 50,
            payload_vertices / 8,
            skirt_vertices / 8,
            hidden_vertices / 8,
            plan.0,
            plan.1,
            roadbed.0,
            roadbed.1,
            payload.0,
            payload.1,
            graded.0,
            graded.1,
            graded_tiles as f64 / 400.0,
            graded_adopted as f64 / 400.0,
            graded_baked as f64 / 400.0,
            core.last_road_edit_metrics.surface_graded_ms,
            core.last_road_edit_metrics.graded_adopted,
            core.last_road_edit_metrics.graded_baked,
            total.0,
            total.1,
            outcome.snapshot_ms,
            apply_ms,
            graph_copy_ms,
            core.region_graph.edge_count(),
        );
    }
}

// A T on a straight trunk changes the terrain only around the junction: the split trunk's far
// half keeps its chain segments, so the commit adopts the same graded tiles on a 220 m trunk as
// on a 660 m one, flat and sloped (`ROAD-68-E` and `TERRAIN-07-E` locality).
#[test]
fn a_split_trunk_rebuilds_only_the_terrain_near_the_junction() {
    let side = [Vector2::new(0.0, -80.0), Vector2::new(25.0, 0.0)];
    for sloped in [false, true] {
        let scopes: Vec<_> = [110.0, 330.0]
            .map(|half_m| {
                let trunk = ([Vector2::new(-half_m, 0.0), Vector2::new(half_m, 0.0)], 1);
                let mut core = committed_on(1024.0, &[trunk], &[0], sloped);
                let points = stroke(&core, side);
                let mut context = context(&mut core);
                let mut preview = preview(&mut context, &points);
                // T27 (d): the preview's terrain catches up before the click.
                while let Some(graded) = context.catch_up_graded() {
                    preview.pieces.as_mut().expect("the pose's pieces").graded = graded;
                }
                commit(&mut core, &points, Some(Arc::new(preview)));
                // `TERRAIN-07-E`: the commit adopts the preview's graded tiles, the same tiles
                // on either trunk.
                assert_eq!(
                    core.last_road_edit_metrics.graded_baked, 0,
                    "sloped {sloped}, trunk {half_m}: the commit adopts the previewed tiles"
                );
                let tiles = core.last_graded_tiles.as_ref().expect("the tiles bake");
                let tiles: Vec<_> = tiles
                    .tiles()
                    .iter()
                    .map(|held| (held.tile.x, held.tile.z))
                    .collect();
                tiles
            })
            .into_iter()
            .collect();
        assert!(
            !scopes[0].is_empty(),
            "sloped {sloped}: the T touches terrain"
        );
        assert_eq!(
            scopes[0], scopes[1],
            "sloped {sloped}: the trunk's length does not change the terrain the T rebuilds"
        );
    }
}

// What one road edit's undo record keeps alive (`ROAD-68-E`), measured with heaptrack: with
// `METRUM_UNDO_HEAP=forget` the record of the locality test's T on a sloped trunk of half length
// `METRUM_UNDO_HEAP_TRUNK_M` (default 110 m) is leaked, otherwise dropped. The difference of the
// two runs' leaked totals is the heap the record retains; an upper bound, as it also counts what
// the record shares with the city.
#[test]
#[ignore = "heaptrack measurement: METRUM_UNDO_HEAP=forget|drop heaptrack $TEST_BINARY measure_undo_record_heap --exact --ignored"]
fn measure_undo_record_heap() {
    let half_m = std::env::var("METRUM_UNDO_HEAP_TRUNK_M").map_or(110.0, |value| {
        value.parse().expect("a trunk half length in metres")
    });
    let trunk = ([Vector2::new(-half_m, 0.0), Vector2::new(half_m, 0.0)], 1);
    let mut core = committed_on(1024.0, &[trunk], &[0], true);
    core.benchmark_mode = false;
    core.undo_stack.clear();
    let points = stroke(&core, [Vector2::new(0.0, -80.0), Vector2::new(25.0, 0.0)]);
    let preview = Arc::new(preview(&mut context(&mut core), &points));
    commit(&mut core, &points, Some(preview));
    let record = core
        .undo_stack
        .pop_back()
        .expect("the commit records its undo");
    assert!(
        record.roadbed.is_some() && record.graded.is_some(),
        "the record holds the replaced pieces and graded tiles"
    );
    if std::env::var("METRUM_UNDO_HEAP").as_deref() == Ok("forget") {
        std::mem::forget(record);
    }
}

// Medians drawn with the road tool (`ROAD-80`): a divided stroke's current preview is what its
// commit installs, piece for piece and chunk for chunk, flat and sloped, for every style: a T
// whose side road noses out before its zebra. The committed road carries the median and the
// width it adds; a fresh commit of the same stroke equals it bitwise.
#[test]
fn a_divided_stroke_preview_is_what_the_commit_installs() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    for (style, width_m) in [(1, 1.0), (2, 3.0), (3, 2.0)] {
        let median = RoadMedian::from_tool(style, width_m).unwrap();
        for sloped in [false, true] {
            let label = format!("style {style}, sloped {sloped}");
            let mut core = committed(&[THROUGH], &[0], sloped);
            let mut control = committed(&[THROUGH], &[0], sloped);
            let points = stroke(&core, side);
            let mut context = context(&mut core);
            let view_chunks = context.view.road_mesh_chunks.clone();
            let preview = Arc::new(preview_with(&mut context, &points, median));
            assert!(preview.validation.is_valid, "{label}: {}", preview.validation.invalid_reason);
            let compiled = preview.pieces.as_ref().expect("a valid stroke compiles pieces");
            commit_with(&mut core, &points, median, Some(Arc::clone(&preview)));
            assert!(core.last_road_edit_metrics.preview_plan_reused, "{label}");
            for (id, mesh) in compiled.delta.pieces() {
                assert_eq!(
                    piece_mesh(&core, id).map(std::ptr::from_ref),
                    mesh.map(std::ptr::from_ref),
                    "{label}: {id:?} is the preview's own piece"
                );
            }
            assert_preview_composes_committed(&core, &view_chunks, compiled, &label);
            let divided: Vec<_> = core
                .region_graph
                .edges()
                .iter()
                .filter(|edge| !edge.deleted && edge.median.is_some())
                .collect();
            assert_eq!(divided.len(), 1, "{label}: the side road alone is divided");
            assert_eq!(divided[0].median, median);
            assert_eq!(divided[0].width, 2.0 * crate::config::LANE_WIDTH + width_m);

            commit_with(&mut control, &points, median, None);
            assert_eq!(
                bits_only(&pieces(&core)),
                bits_only(&pieces(&control)),
                "{label}: installed pieces equal a fresh commit's"
            );
        }
    }
}

// The fork's `drawn_medians_survive_widening_and_cross_junction_splits` (`ROAD-80`): a drawn
// divided road keeps its median when it is widened in place and when a crossing road splits
// it, both halves carrying it. A preview made without the median is not the divided stroke's.
#[test]
fn drawn_medians_survive_widening_and_cross_junction_splits() {
    let through = [Vector2::new(-ARM_M, 40.0), Vector2::new(ARM_M, 40.0)];
    let crossing = [Vector2::new(0.0, -20.0), Vector2::new(0.0, 100.0)];
    for (style, width_m) in [(1, 1.0), (2, 5.0), (3, 2.0)] {
        let median = RoadMedian::from_tool(style, width_m).unwrap();
        let mut core = committed(&[], &[], false);
        let points = stroke(&core, through);
        let mut context = context(&mut core);
        let undivided = Arc::new(preview(&mut context, &points));
        commit_with(&mut core, &points, median, Some(undivided));
        assert!(
            !core.last_road_edit_metrics.preview_plan_reused,
            "style {style}: an undivided preview is not the divided stroke's plan"
        );
        let edge = core
            .region_graph
            .edges()
            .iter()
            .position(|edge| !edge.deleted)
            .unwrap();
        assert_eq!(core.region_graph.edge(edge).median, median);
        assert!(
            core.set_edge_cross_section_internal(
                edge as i32,
                2,
                2,
                median,
                Default::default(),
                crate::simulation::network::road_types::RoadAttributes::DEFAULT
            ),
            "style {style}: widened in place"
        );
        // The pieces publish as the Godot call publishes them, before the next stroke plans.
        core.finalize_network_render_products();
        let points = stroke(&core, crossing);
        commit(&mut core, &points, None);
        let live: Vec<_> = core.region_graph.edges().iter().filter(|e| !e.deleted).collect();
        assert_eq!(live.len(), 4, "style {style}: the crossing splits both roads");
        let divided: Vec<_> = live.iter().filter(|edge| edge.median.is_some()).collect();
        assert_eq!(divided.len(), 2, "style {style}: both halves of the divided road");
        for half in divided {
            assert_eq!(half.median, median);
            assert_eq!((half.fwd_lanes, half.bkw_lanes), (2, 2));
            assert_eq!(half.width, 4.0 * crate::config::LANE_WIDTH + width_m);
        }
    }
}

// A stroke's median takes the in-place edit's rules (`ROAD-80`, `ROAD-92`): no median on a
// one-way road or a footpath, in the hover check and the plan alike.
#[test]
fn a_stroke_median_needs_a_road_with_both_directions() {
    let mut core = committed(&[], &[], false);
    let points = stroke(&core, [Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)]);
    let snapshot = road_tool_snapshot_from_core(&core).expect("current roadbed");
    let median = RoadMedian::from_tool(2, 3.0).unwrap();
    for ((fwd, bkw), reason) in [
        ((1, 0), "median_requires_both_directions"),
        ((0, 0), "median_requires_vehicle_road"),
    ] {
        let request = RoadPreviewRequest {
            parking: Default::default(),
            surface_generation: snapshot.surface_generation,
            points: points.clone(),
            fwd_lanes: fwd,
            bkw_lanes: bkw,
            snap_to_existing_roads: true,
            tunnel: false,
            median,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        };
        assert_eq!(snapshot.validate(&request).1.invalid_reason, reason);
        let outcome = core.commit_road(
            RoadCommitRequest {
                parking: Default::default(),
                points: points.clone(),
                fwd_lanes: fwd,
                bkw_lanes: bkw,
                snap_to_existing_roads: true,
                tunnel: false,
                median,
                attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
                preview: None,
            },
            std::time::Instant::now(),
            Default::default(),
        );
        assert_eq!(outcome.rejection_reason, reason);
    }
}

// The grass layer is chosen by the median's style at emission (`ROAD-80`): a kerbed grass
// median's top draws on it and nowhere else, a paved one's on the sidewalk layer, a painted one
// has no band; every chunk without a grass median leaves the layer empty.
#[test]
fn only_a_kerbed_grass_median_draws_on_the_grass_layer() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    for (style, width_m) in [(0, 0.0), (1, 1.0), (2, 3.0), (3, 2.0)] {
        let median = RoadMedian::from_tool(style, width_m).unwrap();
        let mut core = committed(&[THROUGH], &[0], false);
        let sidewalk_before: usize = core
            .cached_road_mesh_chunks
            .values()
            .map(|mesh| mesh.sidewalk_vertices.len())
            .sum();
        let points = stroke(&core, side);
        commit_with(&mut core, &points, median, None);
        let grass: usize = core
            .cached_road_mesh_chunks
            .values()
            .map(|mesh| mesh.median_grass_vertices.len())
            .sum();
        assert_eq!(grass > 0, style == 2, "style {style}: {grass} grass vertices");
        if style == 2 {
            let mut control = committed(&[THROUGH], &[0], false);
            commit_with(&mut control, &points, RoadMedian::from_tool(3, 2.0).unwrap(), None);
            let sidewalk = |core: &SimCore| -> usize {
                core.cached_road_mesh_chunks
                    .values()
                    .map(|mesh| mesh.sidewalk_vertices.len())
                    .sum()
            };
            assert!(
                sidewalk(&control) > sidewalk(&core),
                "paving's top is sidewalk, grass's is not"
            );
            assert!(sidewalk(&core) > sidewalk_before);
        }
    }
}

// A painted median is paint (`ROAD-80`, R3-c): on a 1 + 1 street, whose lanes are unmarked,
// every marking is the median's: within half its width of the line between the lane groups,
// meeting on that line at both noses, reaching the full width between them, with diagonals
// across. On a 2 + 2 road the lane dividers stay outside the hatched gap. Undivided, a 40 km/h
// street has no marking at all; a 50 km/h one (the built-in 2-Lane Road) has its centre line.
#[test]
fn a_painted_median_is_hatched_between_the_lane_groups() {
    use crate::simulation::network::render::road::RoadRenderer;
    let ends = [Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)];
    let marking_at = |median: Option<RoadMedian>, lanes: i32, speed_kmh: f32| {
        let mut core = committed(&[], &[], false);
        let points = stroke(&core, ends);
        let outcome = core.commit_road(
            RoadCommitRequest {
                parking: Default::default(),
                points,
                fwd_lanes: lanes,
                bkw_lanes: lanes,
                snap_to_existing_roads: true,
                tunnel: false,
                median,
                attributes: crate::simulation::network::road_types::RoadAttributes {
                    speed_ms: speed_kmh / 3.6,
                    ..crate::simulation::network::road_types::RoadAttributes::DEFAULT
                },
                preview: None,
            },
            std::time::Instant::now(),
            Default::default(),
        );
        assert!(outcome.committed, "{}", outcome.rejection);
        let mesh = RoadRenderer.generate_mesh_data_with_surface(
            &core.region_graph,
            &core.heightmap,
            &core.transit_network.roadbed,
        );
        mesh.marking_vertices
    };
    let marking = |median: Option<RoadMedian>, lanes: i32| marking_at(median, lanes, 40.0);
    assert!(marking(None, 1).is_empty(), "an undivided 40 km/h 1 + 1 street is unmarked");
    let centre = marking_at(None, 1, 50.0);
    assert!(
        !centre.is_empty() && centre.iter().all(|v| v.z.abs() <= 0.05 + 1.0e-3),
        "a 50 km/h 1 + 1 road has its centre line, and nothing else"
    );
    let painted = RoadMedian::from_tool(1, 2.0).unwrap();
    let half = 1.0;
    // Paint half width plus a line's half width, and float slack.
    let outer = half + 0.05 + 1.0e-3;
    let hatch = marking(painted, 1);
    assert!(!hatch.is_empty(), "the painted median is paint");
    assert!(
        hatch.iter().all(|v| v.z.abs() <= outer),
        "every line lies in the gap"
    );
    let (west, east) = hatch.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| {
        (lo.min(v.x), hi.max(v.x))
    });
    for nose_x in [west, east] {
        assert!(
            hatch
                .iter()
                .filter(|v| (v.x - nose_x).abs() < 1.0e-3)
                .all(|v| v.z.abs() <= 0.05 + 1.0e-3),
            "the edge lines meet on the centre line at the nose at x {nose_x}"
        );
    }
    assert!(
        hatch.iter().any(|v| v.z.abs() >= half - 0.05 - 1.0e-3),
        "the paint reaches the full width"
    );
    // Diagonals: segments whose ends lie on opposite edge lines, more than the two edge lines'
    // two quads per section pair.
    let quads = hatch.len() / 6;
    assert!(
        quads >= 2 * 2 + (2.0 * ARM_M / 3.0 * 0.5) as usize,
        "{quads} paint quads hold the diagonals"
    );
    let wide = marking(painted, 2);
    let dividers: Vec<_> = wide.iter().filter(|v| v.z.abs() > outer).collect();
    assert!(!dividers.is_empty(), "a 2 + 2 road keeps its lane dividers");
    assert!(
        dividers.iter().all(|v| (v.z.abs() - (half + crate::config::LANE_WIDTH)).abs() <= 0.08),
        "the dividers lie a lane out from the gap"
    );
}

// The solid approach zone (`ROAD-94`) and the turn arrows (`ROAD-75`): on a 2 + 2 T every
// approach's two lanes serve different movements, so the divider between them is solid over the
// last `APPROACH_SOLID_LENGTH_M` before the junction and dashed before it; the dividers of the
// lanes leaving the junction stay dashed, and the centre line between the 2 + 2 groups is a solid
// no-crossing line over the whole span (several lanes one way, the user, 2026-11-05). Each approach lane carries arrows for
// its movements, one glyph 12 m back from the mouth. A straight 2 + 2 road has no junction to
// choose a lane for: no solid line, no arrow. The paint reads the graph alone, so the T's 2 + 2
// preview composes its committed chunks, arrows included.
#[test]
fn a_movement_selective_approach_paints_its_dividers_solid_before_the_junction() {
    use crate::config::{DRIVE_ON_LEFT, LANE_WIDTH};
    use crate::simulation::network::lanes::APPROACH_SOLID_LENGTH_M;
    use crate::simulation::network::render::road::RoadRenderer;
    const SOLID: f32 = 2.0;
    let wide = ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 2);
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    // Every marking triangle as (centroid, its station range, its pattern, vertices). A line's
    // quad is two triangles, each spanning the quad's stations.
    let triangles = |core: &SimCore| {
        let mesh = RoadRenderer.generate_mesh_data_with_surface(
            &core.region_graph,
            &core.heightmap,
            &core.transit_network.roadbed,
        );
        mesh.marking_vertices
            .chunks(3)
            .zip(mesh.marking_uvs.chunks(3))
            .map(|(vertices, uvs)| {
                let centroid = vertices.iter().fold(Vector3::ZERO, |sum, &v| sum + v) / 3.0;
                let (low, high) = uvs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), uv| {
                    (lo.min(uv.x), hi.max(uv.x))
                });
                (centroid, high - low, uvs[0].y, vertices.to_vec())
            })
            .collect::<Vec<_>>()
    };
    let straight = triangles(&committed(&[wide], &[0], false));
    assert!(!straight.is_empty(), "a 2 + 2 road is marked");
    // The road runs along x: its centre line lies on z = 0, its lane dividers at +-LANE_WIDTH.
    assert!(
        straight
            .iter()
            .filter(|&&(_, _, pattern, _)| pattern == SOLID)
            .all(|(centroid, ..)| centroid.z.abs() < 0.3),
        "a road without a junction has no solid approach zone"
    );
    assert!(
        straight
            .iter()
            .filter(|(centroid, ..)| centroid.z.abs() < 0.3)
            .all(|&(_, _, pattern, _)| pattern == SOLID),
        "the centre line of a 2 + 2 road is solid"
    );

    let mut core = committed(&[wide], &[0], false);
    let points = stroke(&core, side);
    let mut context = context(&mut core);
    let view_chunks = context.view.road_mesh_chunks.clone();
    let surface_generation = context.view.surface_generation;
    let preview = Arc::new(context.preview_road(RoadPreviewRequest {
        surface_generation,
        points: points.clone(),
        fwd_lanes: 2,
        bkw_lanes: 2,
        snap_to_existing_roads: true,
        tunnel: false,
        median: None,
        parking: Default::default(),
        attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
    }));
    let pieces = preview.pieces.as_ref().expect("a valid stroke compiles");
    let outcome = core.commit_road(
        RoadCommitRequest {
            points,
            fwd_lanes: 2,
            bkw_lanes: 2,
            snap_to_existing_roads: true,
            tunnel: false,
            median: None,
            parking: Default::default(),
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
            preview: Some(Arc::clone(&preview)),
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    assert_preview_composes_committed(&core, &view_chunks, pieces, "2 + 2 T");

    let tee = triangles(&core);
    // Each arm as (unit direction away from the junction, the lateral axis): a car approaching
    // on it drives towards `-direction`, and under right-hand traffic its lanes lie on the right
    // of that heading.
    let hand = if DRIVE_ON_LEFT { -1.0 } else { 1.0 };
    // Each approach's [left, straight, right] arrows on its inner and outer lane: from the west
    // the side road is a right turn, from the east a left one, the stem turns either way.
    const S: bool = true;
    const N: bool = false;
    for (name, along, right_of_approach, expected) in [
        (
            "west",
            Vector3::LEFT,
            Vector3::BACK * hand,
            [[N, S, N], [N, S, S]],
        ),
        (
            "east",
            Vector3::RIGHT,
            Vector3::FORWARD * hand,
            [[S, S, N], [N, S, N]],
        ),
        (
            "south",
            Vector3::BACK,
            Vector3::RIGHT * hand,
            [[S, N, N], [N, N, S]],
        ),
    ] {
        let arm: Vec<_> = tee
            .iter()
            .map(|(centroid, length, pattern, vertices)| {
                let flat = Vector3::new(centroid.x, 0.0, centroid.z);
                let lateral = flat.dot(right_of_approach);
                // The triangle's vertices as (distance from the junction, lateral).
                let points: Vec<_> = vertices
                    .iter()
                    .map(|v| (v.dot(along), v.dot(right_of_approach)))
                    .collect();
                (flat.dot(along), lateral, *length, *pattern, points)
            })
            .filter(|(distance, lateral, ..)| *distance > 8.0 && lateral.abs() < 8.0)
            .collect();
        let line = |lateral: f32| {
            arm.iter()
                .filter(move |(_, at, ..)| (at - lateral).abs() < 0.3)
        };
        let solid: Vec<_> = line(LANE_WIDTH).filter(|q| q.3 == SOLID).collect();
        let length = solid.iter().map(|q| q.2).sum::<f32>() * 0.5;
        assert!(
            (length - APPROACH_SOLID_LENGTH_M).abs() < 0.5,
            "{name}: the approach divider is solid over {length} m"
        );
        let solid_far = solid.iter().map(|q| q.0).fold(f32::MIN, f32::max);
        let dashed_near = line(LANE_WIDTH)
            .filter(|q| q.3 != SOLID)
            .map(|q| q.0)
            .fold(f32::MAX, f32::min);
        assert!(
            solid_far < dashed_near,
            "{name}: the solid zone ends at the junction, the dashes lie before it"
        );
        // Lines lie on lane edges; any other paint is an arrow, solid, inside an approach lane.
        // The centre line (lateral 0) is solid over the whole arm.
        let on_line = |lateral: f32| {
            [-LANE_WIDTH, LANE_WIDTH]
                .iter()
                .any(|&at| (lateral - at).abs() < 0.3)
        };
        assert!(
            arm.iter().filter(|q| q.3 == SOLID && on_line(q.1)).count() == solid.len(),
            "{name}: of the lane dividers only the approach divider is solid"
        );
        assert!(
            line(0.0).all(|q| q.3 == SOLID),
            "{name}: the centre line is solid"
        );
        assert!(line(0.0).count() > 0 && line(-LANE_WIDTH).count() > 0);
        // The marking range ends where the solid zone does, at the mouth.
        let mouth = solid
            .iter()
            .flat_map(|q| q.4.iter().map(|p| p.0))
            .fold(f32::MAX, f32::min);
        // Under left-hand traffic the inner lane is the rightmost.
        let expected = if DRIVE_ON_LEFT {
            [expected[1], expected[0]]
        } else {
            expected
        };
        for (lane, [left, straight, right]) in [(0.5, expected[0]), (1.5, expected[1])] {
            let centre = lane * LANE_WIDTH;
            // The style's triangle count and where its tail sits: right of the lane's centre
            // beside a lone left branch, left of it beside a lone right one (`arrow_glyph`).
            let triangles = match (left, straight, right) {
                (false, true, false) => 3,
                (_, true, _) => 16,
                _ => 15,
            };
            let tail_side =
                f32::from(u8::from(left && !right)) - f32::from(u8::from(right && !left));
            let paint: Vec<_> = arm
                .iter()
                .filter(|q| !on_line(q.1) && (q.1 - centre).abs() < 1.5)
                .collect();
            // One glyph centred 12 m back from the mouth, 5 m long, and nothing else painted in
            // the lane.
            let back = 12.0;
            let glyph: Vec<_> = paint
                .iter()
                .filter(|q| (q.0 - mouth - back).abs() <= 2.5)
                .collect();
            assert_eq!(
                glyph.len(),
                triangles,
                "{name}: lane {lane} shows its movements {back} m back"
            );
            // Vertices in the driver's frame: `x` right of the lane's centre, `y` ahead.
            let points: Vec<_> = glyph
                .iter()
                .flat_map(|q| q.4.iter())
                .map(|&(distance, lateral)| ((lateral - centre) * hand, mouth + back - distance))
                .collect();
            assert!(
                points.iter().all(|p| p.0.abs() <= 1.3 && p.1.abs() <= 2.55),
                "{name}: lane {lane}'s arrow stays in its lane and its 5 m"
            );
            // The tail's middle, between its two corners.
            let (low, high) = points
                .iter()
                .filter(|p| p.1 < -2.45)
                .fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                    (lo.min(p.0), hi.max(p.0))
                });
            let tail = (low + high) * 0.5;
            assert!(
                (tail - tail_side * tail.abs()).abs() < 1.0e-3
                    && (tail_side == 0.0) == (tail.abs() < 0.05),
                "{name}: lane {lane}'s tail sits at {tail}"
            );
            assert_eq!(
                glyph.len(),
                paint.len(),
                "{name}: lane {lane} holds only its one glyph"
            );
        }
    }
}

// `ROAD-75`: a lane rule edit repaints its node's arms. On a 1 + 1 T, whose arms carry only their
// centre lines, whitelisting the side road's approach lane into the west arm paints a left-turn
// arrow on it; the published chunks then hold what a full paint of the city holds, and undo takes
// the arrow off again.
#[test]
fn a_lane_rule_edit_repaints_the_arrows_of_its_arms() {
    let (mut core, node, side, west) = super::road_crosswalks::t_junction();
    core.benchmark_mode = false;
    let painted = |core: &mut SimCore| {
        core.precompute_road_mesh_data();
        let chunks: usize = core
            .cached_road_mesh_chunks
            .values()
            .map(|mesh| mesh.marking_vertices.len())
            .sum();
        let full = core
            .transit_network
            .try_generate_mesh_data(&core.region_graph, &core.heightmap)
            .expect("current roadbed")
            .marking_vertices
            .len();
        assert_eq!(chunks, full, "the chunks hold the city's paint");
        chunks
    };
    let centre_lines = painted(&mut core);
    let graph = &core.region_graph;
    let approach = if graph.edge(side).end_node == node {
        0
    } else {
        -1
    };
    let leave = if graph.edge(west).start_node == node {
        0
    } else {
        -1
    };
    core.set_lane_connection_internal(node, side as i32, approach, west as i32, leave);
    assert!(
        painted(&mut core) > centre_lines,
        "the whitelisted lane carries its arrow"
    );
    assert!(core.undo_action_internal());
    assert_eq!(painted(&mut core), centre_lines, "undo takes the arrow off");
}
