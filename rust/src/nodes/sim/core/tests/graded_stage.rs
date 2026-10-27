// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-E` acceptance, the Rust half: the graded tiles of every drag frame, the commit,
//! the undo, a recommit of the same stroke and a saved and loaded city each equal the fresh bake
//! of the state they stand for, bitwise, on the T, the Y, the four-way and the split trunk, flat
//! and on the 2.5% slope. A drag's frames stage a bounded share of each pose's tiles (T27 (d)):
//! every shown tile is the exact bake of some pose, and once the poses stop the shown tiles
//! converge to the last pose's in a bounded number of frames; the commit then adopts them all,
//! a commit right after the drag bakes what the preview had not, undo adopts the kept tiles,
//! and a commit with a stale context bakes and equals a fresh bake. Also the drag harness the
//! Kuopio and populated drag benchmarks share.

use super::road_edit_order::{ARM_M, Stroke, committed, committed_on};
use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::nodes::sim::core::graded_stage::{CATCH_UP_FRAME_TILES, GradedPose, POSE_FRAME_TILES};
use crate::nodes::sim::core::road_preview::{RoadPreview, RoadPreviewRequest};
use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{Level, SiteSource, TilePayload, TileStage, bake_tiles};
use godot::prelude::Vector2;
use std::sync::Arc;

const THROUGH: Stroke = ([Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0)], 1);

// A fixture: its name, the world's width, the committed roads and the dragged stroke's ends.
struct Case {
    name: &'static str,
    world_m: f32,
    roads: Vec<Stroke>,
    drag: [Vector2; 2],
}

fn cases() -> Vec<Case> {
    let (s, c) = 60f32.to_radians().sin_cos();
    vec![
        Case {
            name: "t",
            world_m: 512.0,
            roads: vec![THROUGH],
            drag: [Vector2::ZERO, Vector2::new(0.0, ARM_M)],
        },
        Case {
            name: "y",
            world_m: 512.0,
            roads: vec![THROUGH],
            drag: [Vector2::ZERO, Vector2::new(ARM_M * c, ARM_M * s)],
        },
        Case {
            name: "four_way",
            world_m: 512.0,
            roads: vec![THROUGH, ([Vector2::ZERO, Vector2::new(0.0, ARM_M)], 1)],
            drag: [Vector2::ZERO, Vector2::new(0.0, -ARM_M)],
        },
        Case {
            name: "split_trunk",
            world_m: 1024.0,
            roads: vec![([Vector2::new(-330.0, 0.0), Vector2::new(330.0, 0.0)], 1)],
            drag: [Vector2::new(0.0, -80.0), Vector2::new(25.0, 0.0)],
        },
    ]
}

