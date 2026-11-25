// SPDX-License-Identifier: GPL-3.0-or-later

//! The in-place water update against the full solve (`TERRAIN-38`).

use crate::nodes::sim::core::{CityTreasury, SimCore};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::resources::ResourceDepositSystem;
use crate::simulation::world_definition::{
    AuthoredLakeFill, AuthoredOpenWaterFill, LoadedWorldDefinition,
};
use godot::prelude::Vector2;
use std::collections::{BTreeMap, BTreeSet};

// What a solve leaves: the depth bits, the stored chunk count and the debug cache (every field
// of every entry, printed round-trip exact).
type Products = (Vec<u32>, usize, BTreeMap<(usize, usize), String>);

fn products(core: &SimCore) -> Products {
    let depth = core
        .watermap
        .clone_baseline_depth_dense()
        .iter()
        .map(|depth| depth.to_bits())
        .collect();
    let chunks = core
        .watermap
        .baseline_depth_grid()
        .materialized_chunk_count();
    let debug = core
        .authored_water_patch_fill_debug_cache
        .iter()
        .map(|(key, contributors)| (*key, format!("{contributors:?}")))
        .collect();
    (depth, chunks, debug)
}

// A 1,280 m world at 8 m (161 x 161 samples, sample (g, h) at world (8 g - 640, 8 h - 640),
// 3 x 3 render patches) with:
// - a wide basin centred on (64, 80) with a pillar at its centre, holding a lake at 72 m seeded
//   on the pillar and a nested lake at 66 m;
// - a small basin at (136, 80) behind a strait (60 m) that the 72 m lake floods through, with
//   its own lake at 58 m and an open-water fill at 65 m that never reaches the edge;
// - a low corner on the world edge with an edge-escaping lake (30 m) and open water (18 m);
// - a basin at (24, 140) holding a lake at 45 m, behind a one-sample rim (x = 16) from a channel
//   to the west edge that holds open water at 35 m.
fn edit_world() -> SimCore {
    let config = WorldConfig::new(1280.0, 1280.0, 1280.0, 10.0).with_terrain_resolution(8.0);
    let mut core = SimCore {
        treasury: CityTreasury::new(0.0),
        terrain_dirty: false,
        water_dirty: false,
        benchmark_mode: true,
        ..SimCore::new(config)
    };
    let (width, height) = (core.heightmap.width, core.heightmap.height);
    for z in 0..height {
        for x in 0..width {
            let (fx, fz) = (x as f32, z as f32);
            let big = ((fx - 64.0).powi(2) + (fz - 80.0).powi(2)).sqrt();
            let small = ((fx - 136.0).powi(2) + (fz - 80.0).powi(2)).sqrt();
            let drain = ((fx - 24.0).powi(2) + (fz - 140.0).powi(2)).sqrt();
            let mut ground = 200.0f32;
            if big < 56.0 {
                ground = 40.0 + big * 0.6 + (fx * 0.37).sin() * 2.0;
            }
            if small < 12.0 {
                ground = ground.min(50.0 + small * 2.0);
            }
            if (fz - 80.0).abs() < 1.5 && (119.0..=125.0).contains(&fx) {
                ground = 60.0;
            }
            if (fx - 64.0).abs() < 2.0 && (fz - 80.0).abs() < 2.0 {
                ground = 70.0;
            }
            if x + z < 12 {
                ground = 10.0 + (x + z) as f32;
            }
            if drain < 8.0 {
                ground = 30.0 + drain;
            }
            if (139..=141).contains(&z) && x <= 15 {
                ground = 20.0;
            }
            core.heightmap.set_height(x, z, ground);
        }
    }
    let lake = |world_x, world_z, surface_elevation_m| AuthoredLakeFill {
        world_x,
        world_z,
        surface_elevation_m,
    };
    let open = |world_x, world_z, surface_elevation_m| AuthoredOpenWaterFill {
        world_x,
        world_z,
        surface_elevation_m,
    };
    let loaded = LoadedWorldDefinition {
        name: "terrain-38".to_owned(),
        config: core.config,
        terrain: core.heightmap.clone(),
        lake_fills: vec![
            lake(-128.0, 0.0, 72.0),
            lake(-208.0, 0.0, 66.0),
            lake(448.0, 0.0, 58.0),
            lake(-632.0, -632.0, 30.0),
            lake(-448.0, 480.0, 45.0),
        ],
        open_water_fills: vec![
            open(-632.0, -632.0, 18.0),
            open(448.0, 0.0, 65.0),
            open(-624.0, 480.0, 35.0),
        ],
        resource_deposits: ResourceDepositSystem::from_world_config(&core.config),
    };
    core.apply_loaded_world_definition(
        loaded,
        crate::simulation::vegetation::VegetationConfig::default(),
    );
    core
}

