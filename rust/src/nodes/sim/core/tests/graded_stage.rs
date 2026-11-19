// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-E` acceptance, the Rust half: the graded tiles of every drag frame, the commit,
//! the undo, a recommit of the same stroke and a saved and loaded city each equal the fresh bake
//! of the state they stand for, bitwise, on the T, the Y, the four-way and the split trunk, flat
//! and on the 2.5% slope. A drag's poses are staged as whole sets on the background pool
//! (`TERRAIN-14`): every set is the exact bake of one pose, and once the poses stop the last
//! pose's set lands and displays; the commit then adopts them all,
//! a commit right after the drag bakes what the preview had not, undo adopts the kept tiles,
//! and a commit with a stale context bakes and equals a fresh bake. Also the drag harness the
//! Kuopio and populated drag benchmarks share.

use super::road_edit_order::{ARM_M, Stroke, committed, committed_on};
use super::*;
use crate::nodes::sim::core::graded_stage::GradedPose;
use crate::nodes::simulation_node::graded_tiles_api::PackedTile;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
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
        profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
        median: None,
        parking: crate::simulation::network::graph::RoadParking::NONE,
        attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
    })
}

fn commit(core: &mut SimCore, points: &[Vector3], preview: Option<Arc<RoadPreview>>) {
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking: crate::simulation::network::graph::RoadParking::NONE,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
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

// T65 option (c) (`ROAD-112` round G2): what one drawn frame of a pose shows where it replaces
// published pieces. The road draws the pose's roadbed (the published chunks with the replaced runs
// collapsed and the rebuilt pieces over them); the terrain draws the displayed pose tiles, else the
// published tiles, which discard the published grounded coverage. Sampled every `step` metres
// over the replaced pieces' bounds (outside them every published cut lies under a piece the road
// still draws, at the published top): `holes` the published cut's points with no drawn road,
// `open` those where the drawn road's top differs from the published one (a hole, or a step at the
// cut: the background shows past it), `open_outside` those outside the backstop tiles
// (`GradedPreview::cut_tiles`), `steps` the open points at the cut's edge (a neighbour a step away
// lies outside it) with the most the drawn top lies below (`drop_max_m`) and above (`rise_max_m`)
// the published one there, and `over_road` the backstop tiles' cut points where the published
// ground (the inside rule) stands above the drawn top, which a backstop at the ground's own
// depth would show over the road (the most: `over_max_m`); `rule_us` the backstop tiles' cost.
#[derive(Debug, Default)]
pub(crate) struct CutFrame {
    pub(crate) replaced: usize,
    pub(crate) rule_tiles: usize,
    pub(crate) rule_us: f64,
    pub(crate) displayed: usize,
    pub(crate) holes: usize,
    pub(crate) open: usize,
    pub(crate) open_outside: usize,
    pub(crate) steps: usize,
    pub(crate) drop_max_m: f64,
    pub(crate) rise_max_m: f64,
    pub(crate) over_road: usize,
    pub(crate) over_max_m: f64,
}

impl CutFrame {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "replaced": self.replaced, "rule_tiles": self.rule_tiles, "rule_us": self.rule_us,
            "displayed": self.displayed, "holes": self.holes, "open": self.open,
            "open_outside": self.open_outside, "steps": self.steps,
            "drop_max_m": self.drop_max_m, "rise_max_m": self.rise_max_m,
            "over_road": self.over_road, "over_max_m": self.over_max_m,
        })
    }
}