fn stroke(core: &SimCore, ends: [Vector2; 2]) -> Vec<Vector3> {
    ends.map(|point| {
        let height = core.get_world_surface_height_internal(point);
        Vector3::new(point.x, height, point.y)
    })
    .to_vec()
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

fn commit(core: &mut SimCore, points: &[Vector3], preview: Option<Arc<RoadPreview>>) {
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
}

// What a tile's bytes and inputs are, bit for bit.
type TileBits = (
    Vec<[u32; 4]>,
    Vec<[u32; 4]>,
    Vec<[u32; 4]>,
    String,
    bool,
    String,
);
// Each tile's bits by level and tile, in stage order.
type StageBits = Vec<((Level, (i32, i32)), TileBits)>;

fn tile_bits(payload: &TilePayload) -> TileBits {
    let bits = |rows: &[[f32; 4]]| rows.iter().map(|row| row.map(f32::to_bits)).collect();
    (
        bits(&payload.texels),
        bits(&payload.seams),
        bits(&payload.edges),
        // `Debug` prints each float's shortest round-trip form: equal text is equal bits.
        format!("{:?}", payload.walls),
        payload.graded,
        format!("{:?}", payload.inputs),
    )
}

fn stage_bits(stage: &TileStage) -> StageBits {
    stage
        .tiles()
        .iter()
        .map(|held| {
            (
                (held.level, (held.tile.x, held.tile.z)),
                tile_bits(&held.payload),
            )
        })
        .collect()
}

// The fresh bake of `stage`'s tiles over the given state.
fn fresh(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    stage: &TileStage,
) -> StageBits {
    stage
        .tiles()
        .iter()
        .map(|held| {
            let mut out = Vec::new();
            bake_tiles(terrain, roadbed, sites, &[held.tile], held.level, &mut out)
                .expect("the tile bakes");
            ((held.level, (held.tile.x, held.tile.z)), tile_bits(&out[0]))
        })
        .collect()
}

fn live_fresh(core: &mut SimCore, stage: &TileStage) -> StageBits {
    if !core.allocator.building_site_query_index_ready() {
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
    }
    fresh(
        &core.heightmap,
        &core.transit_network.roadbed,
        &core.allocator,
        stage,
    )
}

fn last_tiles(core: &SimCore) -> Arc<TileStage> {
    Arc::clone(
        core.last_graded_tiles
            .as_ref()
            .expect("the graded tiles bake"),
    )
}

/// A digest of a stage's products, untimed, to compare builds bitwise.
pub(crate) fn stage_digest(stage: &TileStage) -> String {
    let payloads = stage.tiles().iter().map(|held| &*held.payload);
    let digest = crate::simulation::terrain::graded::tests::payload_digest(payloads);
    format!("{digest:016x}")
}

/// A dragged stroke's frames on the road tool's preview (T27 (d)).
pub(crate) struct Drag {
    /// Per frame what its graded terrain did: `pose` (1-based; `null` for a catch-up frame),
    /// `frame_ms` (the whole terrain work), `select_ms`, `bake_ms`, `wanted`, `adopted`,
    /// `baked`, `pending`, the shown tiles' `digest` and the call's `span_ms` on the benchmark
    /// clock.
    pub(crate) frames: Vec<serde_json::Value>,
    /// The slowest frame's terrain work, milliseconds.
    pub(crate) frame_max_ms: f64,
    /// The catch-up frames after the last pose.
    pub(crate) catch_up_frames: usize,
    /// The last pose's points and preview, its graded tiles those shown after the last frame.
    pub(crate) points: Vec<Vector3>,
    pub(crate) preview: RoadPreview,
}

fn frame_record(pose: Option<usize>, graded: &GradedPose, span: serde_json::Value) -> serde_json::Value {
    let stats = graded.stats;
    serde_json::json!({
        "pose": pose, "span_ms": span, "frame_ms": graded.frame_ms,
        "select_ms": stats.select_ms, "bake_ms": stats.bake_ms, "wanted": stats.wanted,
        "adopted": stats.adopted, "baked": stats.baked, "pending": graded.pending,
        "error": graded.stage.as_ref().err().map(|e| format!("{e:?}")),
        "digest": graded.stage.as_deref().ok().map(stage_digest),
    })
}

/// Drags the stroke through `poses` (each pose's points) on `context`, one frame a pose; with
/// `converge`, then the catch-up frames until the shown tiles are the last pose's.
pub(crate) fn drag(
    context: &mut RoadPreviewContext,
    poses: &[Vec<Vector3>],
    [fwd_lanes, bkw_lanes]: [i32; 2],
    converge: bool,
) -> Drag {
    use crate::thread_pools::spans::ms;
    let span = |start: std::time::Instant| serde_json::json!([ms(start), ms(std::time::Instant::now())]);
    let (mut frames, mut frame_max_ms, mut last) = (Vec::new(), 0.0f64, None);
    for (step, points) in poses.iter().enumerate() {
        let surface_generation = context.view.surface_generation;
        let called = std::time::Instant::now();
        let preview = context.preview_road(RoadPreviewRequest {
            surface_generation,
            points: points.clone(),
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads: true,
        });
        let span = span(called);
        frames.push(match preview.pieces.as_ref() {
            Some(pieces) => {
                frame_max_ms = frame_max_ms.max(pieces.graded.frame_ms);
                frame_record(Some(step + 1), &pieces.graded, span)
            }
            None => serde_json::json!({
                "pose": step + 1, "span_ms": span, "invalid": preview.validation.invalid_reason,
            }),
        });
        last = Some((points.clone(), preview));
    }
    let (points, mut preview) = last.expect("a pose");
    let mut catch_up_frames = 0;
    while converge {
        let called = std::time::Instant::now();
        let Some(graded) = context.catch_up_graded() else {
            break;
        };
        let span = span(called);
        catch_up_frames += 1;
        frame_max_ms = frame_max_ms.max(graded.frame_ms);
        frames.push(frame_record(None, &graded, span));
        if let Some(pieces) = preview.pieces.as_mut() {
            pieces.graded = graded;
        }
    }
    Drag {
        frames,
        frame_max_ms,
        catch_up_frames,
        points,
        preview,
    }
}

// The commit of a drag's last pose with its preview (`TERRAIN-07-E`): its graded stage, the
// committed tiles' digest and the call's span; with what the undo record keeps, once baked.
fn click(core: &mut SimCore, drag: Drag, [fwd_lanes, bkw_lanes]: [i32; 2]) -> serde_json::Value {
    use crate::thread_pools::spans::ms;
    let called = std::time::Instant::now();
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: drag.points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads: true,
            preview: Some(Arc::new(drag.preview)),
        },
        called,
        Default::default(),
    );
    let span = serde_json::json!([ms(called), ms(std::time::Instant::now())]);
    assert!(outcome.committed, "{}", outcome.rejection);
    let stats = core.last_graded_stats;
    let metrics = &core.last_road_edit_metrics;
    let kept = core
        .undo_stack
        .back()
        .and_then(|record| record.graded.as_ref())
        .expect("the commit keeps its replaced tiles")
        .wait()
        .map(|kept| (kept.len(), kept.payload_bytes()))
        .expect("the kept tiles bake");
    serde_json::json!({
        "ms": stats.total_ms(), "select_ms": stats.select_ms, "bake_ms": stats.bake_ms,
        "tiles": stats.wanted, "adopted": stats.adopted, "baked": stats.baked,
        "digest": core.last_graded_tiles.as_deref().ok().map(stage_digest), "span_ms": span,
        "surface_graded_ms": metrics.surface_graded_ms,
        "preview_reused": metrics.preview_plan_reused,
        "kept_tiles": kept.0, "kept_mb": kept.1 as f64 / 1e6,
    })
}

