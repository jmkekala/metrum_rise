// SPDX-License-Identifier: GPL-3.0-or-later

//! `TERRAIN-28`: every terrain brush edit clamps its footprint to the world and refuses one
//! wholly outside it. An edit whose footprint reaches the world writes exactly the samples it
//! wrote before the clamp (the digest below was recorded on `dev` 6e312470, before it).

use super::*;
use godot::prelude::Vector2;

// `RENDER-32-Q`'s fixture world: 1,020 m square, 10 m samples (103 x 103), 510 m chunks.
const WORLD_M: f32 = 1020.0;

fn world_core() -> SimCore {
    let mut core = test_core();
    core.create_blank_world_internal(WORLD_M, WORLD_M, 10.0, 510.0, 0.0)
        .expect("blank world");
    core
}

// FNV-1a over the products a brush edit leaves: the source samples' bits, the dirty render
// patches with their payload generations, and the draped overlays' height revisions.
fn edit_digest(core: &SimCore) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut eat = |value: u64| {
        for byte in value.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let terrain = &core.heightmap;
    for z in 0..terrain.height {
        for x in 0..terrain.width {
            eat(u64::from(terrain.get_height(x, z).to_bits()));
        }
    }
    let mut states = core.terrain_dirty_patch_states();
    states.sort_unstable();
    for (patch_x, patch_z, generation) in states {
        eat(patch_x as u64);
        eat(patch_z as u64);
        eat(generation);
    }
    for chunk_z in -3..=3 {
        for chunk_x in -3..=3 {
            eat(terrain.height_revision_for_chunk((chunk_x, chunk_z)));
        }
    }
    eat(core.undo_stack.len() as u64);
    hash
}

// Every brush entry, single edits and stroke steps, at points whose footprint reaches the
// world: interior, over each edge and corner, and with a radius larger than the world.
fn run_reaching_edits(core: &mut SimCore) {
    let half = WORLD_M * 0.5;
    let at = |x: f32, z: f32| Vector2::new(x, z);
    core.sculpt_terrain_internal(at(0.0, 42.0), 26.0, 0.4);
    core.sculpt_terrain_internal(at(-half, -half), 24.0, 0.3);
    core.sculpt_terrain_internal(at(half + 10.0, 0.0), 24.0, -0.2);
    core.level_terrain_internal(at(half, half), 40.0, 3.0, 0.5);
    core.level_terrain_internal(at(-120.0, 75.0), 33.0, -2.0, 0.25);
    core.smooth_terrain_internal(at(0.0, 42.0), 30.0, 0.6);
    core.smooth_terrain_internal(at(-half - 5.0, half), 28.0, 0.6);
    core.slope_terrain_internal(
        at(200.0, -half),
        45.0,
        at(150.0, -half),
        1.0,
        at(260.0, -half + 20.0),
        6.0,
        0.4,
    );
    core.sculpt_terrain_internal(at(30.0, -40.0), 2.0 * WORLD_M, 0.05);

    core.start_terrain_stroke_internal();
    core.sculpt_terrain_stroke_step_internal(at(half - 3.0, -half + 3.0), 22.0, 0.3);
    core.level_terrain_stroke_step_internal(at(-half, 0.0), 30.0, 2.5, 0.4);
    core.smooth_terrain_stroke_step_internal(at(100.0, 100.0), 26.0, 0.5);
    core.slope_terrain_stroke_step_internal(
        at(-300.0, half),
        35.0,
        at(-340.0, half),
        0.0,
        at(-260.0, half - 30.0),
        4.0,
        0.5,
    );
    assert!(core.end_terrain_stroke_internal());
}

#[test]
fn edits_reaching_the_world_write_what_they_wrote_before_the_clamp() {
    let mut core = world_core();
    run_reaching_edits(&mut core);
    // Nine single edits and one stroke, none rolled back.
    assert_eq!(core.undo_stack.len(), 10);
    // Recorded on `dev` 6e312470, before the clamp (round T28).
    assert_eq!(edit_digest(&core), 2_240_631_579_743_229_371);
}

