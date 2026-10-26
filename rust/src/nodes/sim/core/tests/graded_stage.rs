// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-E` acceptance, the Rust half: the graded tiles of every drag pose, the commit, the
//! undo, a recommit of the same stroke and a saved and loaded city each equal the fresh bake of
//! the state they stand for, bitwise, on the T, the Y, the four-way and the split trunk, flat and
//! on the 2.5% slope; the commit adopts the last pose's tiles, undo adopts the kept ones, and a
//! commit with a stale context bakes and equals a fresh bake. (A growing stroke rebuilds its
//! whole span each pose, so these drags' poses rebake every tile they select:
//! `docs/terrain.md` T27.)

use super::road_edit_order::{ARM_M, Stroke, committed, committed_on};
use super::*;
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
            // The drag: the stroke grows from its start towards its end.
            let [start, end] = case.drag;
            let mut last = None;
            for u in [0.4, 0.6, 0.8, 1.0] {
                let points = stroke(&core, [start, start.lerp(end, u)]);
                let pose = preview(&mut context, &points);
                let pieces = pose
                    .pieces
                    .as_ref()
                    .unwrap_or_else(|| panic!("{name} u {u}: {}", pose.validation.invalid_reason));
                let stage = pieces.graded.stage.as_ref().expect("the pose's tiles bake");
                assert!(!stage.is_empty(), "{name} u {u}: the pose grades tiles");
                assert_eq!(
                    stage_bits(stage),
                    fresh(
                        &context.view.terrain,
                        &pieces.roadbed,
                        &core.allocator,
                        stage
                    ),
                    "{name} u {u}: the pose's tiles are the fresh bake of the pose"
                );
                let stats = pieces.graded.stats;
                assert_eq!(stats.adopted + stats.baked, stage.len(), "{name} u {u}");
                last = Some((points, Arc::new(pose)));
            }
            let (points, pose) = last.unwrap();
            let posed = Arc::clone(pose.pieces.as_ref().unwrap().graded.stage.as_ref().unwrap());

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