// The undo of the last commit: its graded stage, the restored tiles' digest and the call's span.
fn undo(core: &mut SimCore) -> serde_json::Value {
    use crate::thread_pools::spans::ms;
    let called = std::time::Instant::now();
    assert!(core.undo_action_internal(), "undo");
    let span = serde_json::json!([ms(called), ms(std::time::Instant::now())]);
    let stats = core.last_graded_stats;
    serde_json::json!({
        "ms": stats.total_ms(), "tiles": stats.wanted, "adopted": stats.adopted,
        "baked": stats.baked, "digest": core.last_graded_tiles.as_deref().ok().map(stage_digest),
        "span_ms": span,
    })
}

/// One stroke's drag benchmark (T27 (d)) on `core`, which ends as it began: the drag through
/// `poses` clicked right after its last pose's frame (`fast_commit`, before the preview has
/// caught up) and undone, then the same drag converged (`frames`: every pose frame and
/// catch-up frame) and clicked (`commit`, every tile adopted) and undone (`undo`, the kept
/// tiles adopted). `pose_frames_equal`: the two drags' pose frames showed the same tiles.
/// Commits and undos keep undo records; benchmark mode is restored after.
pub(crate) fn drag_and_click(
    core: &mut SimCore,
    poses: &[Vec<Vector3>],
    lanes: [i32; 2],
) -> serde_json::Value {
    let capture = |core: &mut SimCore| {
        core.refresh_road_terrain_inputs();
        RoadPreviewContext::capture(road_tool_snapshot_from_core(core).expect("current roadbed"))
    };
    let mode = core.benchmark_mode;
    core.benchmark_mode = false;
    let fast = drag(&mut capture(core), poses, lanes, false);
    let (fast_max, fast_digests) = (fast.frame_max_ms, pose_digests(&fast.frames, poses.len()));
    let fast_commit = click(core, fast, lanes);
    let fast_undo = undo(core);
    let converged = drag(&mut capture(core), poses, lanes, true);
    let same = fast_digests == pose_digests(&converged.frames, poses.len());
    let converged_digest = converged.frames.last().map(|frame| frame["digest"].clone());
    let (frames, frame_max_ms, catch_up_frames) = (
        converged.frames.clone(),
        converged.frame_max_ms,
        converged.catch_up_frames,
    );
    let commit = click(core, converged, lanes);
    let undo = undo(core);
    core.benchmark_mode = mode;
    serde_json::json!({
        "frames": frames, "frame_max_ms": frame_max_ms, "catch_up_frames": catch_up_frames,
        "converged_digest": converged_digest, "pose_frames_equal": same,
        "fast_frame_max_ms": fast_max, "fast_commit": fast_commit, "fast_undo": fast_undo,
        "commit": commit, "undo": undo,
    })
}