pub(crate) fn cut_frame(
    context: &mut RoadPreviewContext,
    preview: &RoadPreview,
    step: f64,
) -> Option<CutFrame> {
    use crate::simulation::terrain::graded::TileKey;
    let pieces = preview.pieces.as_ref()?;
    let published = Arc::clone(&context.view.roadbed);
    let drawn = &pieces.roadbed;
    let started = std::time::Instant::now();
    let rule = context
        .graded
        .cut_tiles(&published, Some(&pieces.delta))
        .to_vec();
    let mut frame = CutFrame {
        rule_us: started.elapsed().as_secs_f64() * 1e6,
        rule_tiles: rule.len(),
        ..CutFrame::default()
    };
    let mut displayed: Vec<TileKey> = (context.graded.displayed().tiles().iter())
        .map(|tile| tile.tile)
        .collect();
    displayed.sort_unstable();
    frame.displayed = displayed.len();
    let replaced: Vec<_> = (pieces.delta.pieces())
        .filter_map(|(id, _)| published.piece_mesh(id))
        .collect();
    frame.replaced = replaced.len();
    if replaced.is_empty() {
        return Some(frame);
    }
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for mesh in &replaced {
        let [a, b, c, d] = mesh.bounds_xz();
        bounds = [bounds[0].min(a), bounds[1].min(b), bounds[2].max(c), bounds[3].max(d)];
    }
    let (nx, nz) = (
        ((bounds[2] - bounds[0]) / step).ceil() as i64,
        ((bounds[3] - bounds[1]) / step).ceil() as i64,
    );
    let cut = |x: f64, z: f64| published.grounded_height_at(x as f32, z as f32).is_some();
    context.view.with_graded_surface(|surface, scratch| {
        for i in 0..=nx {
            for j in 0..=nz {
                let (x, z) = (bounds[0] + i as f64 * step, bounds[1] + j as f64 * step);
                if !cut(x, z) {
                    continue;
                }
                let tile = TileKey::at(x, z);
                if displayed.binary_search(&tile).is_ok() {
                    // The pose's exact tile: its cut is the drawn road's.
                    continue;
                }
                let in_rule = rule.binary_search(&tile).is_ok();
                let (old, new) = (
                    published.height_at(x as f32, z as f32),
                    drawn.height_at(x as f32, z as f32),
                );
                frame.holes += usize::from(new.is_none());
                if old != new {
                    frame.open += 1;
                    frame.open_outside += usize::from(!in_rule);
                    let edge = [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)]
                        .iter()
                        .any(|&(dx, dz)| !cut(x + dx, z + dz));
                    if edge && let (Some(old), Some(new)) = (old, new) {
                        let drop = f64::from(old) - f64::from(new);
                        frame.steps += 1;
                        frame.drop_max_m = frame.drop_max_m.max(drop);
                        frame.rise_max_m = frame.rise_max_m.max(-drop);
                    }
                }
                if let (true, Some(top)) = (in_rule, new) {
                    let above = surface.ground_height_at(x, z, scratch) - f64::from(top);
                    if above > 0.0 {
                        frame.over_road += 1;
                        frame.over_max_m = frame.over_max_m.max(above);
                    }
                }
            }
        }
    });
    Some(frame)
}

// The spike's record of a frame, with `METRUM_CUT_SPIKE` set (sampled every
// `METRUM_CUT_SPIKE_STEP` metres, 0.25 by default).
fn cut_spike(context: &mut RoadPreviewContext, preview: &RoadPreview) -> Option<serde_json::Value> {
    std::env::var_os("METRUM_CUT_SPIKE")?;
    let step = std::env::var("METRUM_CUT_SPIKE_STEP")
        .ok()
        .and_then(|step| step.parse().ok())
        .unwrap_or(0.25);
    cut_frame(context, preview, step).map(|frame| frame.json())
}

/// A dragged stroke's frames on the road tool's preview (`TERRAIN-14`).
pub(crate) struct Drag {
    /// Per frame what its graded terrain did: `pose` (1-based; `null` for a catch-up frame),
    /// `frame_ms` (the frame's terrain work), the newest landed set's `select_ms`, `bake_ms`,
    /// `wanted`, `adopted` and `baked` (its job's), `pending` (sets still to land), `behind`
    /// (poses the set is behind the latest), `job_ms` (a set that landed in the frame: its job
    /// from start to landing), `displayed` (tiles the renderer draws), the set's `digest` and
    /// the call's `span_ms` on the benchmark clock. A catch-up frame that landed no set has
    /// `landed: false` and its `frame_ms` only.
    pub(crate) frames: Vec<serde_json::Value>,
    /// The slowest frame's terrain work, milliseconds.
    pub(crate) frame_max_ms: f64,
    /// The catch-up frames after the last pose (polls, with or without a set landing).
    pub(crate) catch_up_frames: usize,
    /// The last pose's points and preview, its graded tiles those shown after the last frame.
    pub(crate) points: Vec<Vector3>,
    pub(crate) preview: RoadPreview,
}