// Every brush entry at one point: the single edits, then a stroke of every step kind.
fn run_every_entry(core: &mut SimCore, pos: Vector2, radius: f32) {
    core.sculpt_terrain_internal(pos, radius, 0.3);
    core.level_terrain_internal(pos, radius, 2.0, 0.5);
    core.smooth_terrain_internal(pos, radius, 0.5);
    let (start, end) = (pos - Vector2::new(5.0, 0.0), pos + Vector2::new(5.0, 0.0));
    core.slope_terrain_internal(pos, radius, start, 0.0, end, 3.0, 0.5);
    core.start_terrain_stroke_internal();
    core.sculpt_terrain_stroke_step_internal(pos, radius, 0.3);
    core.level_terrain_stroke_step_internal(pos, radius, 2.0, 0.5);
    core.smooth_terrain_stroke_step_internal(pos, radius, 0.5);
    core.slope_terrain_stroke_step_internal(pos, radius, start, 0.0, end, 3.0, 0.5);
    core.end_terrain_stroke_internal();
}

// Points `out` metres beyond each corner and edge of the world.
fn around_the_world(out: f32) -> [Vector2; 8] {
    let edge = WORLD_M * 0.5 + out;
    [
        (-1, -1),
        (0, -1),
        (1, -1),
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
    ]
    .map(|(x, z)| Vector2::new(x as f32 * edge, z as f32 * edge))
}

#[test]
fn edits_wholly_outside_the_world_are_refused() {
    // `RENDER-32-Q`'s point among them: 510 m beyond a corner, radius 24 m.
    let points = around_the_world(WORLD_M * 0.5);
    let invalid = [
        (Vector2::new(f32::NAN, 0.0), 24.0),
        (Vector2::ZERO, f32::INFINITY),
        (Vector2::ZERO, 0.0),
        (Vector2::ZERO, -5.0),
        (Vector2::new(-1.0e30, 0.0), 24.0),
    ];
    let mut core = world_core();
    core.take_graded_edits();
    let before = edit_digest(&core);
    for (pos, radius) in points.map(|pos| (pos, 24.0)).into_iter().chain(invalid) {
        run_every_entry(&mut core, pos, radius);
        assert_eq!(
            edit_digest(&core),
            before,
            "({pos:?}, {radius}): nothing written"
        );
        assert!(
            core.undo_stack.is_empty(),
            "({pos:?}, {radius}): no undo record"
        );
        assert!(
            core.take_graded_edits().is_empty(),
            "({pos:?}, {radius}): no tiles"
        );
    }
}

// The graded edits' base rectangles lie inside the world grown by the render patches' margin,
// and the dirty render patches are the world's.
fn assert_marks_inside_the_world(core: &mut SimCore, label: &str) {
    let limit = f64::from(WORLD_M * 0.5 + core.heightmap.render_patch_border_margin_m());
    let edits = core.take_graded_edits();
    assert!(!edits.is_empty(), "{label}: the edit is recorded");
    for edit in edits {
        let crate::nodes::sim::core::graded_edits::GradedEditKind::Base(rect) = edit.kind else {
            panic!("{label}: a base edit");
        };
        assert!(
            rect.iter().all(|v| v.abs() <= limit),
            "{label}: {rect:?} past the world"
        );
    }
    let patches = core.heightmap.render_patch_cols() * core.heightmap.render_patch_rows();
    assert!(
        core.heightmap.dirty_render_patches().len() <= patches,
        "{label}"
    );
}

#[test]
fn edits_partly_outside_or_larger_than_the_world_are_clamped_to_it() {
    let reaching = around_the_world(15.0).map(|pos| (pos, 24.0));
    let larger = [
        (Vector2::new(30.0, -40.0), 2.0 * WORLD_M),
        (Vector2::new(-WORLD_M, 0.0), 1.0e7),
    ];
    for (pos, radius) in reaching.into_iter().chain(larger) {
        let mut core = world_core();
        core.take_graded_edits();
        let before = edit_digest(&core);
        core.sculpt_terrain_internal(pos, radius, 0.3);
        assert_ne!(
            edit_digest(&core),
            before,
            "({pos:?}, {radius}): the world is edited"
        );
        assert_marks_inside_the_world(&mut core, &format!("({pos:?}, {radius})"));
    }
}