fn pose_digests(frames: &[serde_json::Value], poses: usize) -> Vec<serde_json::Value> {
    frames[..poses].iter().map(|frame| frame["digest"].clone()).collect()
}

// One drag frame's shown tiles (T27 (d)): each is the fresh bake of the frame's pose or the
// tile the frame before showed; at least the frame's staged tiles are the pose's; the share
// stays within its bound. Returns the shown tiles.
fn check_frame(
    label: &str,
    context: &RoadPreviewContext,
    roadbed: &RoadbedSystem,
    core: &SimCore,
    (graded, bound): (&GradedPose, usize),
    before: &TileStage,
) -> Arc<TileStage> {
    let stage = graded.stage.as_ref().expect("the frame's tiles bake");
    let stats = graded.stats;
    assert!(stats.adopted + stats.baked <= bound, "{label}: the frame's share");
    assert!(stage.len() <= stats.wanted, "{label}: only the pose's tiles shown");
    let fresh = fresh(&context.view.terrain, roadbed, &core.allocator, stage);
    let before = stage_bits(before);
    let mut posed = 0;
    for (shown, fresh) in stage_bits(stage).iter().zip(&fresh) {
        if shown == fresh {
            posed += 1;
        } else {
            assert!(
                before.contains(shown),
                "{label}: tile {:?} is neither the pose's bake nor the tile shown before",
                shown.0
            );
        }
    }
    assert!(posed >= stats.adopted + stats.baked, "{label}: the staged tiles are the pose's");
    Arc::clone(stage)
}