#[derive(Clone, Copy, Debug)]
enum Edit {
    Sculpt(f32, f32, f32, f32),
    Stroke([(f32, f32, f32, f32); 3]),
    Undo,
}

fn apply(core: &mut SimCore, edit: Edit) {
    match edit {
        Edit::Sculpt(x, z, radius, strength) => {
            core.sculpt_terrain_internal(Vector2::new(x, z), radius, strength)
        }
        Edit::Stroke(steps) => {
            core.start_terrain_stroke_internal();
            for (x, z, radius, strength) in steps {
                core.sculpt_terrain_stroke_step_internal(Vector2::new(x, z), radius, strength);
            }
            assert!(core.end_terrain_stroke_internal());
        }
        Edit::Undo => assert!(core.undo_action_internal(), "an edit to undo"),
    }
}

// The scripted edits, then a deterministic pseudo-random walk of sculpts and undos.
fn edits() -> Vec<(&'static str, Edit)> {
    let mut edits = vec![
        ("shore cut", Edit::Sculpt(-128.0, -432.0, 24.0, -6.0)),
        (
            "raise splits the strait",
            Edit::Sculpt(336.0, 0.0, 16.0, 30.0),
        ),
        (
            "lower joins the basins",
            Edit::Sculpt(336.0, 0.0, 16.0, -45.0),
        ),
        ("far from water", Edit::Sculpt(560.0, -560.0, 30.0, 5.0)),
        (
            "seed raised above its level",
            Edit::Sculpt(-208.0, 0.0, 4.0, 40.0),
        ),
        ("seed lowered", Edit::Sculpt(-208.0, 0.0, 4.0, -40.0)),
        (
            "rim cut drains to the edge",
            Edit::Sculpt(-512.0, 480.0, 4.0, -190.0),
        ),
        ("undo the drain", Edit::Undo),
        ("undo the seed lowering", Edit::Undo),
        (
            "stroke along a shore",
            Edit::Stroke([
                (-128.0, 432.0, 20.0, -4.0),
                (-96.0, 432.0, 20.0, -4.0),
                (-64.0, 424.0, 20.0, -4.0),
            ]),
        ),
        (
            "island over the big lake's seed",
            Edit::Sculpt(-128.0, 0.0, 32.0, 25.0),
        ),
        ("undo the island", Edit::Undo),
    ];
    // A linear congruential walk: positions over the wet half of the world, radii 6-40 m,
    // strengths -12..12 m, an undo every seventh edit.
    let mut state = 0x2545_f491_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) as f32) / (1u64 << 31) as f32
    };
    for step in 0..48 {
        if step % 7 == 6 {
            edits.push(("random undo", Edit::Undo));
            continue;
        }
        let x = -640.0 + next() * 1280.0;
        let z = -560.0 + next() * 1120.0;
        let radius = 6.0 + next() * 34.0;
        let strength = -12.0 + next() * 24.0;
        edits.push(("random sculpt", Edit::Sculpt(x, z, radius, strength)));
    }
    edits
}

// Per kept fill (lakes, then open water): accepted, and the samples it flooded.
fn fill_report(core: &SimCore) -> Vec<(bool, usize)> {
    let book = core.authored_water_book.as_ref().expect("a kept book");
    book.fills
        .iter()
        .map(|fill| (fill.accepted, fill.application.filled_cells))
        .collect()
}

// What the scripted edits must do to the fills (`before`, `after`: [`fill_report`]); lakes 0-4,
// then open water 5-7.
fn check_scripted_outcome(name: &str, before: &[(bool, usize)], after: &[(bool, usize)]) {
    let fills = |index: usize| (before[index], after[index]);
    match name {
        "raise splits the strait" => {
            let ((_, joined), (accepted, split)) = fills(0);
            assert!(
                accepted && split < joined,
                "the 72 m lake loses the small basin"
            );
            assert!(fills(2).1.0, "the small basin keeps its own lake");
        }
        "lower joins the basins" => {
            let ((_, split), (accepted, joined)) = fills(0);
            assert!(
                accepted && joined > split,
                "the 72 m lake floods the small basin again"
            );
        }
        "far from water" => assert_eq!(before, after),
        "seed raised above its level" => {
            assert_eq!(fills(1).1, (true, 0), "the 66 m lake is empty")
        }
        "rim cut drains to the edge" => {
            assert!(
                fills(4).0.0 && !fills(4).1.0,
                "the 45 m lake spills to the edge"
            );
            let ((_, channel), (accepted, joined)) = fills(7);
            assert!(
                accepted && joined > channel,
                "the open water joins the basin"
            );
        }
        "island over the big lake's seed" => assert_eq!(fills(0).1, (true, 0)),
        _ => {}
    }
}