fn frame_record(
    pose: Option<usize>,
    graded: &GradedPose,
    displayed: usize,
    span: serde_json::Value,
) -> serde_json::Value {
    let stats = graded.stats;
    serde_json::json!({
        "pose": pose, "span_ms": span, "frame_ms": graded.frame_ms,
        "select_ms": stats.select_ms, "bake_ms": stats.bake_ms, "wanted": stats.wanted,
        "adopted": stats.adopted, "baked": stats.baked, "pending": graded.pending,
        "behind": graded.behind, "job_ms": graded.job_ms, "displayed": displayed,
        "error": graded.stage.as_ref().err().map(|e| format!("{e:?}")),
        "digest": graded.stage.as_deref().ok().map(stage_digest),
    })
}

// The drag benchmarks' frame pacing (`METRUM_DRAG_FRAME_MS`, milliseconds a frame): none by
// default.
fn frame_pacing() -> Option<std::time::Duration> {
    let ms: f64 = std::env::var("METRUM_DRAG_FRAME_MS").ok()?.parse().ok()?;
    Some(std::time::Duration::from_secs_f64(ms / 1000.0))
}

/// The drag benchmarks' poses a stroke (`METRUM_DRAG_POSES`, 8 by default): more poses are a
/// slower drag in smaller steps.
pub(crate) fn drag_pose_count() -> usize {
    std::env::var("METRUM_DRAG_POSES")
        .ok()
        .and_then(|poses| poses.parse().ok())
        .filter(|&poses| poses > 0)
        .unwrap_or(8)
}

/// The tests' synchronous path to the preview's converged terrain: waits for each set in flight
/// and lands it until none is pending. Returns the last frame that landed a set.
pub(crate) fn converge_graded(context: &mut RoadPreviewContext) -> Option<GradedPose> {
    let mut last = None;
    while context.graded.pending_sets() > 0 {
        context.graded.wait();
        last = context.catch_up_graded().or(last);
    }
    last
}