#[test]
fn graded_tiles_of_drags_commits_undo_and_loads_equal_fresh_bakes() {
    for case in cases() {
        for sloped in [false, true] {
            let name = format!("{} sloped {sloped}", case.name);
            let order: Vec<usize> = (0..case.roads.len()).collect();
            let mut core = committed_on(case.world_m, &case.roads, &order, sloped);
            core.benchmark_mode = false;
            core.refresh_road_terrain_inputs();
            let mut context = RoadPreviewContext::capture(
                road_tool_snapshot_from_core(&core).expect("current roadbed"),
            );
            // The drag: the stroke grows from its start towards its end, a frame a pose, then
            // the catch-up frames.
            let [start, end] = case.drag;
            let poses: Vec<_> = [0.4, 0.6, 0.8, 1.0]
                .map(|u| stroke(&core, [start, start.lerp(end, u)]))
                .to_vec();
            let mut shown = Arc::new(TileStage::default());
            let mut last = None;
            for (index, points) in poses.iter().enumerate() {
                let pose = preview(&mut context, points);
                let pieces = pose.pieces.as_ref().unwrap_or_else(|| {
                    panic!("{name} pose {index}: {}", pose.validation.invalid_reason)
                });
                let label = format!("{name} pose {index}");
                let frame = (&pieces.graded, POSE_FRAME_TILES);
                shown = check_frame(&label, &context, &pieces.roadbed, &core, frame, &shown);
                assert!(!shown.is_empty(), "{label}: the pose grades tiles");
                last = Some(pose);
            }
            let mut pose = last.unwrap();
            let pieces = pose.pieces.as_mut().unwrap();
            let (wanted, pending) = (pieces.graded.stats.wanted, pieces.graded.pending);
            let mut frames = 0;
            while let Some(graded) = context.catch_up_graded() {
                frames += 1;
                let label = format!("{name} catch-up frame {frames}");
                let frame = (&graded, CATCH_UP_FRAME_TILES);
                shown = check_frame(&label, &context, &pieces.roadbed, &core, frame, &shown);
                pieces.graded = graded;
            }
            assert_eq!(
                frames,
                pending.div_ceil(CATCH_UP_FRAME_TILES),
                "{name}: the shown tiles converge in the bounded frames"
            );
            assert_eq!(pieces.graded.pending, 0, "{name}: converged");
            assert_eq!(shown.len(), wanted, "{name}: every wanted tile shown");
            assert_eq!(
                stage_bits(&shown),
                fresh(&context.view.terrain, &pieces.roadbed, &core.allocator, &shown),
                "{name}: the converged tiles are the fresh bake of the last pose"
            );
            let (points, pose) = (poses[poses.len() - 1].clone(), Arc::new(pose));
            let posed = Arc::clone(&shown);

            // Before the commit: the tiles the commit will replace, baked fresh.
            let before = live_fresh(&mut core, &posed);
            commit(&mut core, &points, Some(pose));
            let committed = last_tiles(&core);
            let metrics = &core.last_road_edit_metrics;
            assert_eq!(
                (metrics.graded_adopted, metrics.graded_baked),
                (committed.len(), 0),
                "{name}: the commit adopts the last pose's tiles"
            );
            assert_eq!(
                stage_bits(&committed),
                stage_bits(&posed),
                "{name}: commit = pose"
            );
            assert_eq!(
                stage_bits(&committed),
                live_fresh(&mut core, &committed),
                "{name}: the commit's tiles are the fresh bake of the committed city"
            );

            // Undo: the kept tiles are the pre-commit tiles, adopted.
            let kept = core
                .undo_stack
                .back()
                .and_then(|record| record.graded.as_ref());
            let kept = kept.expect("the commit's undo record keeps its graded tiles");
            let kept = kept.wait().expect("the kept tiles bake").clone();
            assert_eq!(
                stage_bits(&kept),
                before,
                "{name}: the undo record keeps the tiles the commit replaced"
            );
            assert!(core.undo_action_internal(), "{name}: undo");
            let undone = last_tiles(&core);
            assert_eq!(
                (core.last_graded_stats.adopted, core.last_graded_stats.baked),
                (undone.len(), 0),
                "{name}: undo swaps the kept tiles back"
            );
            assert_eq!(
                stage_bits(&undone),
                before,
                "{name}: undo restores the tiles"
            );
            assert_eq!(
                stage_bits(&undone),
                live_fresh(&mut core, &undone),
                "{name}: the undone tiles are the fresh bake of the restored city"
            );

            // The same stroke again, without a preview: the first commit's tiles.
            commit(&mut core, &points, None);
            let recommitted = last_tiles(&core);
            assert_eq!(
                stage_bits(&recommitted),
                stage_bits(&committed),
                "{name}: a recommit equals the commit"
            );

            // Save and load: residency bakes the same tiles on the loaded city.
            let path = temp_save_path("graded_stage");
            core.save_game_settled(path.to_str().unwrap(), None)
                .expect("the fixture saves");
            let mut loaded = test_core();
            let result = loaded.load_game_internal(path.to_str().unwrap());
            std::fs::remove_file(&path).unwrap();
            result.expect("the fixture loads");
            loaded.precompute_road_mesh_data();
            // The base revisions in the inputs are the core's counters, not saved; the tile
            // data is the content.
            let data = |bits: StageBits| -> Vec<_> {
                bits.into_iter()
                    .map(|(key, bits)| (key, (bits.0, bits.1, bits.2, bits.3, bits.4)))
                    .collect()
            };
            assert_eq!(
                data(live_fresh(&mut loaded, &committed)),
                data(stage_bits(&committed)),
                "{name}: the loaded city bakes the committed tiles"
            );
        }
    }
}