// The render patches whose texture (owned samples and the one-texel border ring) holds a
// sample whose depth bits differ between `before` and `after`.
fn patches_reading_changes(
    core: &SimCore,
    before: &[u32],
    after: &[u32],
) -> BTreeSet<(usize, usize)> {
    let water = &core.watermap;
    let (width, height) = (water.width, water.height);
    let mut patches = BTreeSet::new();
    for patch_z in 0..water.render_patch_rows() {
        for patch_x in 0..water.render_patch_cols() {
            let (x0, x1, z0, z1) = water
                .render_patch_owned_sample_bounds(patch_x, patch_z)
                .unwrap();
            let changed = (z0.saturating_sub(1)..=(z1 + 1).min(height - 1)).any(|z| {
                (x0.saturating_sub(1)..=(x1 + 1).min(width - 1))
                    .any(|x| before[z * width + x] != after[z * width + x])
            });
            if changed {
                patches.insert((patch_x, patch_z));
            }
        }
    }
    patches
}

// Runs every edit on a core that updates its water in place and on a twin that is re-solved in
// full after each, and checks after every edit: the products are equal; the in-place core
// marked dirty exactly the patches whose texture reads a changed sample, under one new render
// generation, and nothing when the water did not change. Returns one digest of the products a
// step.
fn run_edits_against_full_solves() -> Vec<u64> {
    let mut live = edit_world();
    let mut twin = edit_world();
    assert_eq!(products(&live), products(&twin));
    assert!(live.authored_water_book.is_some(), "the load keeps a book");
    let mut digests = Vec::new();
    let mut changed_steps = 0;
    for (step, (name, edit)) in edits().into_iter().enumerate() {
        for (patch_x, patch_z, generation) in live.watermap.dirty_render_patch_states() {
            live.watermap
                .acknowledge_render_patch(patch_x, patch_z, generation);
        }
        let before = products(&live);
        let fills_before = fill_report(&live);
        let generation = live.watermap.render_generation();
        apply(&mut live, edit);
        apply(&mut twin, edit);
        twin.rebuild_authored_water_preview_internal()
            .expect("full solve");
        let after = products(&live);
        assert!(
            after == products(&twin),
            "step {step} ({name}, {edit:?}): the in-place water differs from a full solve"
        );
        let book = live.authored_water_book.as_ref().expect("book kept");
        let samples = live.watermap.width * live.watermap.height;
        assert_eq!(
            book.flood.held_bytes(),
            samples.div_ceil(64) * 8,
            "step {step}: between updates the book holds the visit bits, not the work lists"
        );
        assert!(
            book.held_bytes() < samples * 4,
            "step {step}: the book is smaller than one dense terrain copy"
        );
        check_scripted_outcome(name, &fills_before, &fill_report(&live));
        let dirty: BTreeSet<_> = live
            .watermap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        assert_eq!(
            dirty,
            patches_reading_changes(&live, &before.0, &after.0),
            "step {step} ({name}): dirty patches"
        );
        if before.0 == after.0 {
            assert_eq!(live.watermap.render_generation(), generation, "step {step}");
        } else {
            changed_steps += 1;
            assert_eq!(
                live.watermap.render_generation(),
                generation + 1,
                "step {step}"
            );
            assert!(
                dirty
                    .iter()
                    .all(|&(x, z)| live.watermap.render_patch_generation(x, z) == generation + 1)
            );
        }
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in format!("{after:?}").bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
        digests.push(hash);
    }
    assert!(
        changed_steps >= 25,
        "most edits must move the water: {changed_steps}"
    );
    let book = live.authored_water_book.as_ref().unwrap();
    assert!(
        book.local_updates >= 20 && book.refloods >= 5,
        "both paths must run: {} local updates, {} refloods",
        book.local_updates,
        book.refloods
    );
    digests
}

// `TERRAIN-38`: a terrain edit updates the water in place, bit for bit equal to a full solve,
// over shore cuts, a raise that splits a fill and a lowering that joins two, edits far from
// water, on a fill's seed, a rim cut that drains a lake to the world edge (and joins open water
// to its basin), a stroke, undos and a pseudo-random walk of sculpts.
#[test]
fn in_place_water_updates_equal_a_full_solve_after_every_edit() {
    run_edits_against_full_solves();
}

// The update is serial and the solve's parallel parts are order-independent: the same edits
// give the same bytes on one worker and on four.
#[test]
fn in_place_water_updates_do_not_depend_on_the_worker_count() {
    let run = |threads| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("test pool")
            .install(run_edits_against_full_solves)
    };
    assert_eq!(run(1), run(4));
}