/// Drags the stroke through `poses` (each pose's points) on `context`, one frame a pose; with
/// `converge`, then the catch-up frames until no set is pending. With `METRUM_DRAG_FRAME_MS`
/// the frames are paced as a game's (a pose or a poll a frame, the thread asleep between them,
/// so a set lands whenever its job ends); without it each frame first waits for the set in
/// flight (the synchronous path: frame `N` shows pose `N - 1`'s set and the catch-up frame the
/// last pose's), so the frames depend on the poses alone.
pub(crate) fn drag(
    context: &mut RoadPreviewContext,
    poses: &[Vec<Vector3>],
    [fwd_lanes, bkw_lanes]: [i32; 2],
    converge: bool,
) -> Drag {
    use crate::thread_pools::spans::ms;
    let span =
        |start: std::time::Instant| serde_json::json!([ms(start), ms(std::time::Instant::now())]);
    let pacing = frame_pacing();
    let mut next_frame = std::time::Instant::now();
    let mut frame = |context: &RoadPreviewContext| match pacing {
        Some(period) => {
            next_frame += period;
            let now = std::time::Instant::now();
            if next_frame > now {
                std::thread::sleep(next_frame - now);
            } else {
                next_frame = now;
            }
        }
        None => context.graded.wait(),
    };
    let (mut frames, mut frame_max_ms, mut last) = (Vec::new(), 0.0f64, None);
    for (step, points) in poses.iter().enumerate() {
        frame(context);
        let surface_generation = context.view.surface_generation;
        let called = std::time::Instant::now();
        let preview = context.preview_road(RoadPreviewRequest {
            surface_generation,
            points: points.clone(),
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads: true,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking: crate::simulation::network::graph::RoadParking::NONE,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        });
        let span = span(called);
        let displayed = context.graded.displayed().len();
        frames.push(match preview.pieces.as_ref() {
            Some(pieces) => {
                frame_max_ms = frame_max_ms.max(pieces.graded.frame_ms);
                frame_record(Some(step + 1), &pieces.graded, displayed, span)
            }
            None => serde_json::json!({
                "pose": step + 1, "span_ms": span, "invalid": preview.validation.invalid_reason,
            }),
        });
        if let Some(cut) = cut_spike(context, &preview) {
            frames.last_mut().unwrap()["cut"] = cut;
        }
        last = Some((points.clone(), preview));
    }
    let (points, mut preview) = last.expect("a pose");
    let mut catch_up_frames = 0;
    while converge && context.graded.pending_sets() > 0 {
        frame(context);
        let called = std::time::Instant::now();
        let graded = context.catch_up_graded();
        let poll_ms = called.elapsed().as_secs_f64() * 1000.0;
        let span = span(called);
        catch_up_frames += 1;
        let Some(graded) = graded else {
            frame_max_ms = frame_max_ms.max(poll_ms);
            frames.push(serde_json::json!({
                "pose": null, "span_ms": span, "frame_ms": poll_ms, "landed": false,
            }));
            continue;
        };
        frame_max_ms = frame_max_ms.max(graded.frame_ms);
        let displayed = context.graded.displayed().len();
        frames.push(frame_record(None, &graded, displayed, span));
        if let Some(pieces) = preview.pieces.as_mut() {
            pieces.graded = graded;
        }
        if let Some(cut) = cut_spike(context, &preview) {
            frames.last_mut().unwrap()["cut"] = cut;
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
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking: crate::simulation::network::graph::RoadParking::NONE,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
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
        "ms": stats.total_ms(), "select_ms": stats.select_ms, "bake_ms": stats.bake_ms,
        "tiles": stats.wanted, "adopted": stats.adopted, "baked": stats.baked,
        "digest": core.last_graded_tiles.as_deref().ok().map(stage_digest), "span_ms": span,
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
    frames[..poses]
        .iter()
        .map(|frame| frame["digest"].clone())
        .collect()
}

// One drag frame's graded tiles (`TERRAIN-14`): the newest landed set is one pose's whole
// selection, each tile the fresh bake of that pose (`roadbeds`: every staged pose's roadbed, the
// latest last), or none before the first set lands; what the renderer draws is the latest pose's
// whole set or nothing (T60, T61 (1) (a)). Returns the set.
fn check_frame(
    label: &str,
    context: &RoadPreviewContext,
    core: &SimCore,
    graded: &GradedPose,
    roadbeds: &[RoadbedSystem],
) -> Arc<TileStage> {
    let stage = graded.stage.as_ref().expect("the frame's tiles bake");
    let displayed = context.graded.displayed();
    let Some(behind) = graded.behind else {
        assert!(stage.is_empty(), "{label}: no set landed, none shown");
        assert!(displayed.is_empty(), "{label}: no set landed, none displayed");
        return Arc::clone(stage);
    };
    let roadbed = &roadbeds[roadbeds.len() - 1 - behind as usize];
    assert_eq!(stage.len(), graded.stats.wanted, "{label}: a whole set");
    assert_eq!(
        stage_bits(stage),
        fresh(&context.view.terrain, roadbed, &core.allocator, stage),
        "{label}: the set is its pose's exact bake"
    );
    // TERRAIN-12 (T60, T61 (1) (a)): the renderer draws the latest pose's whole set or nothing,
    // never a part of it beside published tiles, nor an earlier pose's whose road lies elsewhere.
    if !displayed.is_empty() {
        assert_eq!(behind, 0, "{label}: only the latest pose's set is displayed");
        assert_eq!(
            stage_bits(displayed),
            stage_bits(stage),
            "{label}: the displayed tiles are the whole set"
        );
    }
    Arc::clone(stage)
}

// T65 option (c) (`ROAD-112` round G2), the Rust-only replay: on every drag of the junction
// fixtures, flat and sloped, a frame of a pose that replaces published pieces draws the pose's
// pieces alone (the drawn roadbed holds each replaced owner's rebuilt piece, never the published
// one: one road version), and every point of the published cut the drawn road leaves open (no
// drawn road there, or a drawn top other than the published one, which lets the background show
// past the cut's edge) lies in the frame's backstop tiles, whose cut ground draws behind every
// drawn surface. The poses and their catch-up frames, sampled every 0.25 m.
#[test]
fn a_pose_replacing_published_pieces_leaves_no_open_cut_outside_its_backstop() {
    let mut replacing = 0;
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
            let [start, end] = case.drag;
            let poses: Vec<_> = [0.4, 0.6, 0.8, 1.0]
                .map(|u| stroke(&core, [start, start.lerp(end, u)]))
                .to_vec();
            let mut last = None;
            for (index, points) in poses.iter().enumerate() {
                let pose = preview(&mut context, points);
                let label = format!("{name} pose {index}");
                let pieces = pose.pieces.as_ref().unwrap_or_else(|| panic!("{label}: no pieces"));
                for (id, rebuilt) in pieces.delta.pieces() {
                    let drawn = pieces.roadbed.piece_mesh(id);
                    assert!(
                        match (drawn, rebuilt) {
                            (Some(drawn), Some(rebuilt)) => std::ptr::eq(drawn, rebuilt),
                            (drawn, rebuilt) => drawn.is_none() && rebuilt.is_none(),
                        },
                        "{label}: owner {id:?} draws a piece other than its rebuilt one"
                    );
                }
                let frame = cut_frame(&mut context, &pose, 0.25).expect("the pose's pieces");
                assert_eq!(frame.open_outside, 0, "{label}: {frame:?}");
                replacing += usize::from(frame.replaced > 0);
                last = Some(pose);
            }
            let last = last.expect("a pose");
            assert!(converge_graded(&mut context).is_some(), "{name}: the last set lands");
            let frame = cut_frame(&mut context, &last, 0.25).expect("the pose's pieces");
            assert_eq!(frame.open_outside, 0, "{name} converged: {frame:?}");
        }
    }
    assert!(replacing > 0, "no pose replaced a published piece");
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
            // The drag: the stroke grows from its start towards its end, a frame a pose, each
            // frame after the set in flight landed (the synchronous path), then the catch-up
            // frame that lands the last pose's set.
            let [start, end] = case.drag;
            let poses: Vec<_> = [0.4, 0.6, 0.8, 1.0]
                .map(|u| stroke(&core, [start, start.lerp(end, u)]))
                .to_vec();
            let mut roadbeds = Vec::new();
            let mut last = None;
            for (index, points) in poses.iter().enumerate() {
                context.graded.wait();
                let pose = preview(&mut context, points);
                let pieces = pose.pieces.as_ref().unwrap_or_else(|| {
                    panic!("{name} pose {index}: {}", pose.validation.invalid_reason)
                });
                let label = format!("{name} pose {index}");
                roadbeds.push(pieces.roadbed.snapshot());
                check_frame(&label, &context, &core, &pieces.graded, &roadbeds);
                let behind = (index > 0).then_some(1);
                assert_eq!(pieces.graded.behind, behind, "{label}: the set before shown");
                assert!(context.graded.displayed().is_empty(), "{label}: none under the road");
                assert_eq!(pieces.graded.pending, 1, "{label}: the pose's set in flight");
                last = Some(pose);
            }
            let mut pose = last.unwrap();
            let pieces = pose.pieces.as_mut().unwrap();
            context.graded.wait();
            let graded = context.catch_up_graded().expect("the last pose's set lands");
            let label = format!("{name} catch-up frame");
            let shown = check_frame(&label, &context, &core, &graded, &roadbeds);
            assert_eq!((graded.behind, graded.pending), (Some(0), 0), "{name}: converged");
            assert!(!shown.is_empty(), "{name}: the pose grades tiles");
            assert_eq!(
                stage_bits(context.graded.displayed()),
                stage_bits(&shown),
                "{name}: the converged set is displayed"
            );
            pieces.graded = graded;
            assert_eq!(
                stage_bits(&shown),
                fresh(
                    &context.view.terrain,
                    &pieces.roadbed,
                    &core.allocator,
                    &shown
                ),
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

// TERRAIN-12 (T60), `TERRAIN-14`: a stroke swung sideways after it converged hands the renderer
// only the latest pose's whole set (checked in check_frame): nothing under a moved road, the
// pose's set once it lands; a set adopts every tile of the set before it whose inputs the move
// left unchanged; a pose that draws no road hands every displayed tile back.
#[test]
fn a_swung_preview_displays_only_exact_tiles_and_an_invalid_pose_none() {
    let case = &cases()[0];
    let mut core = committed_on(case.world_m, &case.roads, &[0], true);
    core.benchmark_mode = false;
    core.refresh_road_terrain_inputs();
    let mut context =
        RoadPreviewContext::capture(road_tool_snapshot_from_core(&core).expect("current roadbed"));
    let start = case.drag[0];
    let mut roadbeds = Vec::new();
    let mut renderer = std::collections::BTreeMap::new();
    let mut changes = crate::nodes::sim::core::ShownChanges::default();
    let mut draw = |context: &mut RoadPreviewContext| {
        context.graded.take_shown_changes(&mut changes);
        for tile in &changes.removed {
            renderer.remove(&(tile.x, tile.z));
        }
        // `TERRAIN-14` commit 2: the set's job packed each tile as the frame packed it before.
        let epoch = i64::try_from(context.graded.epoch()).unwrap();
        let cell_m = f64::from(context.view.terrain.cell_size_m());
        for &index in &changes.changed {
            let tile = &changes.shown.stage.tiles()[index];
            let fresh = PackedTile::pack(tile, epoch, cell_m, Default::default());
            assert_eq!(changes.shown.packed[index].bytes(), fresh.bytes(), "the job's pack");
            renderer.insert((tile.tile.x, tile.tile.z), tile.clone());
        }
        renderer.len()
    };
    for (index, x) in [0.0f32, 12.0, 24.0, 24.0].into_iter().enumerate() {
        let end = Vector2::new(x, case.drag[1].y);
        let pose = preview(&mut context, &stroke(&core, [start, end]));
        let pieces = pose.pieces.as_ref().expect("the swing's poses are valid");
        let label = format!("swing pose {index}");
        roadbeds.push(pieces.roadbed.snapshot());
        check_frame(&label, &context, &core, &pieces.graded, &roadbeds);
        assert_eq!(draw(&mut context), 0, "{label}: nothing under the moved road");
        let graded = converge_graded(&mut context).expect("the pose's set lands");
        let label = format!("swing pose {index} landed");
        let shown = check_frame(&label, &context, &core, &graded, &roadbeds);
        assert_eq!(graded.behind, Some(0), "{label}");
        assert_eq!(draw(&mut context), shown.len(), "{label}: the whole set drawn");
        if index == 3 {
            assert_eq!(
                (graded.stats.adopted, graded.stats.baked),
                (shown.len(), 0),
                "{label}: the repeated pose adopts every tile of the set before it"
            );
        }
    }
    let drawn = draw(&mut context);
    assert!(drawn > 0, "the converged pose's tiles are drawn");
    // A pose that compiles nothing (a stroke too short to plan) draws no road: no preview terrain.
    let pose = preview(&mut context, &stroke(&core, [start, start]));
    assert!(pose.pieces.is_none(), "the degenerate stroke has no pieces");
    let mut changes = crate::nodes::sim::core::ShownChanges::default();
    context.graded.take_shown_changes(&mut changes);
    assert!(changes.changed.is_empty(), "nothing new is shown");
    assert_eq!(changes.removed.len(), drawn, "every drawn tile is handed back");
    assert!(context.graded.displayed().is_empty());
}

// `TERRAIN-14`: a set of a pose that a release or a pose without a drawn road invalidated is
// dropped as it lands and never displays; the set it would have replaced stays the adoption pool.
#[test]
fn a_set_of_an_invalidated_pose_never_displays() {
    let case = &cases()[0];
    let core = committed_on(case.world_m, &case.roads, &[0], true);
    let [start, end] = case.drag;
    let snapshot = |core: &mut SimCore| {
        core.refresh_road_terrain_inputs();
        RoadPreviewContext::capture(road_tool_snapshot_from_core(core).expect("current roadbed"))
    };
    let mut core = core;
    for release in [false, true] {
        let mut context = snapshot(&mut core);
        context.graded.defer_jobs();
        let pose = preview(&mut context, &stroke(&core, [start, end]));
        assert_eq!(pose.pieces.as_ref().unwrap().graded.pending, 1);
        if release {
            context.graded.release();
        } else {
            let none = preview(&mut context, &stroke(&core, [start, start]));
            assert!(none.pieces.is_none(), "the degenerate stroke has no pieces");
        }
        assert_eq!(context.graded.run_deferred(), 1, "release {release}: the set ran");
        assert!(context.catch_up_graded().is_none(), "release {release}: dropped as it lands");
        assert_eq!(context.graded.pending_sets(), 0, "release {release}");
        assert!(context.graded.displayed().is_empty(), "release {release}");
        let mut changes = crate::nodes::sim::core::ShownChanges::default();
        context.graded.take_shown_changes(&mut changes);
        assert!(changes.changed.is_empty() && changes.removed.is_empty(), "release {release}");
    }
}

// `TERRAIN-14`: newest wins. While a set is in flight a newer pose replaces the queued one, which
// never starts a job; the set in flight lands (behind, not displayed under the newer road), then
// the newest pose's set runs and displays.
#[test]
fn a_queued_pose_replaced_by_a_newer_one_starts_no_job() {
    let case = &cases()[0];
    let mut core = committed_on(case.world_m, &case.roads, &[0], true);
    core.refresh_road_terrain_inputs();
    let mut context =
        RoadPreviewContext::capture(road_tool_snapshot_from_core(&core).expect("current roadbed"));
    context.graded.defer_jobs();
    let [start, end] = case.drag;
    let mut roadbeds = Vec::new();
    for u in [0.5, 0.75, 1.0] {
        let pose = preview(&mut context, &stroke(&core, [start, start.lerp(end, u)]));
        roadbeds.push(pose.pieces.as_ref().expect("the pose's pieces").roadbed.snapshot());
    }
    assert_eq!(context.graded.pending_sets(), 2, "one set in flight, one pose queued");
    assert_eq!(context.graded.run_deferred(), 1, "the first pose's set");
    let first = context.catch_up_graded().expect("the first set lands");
    check_frame("first", &context, &core, &first, &roadbeds);
    assert_eq!(first.behind, Some(2), "the first pose's set, two poses behind");
    assert!(context.graded.displayed().is_empty(), "not under the newest road");
    assert_eq!(context.graded.run_deferred(), 1, "only the newest pose's set runs");
    let newest = context.catch_up_graded().expect("the newest set lands");
    let shown = check_frame("newest", &context, &core, &newest, &roadbeds);
    assert_eq!((newest.behind, newest.pending), (Some(0), 0));
    assert_eq!(stage_bits(context.graded.displayed()), stage_bits(&shown));
    assert_eq!(context.graded.run_deferred(), 0, "no job for the replaced pose");
}

// T27 (d), `TERRAIN-14`: a click right after a fast drag, before the last pose's set has landed,
// commits the tiles a click after the converged drag commits, bitwise: the commit adopts the
// landed set's tiles whose inputs match and bakes the rest in its own frame. On the synchronous
// path the two drags' pose frames show the same sets.
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
            assert!(
                pending > 0 && baked > 0,
                "the fast click bakes the unstaged tiles"
            );
        }
        runs.push((pose_digests, stage_bits(&tiles)));
    }
    assert_eq!(runs[0].0, runs[1].0, "the pose frames show the same tiles");
    assert_eq!(
        runs[0].1, runs[1].1,
        "the fast click commits the converged click's tiles"
    );
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