// T27 (d): a click right after a fast drag, before the preview has caught up, commits the tiles
// a click after the converged drag commits, bitwise: the commit adopts the shown tiles whose
// inputs match and bakes the rest in its own frame. The two drags' pose frames show the same
// tiles: they depend on the pose sequence only.
#[test]
fn a_commit_right_after_a_fast_drag_bakes_what_the_preview_had_not() {
    let case = &cases()[3];
    let order: Vec<usize> = (0..case.roads.len()).collect();
    let [start, end] = case.drag;
    let mut runs = Vec::new();
    for converge in [false, true] {
        let mut core = committed_on(case.world_m, &case.roads, &order, true);
        core.benchmark_mode = false;
        core.refresh_road_terrain_inputs();
        let mut context = RoadPreviewContext::capture(
            road_tool_snapshot_from_core(&core).expect("current roadbed"),
        );
        let poses: Vec<_> = [0.4, 0.6, 0.8, 1.0]
            .map(|u| stroke(&core, [start, start.lerp(end, u)]))
            .to_vec();
        let drag = drag(&mut context, &poses, [1, 1], converge);
        let pose_digests: Vec<_> = drag.frames[..poses.len()]
            .iter()
            .map(|frame| frame["digest"].clone())
            .collect();
        let pending = drag.preview.pieces.as_ref().unwrap().graded.pending;
        commit(&mut core, &drag.points, Some(Arc::new(drag.preview)));
        let tiles = last_tiles(&core);
        let metrics = &core.last_road_edit_metrics;
        let (adopted, baked) = (metrics.graded_adopted, metrics.graded_baked);
        assert_eq!(adopted + baked, tiles.len(), "converge {converge}");
        assert_eq!(
            stage_bits(&tiles),
            live_fresh(&mut core, &tiles),
            "converge {converge}: the committed tiles are the fresh bake"
        );
        if converge {
            assert_eq!(baked, 0, "the converged preview's tiles are all adopted");
        } else {
            assert!(pending > 0 && baked > 0, "the fast click bakes the unstaged tiles");
        }
        runs.push((pose_digests, stage_bits(&tiles)));
    }
    assert_eq!(runs[0].0, runs[1].0, "the pose frames show the same tiles");
    assert_eq!(runs[0].1, runs[1].1, "the fast click commits the converged click's tiles");
}

#[test]
fn a_stale_context_commit_bakes_and_equals_a_fresh_bake() {
    let side = [Vector2::ZERO, Vector2::new(0.0, ARM_M)];
    let elsewhere: Stroke = ([Vector2::new(-ARM_M, 120.0), Vector2::new(ARM_M, 120.0)], 1);
    for sloped in [false, true] {
        let mut core = committed(&[THROUGH], &[0], sloped);
        core.refresh_road_terrain_inputs();
        let mut context = RoadPreviewContext::capture(
            road_tool_snapshot_from_core(&core).expect("current roadbed"),
        );
        let points = stroke(&core, side);
        let stale = Arc::new(preview(&mut context, &points));
        let other = stroke(&core, elsewhere.0);
        commit(&mut core, &other, None);
        commit(&mut core, &points, Some(stale));
        let tiles = last_tiles(&core);
        assert_eq!(
            core.last_road_edit_metrics.graded_adopted, 0,
            "sloped {sloped}: a stale context's tiles are not offered"
        );
        assert_eq!(
            stage_bits(&tiles),
            live_fresh(&mut core, &tiles),
            "sloped {sloped}: the stale commit's tiles are the fresh bake"
        );
        let mut control = committed(&[THROUGH, elsewhere], &[0, 1], sloped);
        commit(&mut control, &points, None);
        assert_eq!(
            stage_bits(&tiles),
            stage_bits(&last_tiles(&control)),
            "sloped {sloped}: the stale commit equals a fresh commit"
        );
    }
}
