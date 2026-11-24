// SPDX-License-Identifier: GPL-3.0-or-later

//! Terrain and water undo records: what a restore puts back, bit for bit.

use crate::nodes::sim::core::{CityTreasury, SimCore};
use crate::simulation::core::config::WorldConfig;
use godot::prelude::Vector2;

// A 2,000 m world at 10 m: 201 samples a side in 52-sample storage chunks (4 x 4), whose
// borders fall at x or z = -480, 40 and 560 m.
fn terrain_core() -> SimCore {
    SimCore {
        treasury: CityTreasury::new(0.0),
        terrain_dirty: false,
        water_dirty: false,
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::new(2_000.0, 2_000.0, 40.0, 10.0))
    }
}

// The heights and water depths as bits.
#[derive(Clone, Debug, PartialEq)]
struct Grids {
    heights: Vec<u32>,
    water: Vec<u32>,
}

fn grids(core: &SimCore) -> Grids {
    let bits = |dense: Vec<f32>| dense.into_iter().map(f32::to_bits).collect();
    Grids {
        heights: bits(core.heightmap.clone_source_dense()),
        water: bits(core.watermap.clone_baseline_depth_dense()),
    }
}

fn stroke(core: &mut SimCore, points: &[(f32, f32)], strength: f32) {
    core.start_terrain_stroke_internal();
    for &(x, z) in points {
        core.sculpt_terrain_stroke_step_internal(Vector2::new(x, z), 90.0, strength);
    }
    assert!(core.end_terrain_stroke_internal());
}

fn slope(core: &mut SimCore) {
    core.slope_terrain_internal(
        Vector2::new(560.0, -480.0),
        110.0,
        Vector2::new(500.0, -520.0),
        5.0,
        Vector2::new(620.0, -440.0),
        45.0,
        0.5,
    );
}

// Water on every seventh sample.
fn set_water(core: &mut SimCore, depth: f32) {
    let len = core.watermap.width * core.watermap.height;
    let dense: Vec<f32> = (0..len)
        .map(|index| if index % 7 == 0 { depth } else { 0.0 })
        .collect();
    core.watermap
        .replace_baseline_depth_from_dense(&dense)
        .unwrap();
}

// Several brushes, edits across storage-chunk borders, a water record, undo past the first
// record, and an undone brush applied again after a new edit: every undo puts back the grids
// as they stood before its record, bit for bit.
#[test]
fn terrain_and_water_undo_restore_the_recorded_grids_bitwise() {
    let mut core = terrain_core();
    // The grids before each record, in push order.
    let mut before = vec![grids(&core)];
    stroke(&mut core, &[(20.0, 20.0), (60.0, 30.0), (100.0, 60.0)], 0.4);
    before.push(grids(&core));
    core.smooth_terrain_internal(Vector2::new(40.0, 40.0), 120.0, 0.3);
    before.push(grids(&core));
    core.level_terrain_internal(Vector2::new(-480.0, 40.0), 100.0, 30.0, 0.2);
    before.push(grids(&core));
    slope(&mut core);
    set_water(&mut core, 1.5);
    let water_record = grids(&core);
    core.push_undo_state(true, true, true);
    set_water(&mut core, 2.25);
    let sculpt_record = grids(&core);
    core.sculpt_terrain_internal(Vector2::new(40.0, 560.0), 70.0, -0.3);
    assert_eq!(core.undo_stack.len(), 6);
    // Every edit changed the grids, so no restore below holds by accident.
    assert!(before.windows(2).all(|pair| pair[0] != pair[1]));
    assert_ne!(sculpt_record, grids(&core));

    // The sculpt, the water record, the slope.
    assert!(core.undo_action_internal());
    assert_eq!(grids(&core), sculpt_record);
    assert!(core.undo_action_internal());
    assert_eq!(grids(&core), water_record);
    assert!(core.undo_action_internal());
    let water = water_record.water.clone();
    let with_water = |grids: &Grids| Grids {
        heights: grids.heights.clone(),
        water: water.clone(),
    };
    assert_eq!(grids(&core), with_water(&before[3]));

    // A new stroke across a border, then the slope again: both undo to where they began.
    stroke(&mut core, &[(40.0, -480.0), (40.0, -440.0)], -0.25);
    let after_new = grids(&core);
    slope(&mut core);
    assert!(core.undo_action_internal());
    assert_eq!(grids(&core), after_new);
    assert!(core.undo_action_internal());
    assert_eq!(grids(&core), with_water(&before[3]));

    // The level, the smooth and the first stroke, then nothing is left to undo.
    for state in before[..3].iter().rev() {
        assert!(core.undo_action_internal());
        assert_eq!(grids(&core), with_water(state));
    }
    assert!(!core.undo_action_internal());
    assert_eq!(grids(&core), with_water(&before[0]));
}

// A record shares every chunk a later edit does not write, by pointer, and the edit does not
// write through it: the dense record held the whole grid (`width * height` samples).
#[test]
fn terrain_and_water_records_share_the_chunks_later_edits_leave() {
    let mut core = terrain_core();
    // Uneven ground everywhere, so every chunk is stored.
    let (width, height) = (core.heightmap.width, core.heightmap.height);
    let uneven: Vec<f32> = (0..width * height)
        .map(|index| 0.5 + (index % 5) as f32 * 0.1)
        .collect();
    core.heightmap.replace_source_from_dense(&uneven).unwrap();
    let stored = 16;

    // A small sculpt inside the first chunk (samples 27..=33 of 0..52 on each axis).
    core.sculpt_terrain_internal(Vector2::new(-700.0, -700.0), 30.0, 0.2);
    let first = core.undo_stack[0].terrain.as_ref().unwrap();
    assert_eq!(
        first.clone_dense(),
        uneven,
        "the edit wrote no shared chunk"
    );
    let live = core.heightmap.source_snapshot();
    assert_eq!(first.shared_chunk_counts(&live), (stored - 1, stored));

    // A sculpt on the corner of four chunks copies those four alone.
    let after_first = core.heightmap.clone_source_dense();
    core.sculpt_terrain_internal(Vector2::new(40.0, 40.0), 30.0, 0.2);
    let [first, second] = [0, 1].map(|index| core.undo_stack[index].terrain.as_ref().unwrap());
    assert_eq!(second.clone_dense(), after_first);
    let live = core.heightmap.source_snapshot();
    assert_eq!(second.shared_chunk_counts(&live), (stored - 4, stored));
    assert_eq!(first.shared_chunk_counts(second), (stored - 1, stored));
    // Each record's own memory: the chunks no newer grid shares, 52 x 52 samples each, where
    // the dense record held 201 x 201.
    assert_eq!((width, height), (201, 201));

    // Water: the record is the live grid's `Arc` until the next write replaces it.
    set_water(&mut core, 1.5);
    let water = core.watermap.clone_baseline_depth_dense();
    core.push_undo_state(false, true, false);
    let record = &core.undo_stack[2].water.as_ref().unwrap().baseline_depth;
    assert!(std::sync::Arc::ptr_eq(
        record,
        &core.watermap.baseline_depth_snapshot()
    ));
    set_water(&mut core, 2.25);
    let record = &core.undo_stack[2].water.as_ref().unwrap().baseline_depth;
    assert!(!std::sync::Arc::ptr_eq(
        record,
        &core.watermap.baseline_depth_snapshot()
    ));
    assert_eq!(record.clone_dense(), water);
}