// `ROAD-61` round R8e: a tunnel stroke through the 1:2 hill, its approaches in walled cuts. The
// preview's converged tiles (with the trough's walls), the commit, the undo and a recommit each
// equal the fresh bake of the state they stand for, bitwise. (The test hill's heightmap is set
// without a world config, so its save does not load; the load is the tunnel-free cases'.)
#[test]
fn tunnel_tiles_of_a_preview_commit_undo_and_load_equal_fresh_bakes() {
    use super::road_tunnels::{THROUGH_HILL, hill_core};
    let mut core = hill_core(80.0);
    core.refresh_road_terrain_inputs();
    let points = THROUGH_HILL.to_vec();
    let mut context =
        RoadPreviewContext::capture(road_tool_snapshot_from_core(&core).expect("current roadbed"));
    let request = |context: &RoadPreviewContext| RoadPreviewRequest {
        surface_generation: context.view.surface_generation,
        points: points.clone(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: false,
        profile: crate::simulation::network::road_edit::RoadProfile::Tunnel,
        median: None,
        parking: crate::simulation::network::graph::RoadParking::NONE,
        attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
    };
    let mut pose = context.preview_road(request(&context));
    let pieces = pose.pieces.as_mut().expect("the tunnel's pieces");
    pieces.graded = converge_graded(&mut context).expect("the pose's set lands");
    assert_eq!(pieces.graded.pending, 0, "converged");
    let shown = Arc::clone(pieces.graded.stage.as_ref().expect("the pose's tiles bake"));
    assert!(
        shown.tiles().iter().any(|held| !held.payload.walls.is_empty()),
        "the trough's walls are in the pose's tiles"
    );
    assert_eq!(
        stage_bits(&shown),
        fresh(&context.view.terrain, &pieces.roadbed, &core.allocator, &shown),
        "the pose's tiles are the fresh bake"
    );
    let before = live_fresh(&mut core, &shown);
    let commit = |core: &mut SimCore, preview: Option<Arc<RoadPreview>>| {
        let outcome = core.commit_road(
            RoadCommitRequest {
                points: points.clone(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: false,
                profile: crate::simulation::network::road_edit::RoadProfile::Tunnel,
                median: None,
                parking: crate::simulation::network::graph::RoadParking::NONE,
                attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
                preview,
            },
            std::time::Instant::now(),
            Default::default(),
        );
        assert!(outcome.committed, "{}", outcome.rejection);
    };
    commit(&mut core, Some(Arc::new(pose)));
    let committed = last_tiles(&core);
    assert_eq!(stage_bits(&committed), stage_bits(&shown), "commit = pose");
    assert_eq!(
        stage_bits(&committed),
        live_fresh(&mut core, &committed),
        "the commit's tiles are the fresh bake of the committed city"
    );
    assert!(core.undo_action_internal(), "undo");
    let undone = last_tiles(&core);
    assert_eq!(stage_bits(&undone), before, "undo restores the tiles");
    assert_eq!(stage_bits(&undone), live_fresh(&mut core, &undone));
    commit(&mut core, None);
    let recommitted = last_tiles(&core);
    assert_eq!(stage_bits(&recommitted), stage_bits(&committed), "recommit = commit");
}
