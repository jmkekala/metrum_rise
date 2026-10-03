// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic edit, persistence, patch ownership and bounded-cost regressions.

use super::*;
use crate::simulation::buildings::allocator::yard::YardEvent;
use crate::simulation::vegetation::{VegetationConfig, VegetationGenerator};
use std::collections::HashSet;

fn core() -> SimCore {
    super::super::tests::vegetation_test_core(VegetationConfig::default())
}

fn scatter(core: &SimCore, canopy: bool) -> Vec<f32> {
    let layer = if canopy {
        VegetationLayer::Canopy
    } else {
        VegetationLayer::Understory
    };
    let (cell_m, _) = grid(core, layer);
    let origin = Vector2::new(-255.0, -255.0);
    // Relative to the patch, which is what every assertion here was written against. The
    // f32 subtraction is the one the packing itself did before it packed world positions.
    let mut records = scatter_layer(
        core,
        origin,
        510.0,
        cell_m,
        if canopy { CANOPY_SALT } else { UNDERSTORY_SALT },
        canopy,
    );
    for record in records.chunks_exact_mut(6) {
        record[0] -= origin.x;
        record[2] -= origin.y;
    }
    records
}

fn first_candidate(core: &SimCore, accepted: bool) -> Plant {
    let (cell_m, salt) = grid(core, VegetationLayer::Canopy);
    disc_cells(Vector2::ZERO, 128.0, cell_m, VegetationLayer::Canopy)
        .find_first(|cell| evaluate_cell(core, *cell, cell_m, salt).is_some() == accepted)
        .map(|cell| {
            // An accepted cell reports the species the generator picked, so a caller can paint
            // that same species back. A rejected cell holds no plant and has no species.
            evaluate_cell(core, cell, cell_m, salt).unwrap_or_else(|| {
                let [x, z, yaw, scale] = candidate(cell.x, cell.z, cell_m, salt);
                Plant {
                    x,
                    z,
                    yaw,
                    scale,
                    species: 0,
                    variant: VARIANT_FROM_SEED,
                }
            })
        })
        .unwrap()
}

#[test]
fn bulldoze_plant_lookup_is_bounded_and_removes_one_coincident_plant() {
    let mut core = core();
    core.vegetation.config.enabled = false;
    assert!(add_at(&mut core, Vector2::new(1.0, 1.0), 2));
    assert!(add_at(&mut core, Vector2::new(3.0, 1.0), 3));
    let picked = plant_at(&core, Vector2::new(2.0, 1.0)).unwrap();
    assert_eq!(
        (picked.0.plant.x, picked.0.plant.z, picked.0.plant.species),
        (1.0, 1.0, 2)
    );
    assert!(picked.1 > 0.0);
    assert_eq!(plant_at(&core, Vector2::new(2.0, 1.0)), Some(picked));
    assert_eq!(plant_at(&core, Vector2::new(1.0, 1.0)), Some(picked));
    assert!(plant_at(&core, Vector2::new(1.0, 5.01)).is_none());
    assert!(plant_at(&core, Vector2::new(f32::NAN, 1.0)).is_none());
    assert!(add_at(&mut core, Vector2::new(1.0, 1.0), 0));
    assert_eq!(plant_at(&core, Vector2::new(1.0, 1.0)), Some(picked));
    assert!(remove_target(&mut core, picked.0));
    assert!(!remove_target(&mut core, picked.0));
    assert_eq!(
        plant_at(&core, Vector2::new(1.0, 1.0))
            .unwrap()
            .0
            .plant
            .species,
        0
    );
    assert!(core.undo_action_internal());
    // Area clear still removes both classes at the same position.
    assert_eq!(remove_at(&mut core, Vector2::new(1.0, 1.0), 0.0, 0), 2);
    assert_eq!(
        plant_at(&core, Vector2::new(3.0, 1.0))
            .unwrap()
            .0
            .plant
            .species,
        3
    );
}

#[test]
fn vegetation_removal_matches_scatter_disc_in_both_layers() {
    let mut core = core();
    assert!(add_at(&mut core, Vector2::ZERO, 0));
    let before = [scatter(&core, true), scatter(&core, false)];
    let pos = Vector2::new(9.0, -7.0);
    let radius = 63.0;
    let expected = before.each_ref().map(|records| {
        records
            .chunks_exact(6)
            .filter(|r| {
                Vector2::new(r[0] - 255.0, r[2] - 255.0).distance_squared_to(pos) > radius * radius
            })
            .flatten()
            .copied()
            .collect::<Vec<_>>()
    });
    let count = remove_at(&mut core, pos, radius, 0);
    assert!(count > 1);
    assert_eq!(
        count * 6,
        before.iter().map(Vec::len).sum::<usize>() - expected.iter().map(Vec::len).sum::<usize>()
    );
    assert_eq!([scatter(&core, true), scatter(&core, false)], expected);
    assert_eq!(remove_at(&mut core, pos, radius, 0), 0);
}

#[test]
fn vegetation_point_and_brush_respect_visible_generated_stems() {
    let mut core = core();
    let generated = first_candidate(&core, true);
    let pos = Vector2::new(generated.x, generated.z);
    assert!(!add_at(&mut core, pos + Vector2::new(1.0, 0.0), 1));
    assert!(add_at(&mut core, pos, 2));
    paint_at(&mut core, pos, 32.0, 4, 0, usize::MAX);
    for p in authored(&core).into_iter().filter(|p| p.species < 2) {
        assert!((p.x - generated.x).powi(2) + (p.z - generated.z).powi(2) >= 2.5 * 2.5);
    }
}

#[test]
fn vegetation_repaint_restores_generated_tree_and_prunes_tombstone() {
    let mut core = core();
    let before = scatter(&core, true);
    let p = first_candidate(&core, true);
    let pos = Vector2::new(p.x, p.z);
    assert_eq!(remove_at(&mut core, pos, 0.01, 0), 1);
    assert_eq!(core.vegetation_edits.len(), 1);
    // Painting the species that was cleared reproduces the identical plant, so the edit
    // collapses back to nothing rather than authoring a copy of what the generator already has.
    assert_eq!(paint_at(&mut core, pos, 0.01, i64::from(p.species), 0, usize::MAX), 1);
    assert_eq!(core.vegetation_edits.len(), 0);
    assert_eq!(scatter(&core, true), before);
}

#[test]
fn vegetation_repaint_with_another_species_does_not_regrow_the_cleared_plant() {
    let mut core = core();
    let p = first_candidate(&core, true);
    let pos = Vector2::new(p.x, p.z);
    assert_eq!(remove_at(&mut core, pos, 0.01, 0), 1);
    // The canopy grid carries every painted species, so a rock brush over cleared forest
    // must leave a rock standing there and no tree at all.
    let painted = SPECIES_ROCK as u8;
    assert_ne!(p.species, painted);
    assert_eq!(paint_at(&mut core, pos, 0.01, i64::from(painted), 0, usize::MAX), 1);
    let standing: Vec<_> = scatter(&core, true)
        .chunks_exact(6)
        .filter(|r| r[0] == p.x + 255.0 && r[2] == p.z + 255.0)
        .map(|r| r[5] as u8)
        .collect();
    assert_eq!(standing, vec![painted]);
    // The brush is idempotent: a second pass must not stack a second rock on the same cell.
    assert_eq!(paint_at(&mut core, pos, 0.01, i64::from(painted), 0, usize::MAX), 0);
}

#[test]
fn vegetation_authored_boundary_plant_belongs_to_exactly_one_patch_even_when_disabled() {
    let mut core = core();
    core.vegetation = VegetationGenerator::resolve(
        VegetationConfig {
            enabled: false,
            ..Default::default()
        },
        &core.config,
    );
    let left = Vector2::new(-124.25, -124.25);
    let right = Vector2::new(3.25, -124.25);
    assert!(add_at(&mut core, Vector2::new(3.25, -32.0), 2));
    let query = |origin| {
        scatter_layer(
            &core,
            origin,
            127.5,
            core.vegetation.canopy_cell_m,
            CANOPY_SALT,
            true,
        )
    };
    assert!(query(left).is_empty());
    let records = query(right);
    assert_eq!(records.len(), 6);
    assert_eq!(records[0], 3.25);
    assert_eq!(records[5], 2.0);
}

#[test]
fn vegetation_authored_plants_clear_under_a_surface_placed_later() {
    let mut core = core();
    // Generator off, so the patch payload is the two authored plants and nothing else. Water
    // stands in for a road deck, a building pad or a terraform: all four reach a plant through
    // the one placement_clear predicate, and only water can be authored without a road fixture.
    core.vegetation = VegetationGenerator::resolve(
        VegetationConfig {
            enabled: false,
            ..Default::default()
        },
        &core.config,
    );
    let drowned = Vector2::new(-100.0, 0.0);
    let dry = Vector2::new(100.0, 0.0);
    assert!(add_at(&mut core, drowned, 0));
    assert!(add_at(&mut core, dry, 1));
    let before = scatter(&core, true);
    assert_eq!(before.len(), 12);

    // Clamping at a position past the far edge reports the last grid cell, which is the row
    // stride the dense baseline buffer is written in.
    let (last_x, last_z) = core.watermap.world_to_grid_cell_clamped(1.0e6, 1.0e6);
    let (gx, gz) = core
        .watermap
        .world_to_grid_cell_clamped(drowned.x, drowned.y);
    let drained = core.watermap.clone_baseline_depth_dense();
    let mut flooded = drained.clone();
    // Wider than the 6 m canopy footprint the plant is tested over, and far short of the
    // second plant 200 m away.
    for z in gz.saturating_sub(2)..=(gz + 2).min(last_z) {
        for x in gx.saturating_sub(2)..=(gx + 2).min(last_x) {
            flooded[z * (last_x + 1) + x] = 1.0;
        }
    }
    core.watermap
        .replace_baseline_depth_from_dense(&flooded)
        .unwrap();

    // Only the submerged plant leaves the payload, and the delta still stores both cells.
    let after = scatter(&core, true);
    assert_eq!(after.len(), 6);
    assert_eq!(after[5], 1.0);
    assert_eq!(core.vegetation_edits.len(), 2);

    // Clearance is re-evaluated rather than destructive, so removing the surface restores the
    // plant bit for bit, which is already how a generated candidate behaves.
    core.watermap
        .replace_baseline_depth_from_dense(&drained)
        .unwrap();
    assert_eq!(scatter(&core, true), before);
}

#[test]
fn vegetation_clears_under_a_committed_field_and_returns_when_it_is_removed() {
    let mut core = core();
    // Generator off, so the patch payload is exactly the two authored plants below.
    core.vegetation = VegetationGenerator::resolve(
        VegetationConfig {
            enabled: false,
            ..Default::default()
        },
        &core.config,
    );
    let ploughed = Vector2::new(-100.0, 0.0);
    let untouched = Vector2::new(100.0, 0.0);
    assert!(add_at(&mut core, ploughed, 0));
    assert!(add_at(&mut core, untouched, 1));
    let before = scatter(&core, true);
    assert_eq!(before.len(), 12);

    // A farm's committed field, indexed the way agriculture indexes one on commit.
    core.allocator.field_clearance.set(
        0,
        &[
            Vector2::new(-120.0, -20.0),
            Vector2::new(-80.0, -20.0),
            Vector2::new(-80.0, 20.0),
            Vector2::new(-120.0, 20.0),
        ],
    );
    let after = scatter(&core, true);
    assert_eq!(after.len(), 6);
    assert_eq!(after[5], 1.0);

    // The patch the field covers is restaled so the change reaches the renderer this session,
    // and a patch the field does not reach keeps its revision.
    let layout = PatchLayout::new(&core);
    let covered = layout.key(ploughed.x, ploughed.y);
    let remote = layout.key(layout.span * 2.0, layout.span * 2.0);
    let covered_before = core.vegetation_edits.patch_generation(covered);
    let remote_before = core.vegetation_edits.patch_generation(remote);
    core.invalidate_vegetation_over((Vector2::new(-120.0, -20.0), Vector2::new(-80.0, 20.0)));
    assert_eq!(
        core.vegetation_edits.patch_generation(covered),
        covered_before + 1
    );
    assert_eq!(
        core.vegetation_edits.patch_generation(remote),
        remote_before
    );

    // Clearance is re-evaluated rather than destructive, so removing the field restores the
    // plant bit for bit, exactly as removing a road deck or a building pad does.
    core.allocator.field_clearance.clear();
    assert_eq!(scatter(&core, true), before);
}

#[test]
fn vegetation_patch_revisions_touch_only_changed_plants() {
    let mut core = core();
    let layout = PatchLayout::new(&core);
    let key = layout.key(1.0, 1.0);
    let neighbor = layout.key(layout.span + 1.0, 1.0);
    let terrain_generation = core.heightmap.source_generation();
    assert!(add_at(&mut core, Vector2::ONE, 0));
    assert_eq!(core.vegetation_edits.patch_generation(key), 1);
    assert_eq!(core.vegetation_edits.patch_generation(neighbor), 0);
    assert_eq!(remove_at(&mut core, Vector2::ONE, 0.01, 0), 1);
    assert_eq!(core.vegetation_edits.patch_generation(key), 2);
    assert_eq!(core.vegetation_edits.patch_generation(neighbor), 0);
    assert_eq!(core.heightmap.source_generation(), terrain_generation);
    // A disc straddling a render boundary touches both patches, not a third one.
    assert!(add_at(&mut core, Vector2::new(-1.5, 1.0), 1));
    assert!(add_at(&mut core, Vector2::ONE, 0));
    let other = layout.key(-1.0, 1.0);
    assert_eq!(remove_at(&mut core, Vector2::ZERO, 2.0, 0), 2);
    assert_eq!(core.vegetation_edits.patch_generation(other), 2);
    assert_eq!(core.vegetation_edits.patch_generation(key), 4);
    assert_eq!(core.vegetation_edits.patch_generation(neighbor), 0);
}

#[test]
fn vegetation_invalid_inputs_store_nothing_and_empty_removal_is_sparse() {
    let mut core = core();
    for pos in [
        Vector2::splat(f32::INFINITY),
        Vector2::new(f32::NAN, 0.0),
        Vector2::new(1280.0, 0.0),
    ] {
        assert!(!add_at(&mut core, pos, 0));
    }
    // One below the preset table and one past its end, so neither names a preset.
    for option in [-1, brush::PRESETS.len() as i64] {
        assert!(!add_at(&mut core, Vector2::ZERO, option));
        assert_eq!(paint_at(&mut core, Vector2::ZERO, 8.0, option, 0, usize::MAX), 0);
    }
    for radius in [-1.0, f32::NAN, f32::INFINITY, 1025.0] {
        assert_eq!(remove_at(&mut core, Vector2::ZERO, radius, 0), 0);
        assert_eq!(paint_at(&mut core, Vector2::ZERO, radius, 0, 0, usize::MAX), 0);
    }
    let p = first_candidate(&core, false);
    assert_eq!(remove_at(&mut core, Vector2::new(p.x, p.z), 0.0, 0), 0);
    assert_eq!(core.vegetation_edits.len(), 0);
    let mut depth = core.watermap.clone_baseline_depth_dense();
    depth.fill(1.0);
    core.watermap
        .replace_baseline_depth_from_dense(&depth)
        .unwrap();
    assert!(!add_at(&mut core, Vector2::ZERO, 0));
    assert_eq!(paint_at(&mut core, Vector2::ZERO, 32.0, 0, 0, usize::MAX), 0);
    assert_eq!(core.vegetation_edits.len(), 0);
}

#[test]
fn vegetation_save_round_trip_reproduces_scatter_and_v60_loads_empty_delta() {
    let mut core = core();
    // Full world setup belongs here because this test exercises the actual save bridge.
    core.create_blank_world_internal(2560.0, 2560.0, 10.0, 640.0, 20.0)
        .unwrap();
    let p = first_candidate(&core, true);
    assert_eq!(remove_at(&mut core, Vector2::new(p.x, p.z), 0.01, 0), 1);
    assert!(add_at(&mut core, Vector2::new(3.25, -32.0), 3));
    assert!(paint_at(&mut core, Vector2::ZERO, 45.0, 1, 0, usize::MAX) > 0);
    assert!(paint_at(&mut core, Vector2::ZERO, 45.0, 2, 0, usize::MAX) > 0);
    let before = [scatter(&core, true), scatter(&core, false)];
    let path =
        std::env::temp_dir().join(format!("metrum-vegetation-{}.sqlite", std::process::id()));
    core.save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let ordered_rows = |table: &str| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT layer, cell_z, cell_x FROM {table} ORDER BY rowid"
            ))
            .unwrap();
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, i32>(0)?,
                r.get::<_, i32>(1)?,
                r.get::<_, i32>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    for table in ["vegetation_removals", "vegetation_additions"] {
        assert!(
            ordered_rows(table)
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
        );
    }
    core.load_game_internal(path.to_str().unwrap()).unwrap();
    assert_eq!([scatter(&core, true), scatter(&core, false)], before);
    conn.execute_batch("UPDATE save_meta SET version = 60; DROP TABLE vegetation_removals; DROP TABLE vegetation_additions;").unwrap();
    core.load_game_internal(path.to_str().unwrap()).unwrap();
    assert_eq!(core.vegetation_edits.len(), 0);
    drop(conn);
    std::fs::remove_file(path).unwrap();
}

#[test]
#[ignore = "targeted Criterion vegetation locality benchmark"]
fn vegetation_edit_benchmark() {
    use criterion::Criterion;
    use std::hint::black_box;
    // A canopy patch is only ~1024 cells, so Rayon scheduling variance there is larger than any
    // per-cell effect worth measuring. The understory arm below carries ~50x the cells at the
    // same span, which is what actually resolves per-cell cost; both get a longer measurement.
    let mut criterion = Criterion::default()
        .sample_size(50)
        .warm_up_time(std::time::Duration::from_secs(2))
        .measurement_time(std::time::Duration::from_secs(6));
    let mut group = criterion.benchmark_group("VegetationEdits");
    for background in [0, 100_000] {
        let mut core = core();
        let mut edits = crate::simulation::vegetation::edits::VegetationEdits::default();
        edits.reserve(background, 0);
        for i in 0..background {
            edits.set_removed(
                VegetationCell {
                    layer: VegetationLayer::Canopy,
                    x: i as i32 + 1000,
                    z: 1000,
                },
                true,
            );
        }
        core.vegetation_edits = edits;
        prepare_sites(&mut core);
        group.bench_function(format!("scatter_{background}"), |b| {
            b.iter(|| black_box(scatter(black_box(&core), true)))
        });
        group.bench_function(format!("coverage_{background}"), |b| {
            b.iter(|| {
                black_box(land_cover::Coverage::build(
                    black_box(&core),
                    Vector2::splat(-255.0),
                    510.0,
                ))
            })
        });
        group.bench_function(format!("understory_{background}"), |b| {
            b.iter(|| black_box(scatter(black_box(&core), false)))
        });
        // Reset only the affected neighborhood outside each timing interval. Cloning the
        // background map here would evict its local buckets and measure setup cache churn.
        let center = Vector2::ZERO;
        remove_at(&mut core, center, 64.0, 0);
        let expected_added = paint_at(&mut core, center, 64.0, 0, 0, usize::MAX);
        group.bench_function(format!("paint_{background}"), |b| {
            b.iter_custom(|iterations| {
                let mut elapsed = std::time::Duration::ZERO;
                for _ in 0..iterations {
                    remove_at(&mut core, center, 64.0, 0);
                    let start = std::time::Instant::now();
                    let added = black_box(paint_at(&mut core, center, 64.0, 0, 0, usize::MAX));
                    elapsed += start.elapsed();
                    assert_eq!(added, expected_added);
                }
                elapsed
            })
        });
        // Matched control for the block index: one edit inside the measured rectangle puts the
        // per-cell store lookups back on, which is what an untouched patch now skips entirely.
        for layer in [VegetationLayer::Canopy, VegetationLayer::Understory] {
            core.vegetation_edits
                .set_removed(VegetationCell { layer, x: 0, z: 0 }, true);
        }
        group.bench_function(format!("scatter_edited_{background}"), |b| {
            b.iter(|| black_box(scatter(black_box(&core), true)))
        });
        group.bench_function(format!("understory_edited_{background}"), |b| {
            b.iter(|| black_box(scatter(black_box(&core), false)))
        });
        // Worst case for the authored clearance pass: a stroke covering the whole measured
        // patch, so every cell the generator rejected carries an authored plant that has to be
        // re-tested against the current surface on every fetch.
        let painted = paint_at(&mut core, center, 255.0, 0, 0, usize::MAX);
        assert!(
            painted > 100,
            "the painted arm must fill the measured patch"
        );
        group.bench_function(format!("scatter_painted_{background}"), |b| {
            b.iter(|| black_box(scatter(black_box(&core), true)))
        });
        group.bench_function(format!("coverage_painted_{background}"), |b| {
            b.iter(|| {
                black_box(land_cover::Coverage::build(
                    black_box(&core),
                    Vector2::splat(-255.0),
                    510.0,
                ))
            })
        });
    }
    group.finish();
}

#[test]
fn vegetation_footprint_rejects_water_built_surfaces_and_steep_ground() {
    assert!(clear_footprint(0.0, 0.0, 10.0, 6.0, 3.0, |_, _| Some(10.0)));
    assert!(!clear_footprint(0.0, 0.0, 10.0, 6.0, 3.0, |_, _| None));
    // Dry roots are insufficient if one shoreline/canopy sample is blocked.
    assert!(!clear_footprint(0.0, 0.0, 10.0, 6.0, 3.0, |x, z| {
        if x == 6.0 && z == 6.0 {
            None
        } else {
            Some(10.0)
        }
    }));
    assert!(!clear_footprint(0.0, 0.0, 10.0, 6.0, 3.0, |x, _| Some(
        10.0 + x
    )));
    assert!(!clear_footprint(0.0, 0.0, 10.0, 6.0, 3.0, |_, _| Some(
        f32::NAN
    )));
    // The understory radius accepts relief that would reject a canopy tree.
    assert!(clear_footprint(0.0, 0.0, 10.0, 2.5, 1.6, |x, _| Some(
        10.0 + x * 0.5
    )));
    assert!(!clear_footprint(0.0, 0.0, 10.0, 6.0, 3.0, |x, _| Some(
        10.0 + x * 0.6
    )));
}

#[test]
fn vegetation_candidates_are_stable_bounded_and_order_independent() {
    let cell = VegetationGenerator::default().canopy_cell_m;
    let expected: Vec<_> = (-100..100)
        .map(|x| candidate(x, -7, cell, CANOPY_SALT))
        .collect();
    let parallel: Vec<_> = (-100..100)
        .into_par_iter()
        .map(|x| candidate(x, -7, cell, CANOPY_SALT))
        .collect();
    assert_eq!(expected, parallel);
    for (x, c) in (-100..100).zip(expected) {
        assert!(c[0] >= x as f32 * cell && c[0] < (x + 1) as f32 * cell);
        assert!(c[1] >= -8.0 * cell && c[1] < -6.0 * cell);
        assert!((0.75..1.35).contains(&c[3]));
    }
}

#[test]
fn vegetation_understory_grid_is_denser_and_independent_of_the_canopy_grid() {
    let g = VegetationGenerator::default();
    let canopy = candidate(
        3,
        5,
        g.canopy_cell_m,
        layer_base(CANOPY_SALT, g.config.seed),
    );
    let understory = candidate(
        3,
        5,
        g.understory_cell_m,
        layer_base(UNDERSTORY_SALT, g.config.seed),
    );
    // A shared salt would place the understory at a scaled copy of the canopy offsets.
    assert!((canopy[0] / g.canopy_cell_m - understory[0] / g.understory_cell_m).abs() > 1e-4);
    assert!(understory[0] >= 3.0 * g.understory_cell_m);
    assert!(understory[0] < 4.0 * g.understory_cell_m);
}

#[test]
#[ignore = "matched release brush timing"]
fn vegetation_brush_benchmark() {
    use criterion::Criterion;
    let mut core = core();
    prepare_sites(&mut core);
    let mut criterion = Criterion::default()
        .sample_size(20)
        .warm_up_time(std::time::Duration::from_secs(1))
        .measurement_time(std::time::Duration::from_secs(3));
    remove_at(&mut core, Vector2::ZERO, 64.0, 0);
    let expected = paint_at(&mut core, Vector2::ZERO, 64.0, 0, 0, usize::MAX);
    eprintln!(
        "brush radius=64m plants={expected} workers={}",
        rayon::current_num_threads()
    );
    criterion.bench_function("VegetationBrush/paint_64m", |b| {
        b.iter_custom(|iterations| {
            let mut elapsed = std::time::Duration::ZERO;
            for _ in 0..iterations {
                remove_at(&mut core, Vector2::ZERO, 64.0, 0);
                let start = std::time::Instant::now();
                let added = paint_at(&mut core, Vector2::ZERO, 64.0, 0, 0, usize::MAX);
                elapsed += start.elapsed();
                assert_eq!(added, expected);
            }
            elapsed
        })
    });
    // A held stamp repeats about ten times a second. Its worst case is the largest tree disc
    // over a stand already at density: every proposal is ranked and checked, and none is added.
    remove_at(&mut core, Vector2::ZERO, 256.0, 0);
    let full = paint_at(&mut core, Vector2::ZERO, 256.0, 8, 0, usize::MAX);
    let limit = stamp_limit(8, 256.0);
    eprintln!("held radius=256m stand={full} limit={limit}");
    criterion.bench_function("VegetationBrush/held_full_256m", |b| {
        b.iter(|| assert_eq!(paint_at(&mut core, Vector2::ZERO, 256.0, 8, 0, limit), 0))
    });
    criterion.final_summary();
}

#[test]
#[ignore = "release class, overlap and dense-cell brush costs"]
fn vegetation_brush_class_benchmark() {
    use criterion::Criterion;
    let mut criterion = Criterion::default()
        .sample_size(20)
        .warm_up_time(std::time::Duration::from_secs(1))
        .measurement_time(std::time::Duration::from_secs(3));
    let mut group = criterion.benchmark_group("VegetationBrushClasses");
    for background in [0, 100_000] {
        for option in [4, 2, 3] {
            let mut core = core();
            core.vegetation.config.enabled = false;
            core.vegetation_edits.reserve(background, 0);
            for i in 0..background {
                core.vegetation_edits.set_removed(
                    VegetationCell {
                        layer: VegetationLayer::Canopy,
                        x: i as i32 + 1000,
                        z: 1000,
                    },
                    true,
                );
            }
            prepare_sites(&mut core);
            let expected = paint_at(&mut core, Vector2::ZERO, 64.0, option, 0, usize::MAX);
            assert!(core.undo_action_internal());
            eprintln!("class preset={option} background={background} plants={expected}");
            group.bench_function(format!("fresh_{option}_{background}"), |b| {
                b.iter_custom(|iterations| {
                    let mut elapsed = std::time::Duration::ZERO;
                    for _ in 0..iterations {
                        let start = std::time::Instant::now();
                        let count = paint_at(&mut core, Vector2::ZERO, 64.0, option, 0, usize::MAX);
                        elapsed += start.elapsed();
                        assert_eq!(count, expected);
                        assert!(core.undo_action_internal());
                    }
                    elapsed
                })
            });
            paint_at(&mut core, Vector2::ZERO, 64.0, option, 0, usize::MAX);
            paint_at(&mut core, Vector2::new(8.0, 0.0), 64.0, option, 0, usize::MAX);
            group.bench_function(format!("overlap_repeat_{option}_{background}"), |b| {
                b.iter(|| {
                    assert_eq!(paint_at(&mut core, Vector2::ZERO, 64.0, option, 0, usize::MAX), 0)
                })
            });
        }
    }
    // Dense legacy vectors are permitted by old saves: report their local cost explicitly.
    let mut core = core();
    core.vegetation.config.enabled = false;
    prepare_sites(&mut core);
    let cell = cell_at(
        Vector2::ONE,
        VegetationLayer::Canopy,
        core.vegetation.canopy_cell_m,
    );
    for i in 0..4000 {
        core.vegetation_edits.add(
            cell,
            Plant {
                x: 1.0 + i as f32 * 0.0001,
                z: 1.0,
                yaw: 0.0,
                scale: 1.0,
                species: 0,
                variant: 0,
            },
        );
    }
    let expected = paint_at(&mut core, Vector2::ZERO, 64.0, 4, 0, usize::MAX);
    assert!(core.undo_action_internal());
    eprintln!("dense local legacy=4000 plants={expected}");
    group.bench_function("dense_local_4000", |b| {
        b.iter_custom(|iterations| {
            let mut elapsed = std::time::Duration::ZERO;
            for _ in 0..iterations {
                let start = std::time::Instant::now();
                let count = paint_at(&mut core, Vector2::ZERO, 64.0, 4, 0, usize::MAX);
                elapsed += start.elapsed();
                assert_eq!(count, expected);
                assert!(core.undo_action_internal());
            }
            elapsed
        })
    });
    group.finish();
}

fn authored(core: &SimCore) -> Vec<Plant> {
    core.vegetation_edits
        .sorted_cells()
        .into_iter()
        .flat_map(|cell| core.vegetation_edits.cell(cell).1.iter().copied())
        .collect()
}

fn assert_spacing(plants: &[Plant]) {
    for (i, a) in plants.iter().enumerate() {
        let class = brush::PlantClass::of(a.species, a.variant);
        for b in &plants[i + 1..] {
            if brush::PlantClass::of(b.species, b.variant) == class {
                assert!(
                    (a.x - b.x).powi(2) + (a.z - b.z).powi(2) >= class.spacing().powi(2),
                    "{a:?} conflicts with {b:?}"
                );
            }
        }
    }
}

#[test]
fn vegetation_brush_spacing_repeat_and_cross_class_coexistence() {
    for order in [[2, 4], [4, 2], [3, 4]] {
        let mut core = core();
        core.vegetation.config.enabled = false;
        for option in order {
            assert!(paint_at(&mut core, Vector2::ZERO, 32.0, option, 17, usize::MAX) > 0);
            assert_eq!(paint_at(&mut core, Vector2::ZERO, 32.0, option, 17, usize::MAX), 0);
        }
        let plants = authored(&core);
        assert_spacing(&plants);
        assert!(plants.iter().any(|p| p.species < 2));
        assert!(plants.iter().any(|p| p.species >= 2));
        // Nearby cross-class pairs prove independent occupancy, beyond merely sharing a disc.
        assert!(plants.iter().any(|a| {
            a.species < 2
                && plants.iter().any(|b| {
                    b.species >= 2 && (a.x - b.x).powi(2) + (a.z - b.z).powi(2) < 0.8 * 0.8
                })
        }));
        assert!(core.undo_action_internal());
        assert!(authored(&core).is_empty());
    }
    for order in [[2, 4], [4, 2]] {
        let mut core = core();
        core.vegetation.config.enabled = false;
        for option in order {
            assert!(add_at(&mut core, Vector2::ZERO, option));
        }
        assert!(!add_at(&mut core, Vector2::new(0.1, 0.0), 2));
        assert!(!add_at(&mut core, Vector2::new(2.49, 0.0), 0));
        assert_eq!(remove_at(&mut core, Vector2::ZERO, 0.0, 0), 2);
    }
}

#[test]
fn vegetation_brush_spacing_is_independent_and_parallel_order_is_stable() {
    let mut reference = None;
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let mut core = core();
        pool.install(|| {
            paint_at(&mut core, Vector2::new(-20.0, 15.0), 64.0, 2, 0, usize::MAX);
            paint_at(&mut core, Vector2::new(-5.0, 15.0), 64.0, 4, 0, usize::MAX);
        });
        let records = authored(&core);
        if let Some(expected) = &reference {
            assert_eq!(&records, expected);
        }
        reference = Some(records);
    }
    let mut reference = None;
    for canopy_m in [8.0, 16.0, 32.0] {
        let mut core = core();
        core.vegetation.config.enabled = false;
        core.vegetation.canopy_cell_m = canopy_m;
        paint_at(&mut core, Vector2::ZERO, 32.0, 1, 0, usize::MAX);
        let mut records = authored(&core);
        records.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.z.total_cmp(&b.z)));
        if let Some(expected) = &reference {
            assert_eq!(&records, expected);
        }
        reference = Some(records);
    }
}

#[test]
fn vegetation_brush_bounds_and_full_cell_darts() {
    use brush::PlantClass;
    for (option, class) in [
        (4, PlantClass::Tree),
        (2, PlantClass::Ground),
        (3, PlantClass::Rock),
    ] {
        let mut core = core();
        assert_eq!(
            paint_at(
                &mut core,
                Vector2::ZERO,
                class.max_radius() + 0.01,
                option,
                0,
                usize::MAX,
            ),
            0
        );
        assert_eq!(core.vegetation_edits.len(), 0);
        let step = class.spacing() * std::f32::consts::FRAC_1_SQRT_2;
        let axis_bound = (2.0 * class.max_radius() / step).ceil() as usize + 1;
        assert!(2 * axis_bound * axis_bound <= 169_362);
    }
}

#[test]
fn vegetation_named_preset_density_on_clear_ground() {
    let mut valid = true;
    for (option, low, high) in [
        (4, 550.0, 700.0),
        (5, 550.0, 700.0),
        (6, 550.0, 700.0),
        (7, 550.0, 700.0),
        (8, 460.0, 600.0),
        (9, 50.0, 75.0),
        (10, 185.0, 255.0),
    ] {
        let mut total = 0;
        for seed in [0, 7, 99] {
            let mut core = core();
            core.vegetation.config.enabled = false;
            core.vegetation.config.seed = seed;
            total += paint_at(&mut core, Vector2::ZERO, 192.0, option, 0, usize::MAX);
        }
        let density = total as f32 / (3.0 * std::f32::consts::PI * 192.0 * 192.0 / 10_000.0);
        eprintln!("preset={option} density={density:.2} stems/ha band={low}..{high}");
        valid &= (low..=high).contains(&density);
    }
    assert!(valid, "preset density outside its documented band");
}

#[test]
fn vegetation_occupancy_queries_both_layers_across_owner_boundaries() {
    let mut core = core();
    core.vegetation.config.enabled = false;
    let pos = Vector2::new(-0.1, 0.0);
    let owner = cell_at(
        pos,
        VegetationLayer::Understory,
        core.vegetation.understory_cell_m,
    );
    core.vegetation_edits.add(
        owner,
        Plant {
            x: pos.x,
            z: pos.y,
            yaw: 0.0,
            scale: 1.0,
            species: 3,
            variant: 0,
        },
    );
    assert!(!add_at(&mut core, Vector2::new(0.1, 0.0), 3));
    assert!(add_at(&mut core, Vector2::new(0.1, 0.0), 4));
}

#[test]
fn vegetation_restore_cannot_violate_new_spacing() {
    let mut core = core();
    let plant = first_candidate(&core, true);
    let pos = Vector2::new(plant.x, plant.z);
    remove_at(&mut core, pos, 0.01, 0);
    assert!(add_at(&mut core, pos + Vector2::new(1.0, 0.0), 4));
    assert_eq!(paint_at(&mut core, pos, 0.01, plant.species as i64, 0, usize::MAX), 0);
}

#[test]
fn undo_restores_the_scatter_a_brush_stroke_replaced() {
    let mut core = core();
    let before = [scatter(&core, true), scatter(&core, false)];
    let standing = first_candidate(&core, true);
    let center = Vector2::new(standing.x, standing.z);
    assert!(remove_at(&mut core, center, 48.0, 0) > 0);
    assert_ne!(
        scatter(&core, true),
        before[0],
        "the fixture must lose canopy for this to prove anything"
    );
    assert!(core.undo_action_internal());
    assert_eq!(
        [scatter(&core, true), scatter(&core, false)],
        before,
        "undoing a clear-cut must reproduce every generated plant bit"
    );

    // Painting over the cleared ground and undoing that must not resurrect the clear-cut:
    // each journal holds only the state its own stroke found.
    assert!(paint_at(&mut core, center, 48.0, 1, 0, usize::MAX) > 0);
    let painted = scatter(&core, true);
    assert!(add_at(&mut core, Vector2::new(-64.0, 64.0), 3));
    assert!(core.undo_action_internal());
    assert_eq!(scatter(&core, true), painted, "a point plant undoes alone");
    assert!(core.undo_action_internal());
    assert_eq!(scatter(&core, true), before[0], "the stroke undoes as one");
}

#[test]
fn a_dragged_stroke_costs_one_undo_entry() {
    let mut core = core();
    let before = scatter(&core, true);
    let depth = core.undo_stack.len();
    for offset in [-32.0, -16.0, 0.0, 16.0, 32.0] {
        assert!(remove_at(&mut core, Vector2::new(offset, 0.0), 24.0, 7) > 0);
    }
    assert_eq!(
        core.undo_stack.len(),
        depth + 1,
        "a held drag must not fill the history with its own stamps"
    );
    assert!(core.undo_action_internal());
    assert_eq!(
        scatter(&core, true),
        before,
        "one undo must reverse the whole drag"
    );
}

#[test]
fn a_stroke_that_starts_over_nothing_does_not_merge_into_the_previous_action() {
    let mut core = core();
    let standing = first_candidate(&core, true);
    let center = Vector2::new(standing.x, standing.z);
    assert!(paint_at(&mut core, center, 48.0, 1, 0, usize::MAX) > 0);

    // One earlier action of its own, so there is a vegetation entry for a later stroke to
    // wrongly fold into.
    assert_eq!(remove_at(&mut core, center, 0.01, 11), 1);
    let before_clear = scatter(&core, true);

    // The press of the next stroke lands where that action already cleared the ground, so
    // the stamp changes nothing and journals nothing. The stamps after it still belong to
    // their own stroke and must not attach to the entry underneath.
    assert_eq!(
        remove_at(&mut core, center, 0.01, 12),
        0,
        "the opening stamp of this stroke must change nothing"
    );
    assert!(remove_at(&mut core, center, 48.0, 12) > 0);
    assert!(core.undo_action_internal());
    assert_eq!(
        scatter(&core, true),
        before_clear,
        "undoing a clear must not also undo the action beneath it"
    );
}

#[test]
fn lane_five_carries_a_pinned_variant_without_disturbing_the_species_under_it() {
    // The renderer reads this lane back as `packed & 3` and `packed >> 2`; see the constants
    // of the same name in vegetation.gd. An unpinned plant must still pack to a bare ordinal,
    // because that is what every save and every generated plant written so far contains.
    for species in 0..4u8 {
        assert_eq!(
            pack_species_variant(species, VARIANT_FROM_SEED),
            f32::from(species),
            "an unpinned plant must pack exactly as it did before pinning existed"
        );
        for variant in 0..=12u8 {
            let packed = pack_species_variant(species, variant) as u32;
            assert_eq!(packed & 3, u32::from(species));
            assert_eq!(packed >> 2, u32::from(variant));
        }
    }
}

// Preset ordinals under test, matching the table in brush.rs.
const CONIFER: i64 = 0;
const PINE: i64 = 4;
const SPRUCE: i64 = 5;
const MIXED_FOREST: i64 = 8;
const MEADOW: i64 = 9;
const NORTHERN_DWARF: i64 = 10;

// Every packed placement of the canopy layer as (species, biased variant, scale).
fn planted(core: &SimCore) -> Vec<(u8, u8, f32)> {
    scatter(core, true)
        .chunks_exact(6)
        .map(|record| {
            let packed = record[5] as u32;
            ((packed & 3) as u8, (packed >> 2) as u8, record[4])
        })
        .collect()
}

#[test]
fn a_named_tree_plants_only_the_meshes_that_are_that_tree() {
    let mut spruce_core = core();
    let before = planted(&spruce_core).len();
    assert!(paint_at(&mut spruce_core, Vector2::ZERO, 64.0, SPRUCE, 0, usize::MAX) > 0);
    let pinned: Vec<_> = planted(&spruce_core)
        .into_iter()
        .filter(|p| p.1 != 0)
        .collect();
    assert!(
        pinned.len() > before / 4,
        "the stroke must actually pin most of what it planted"
    );
    for (species, variant, _) in &pinned {
        assert_eq!(*species, 0, "spruce is a conifer");
        // tree_species.gd reads the unbiased variant, where every third one is a spruce.
        assert_eq!(
            (variant - 1) % 3,
            2,
            "a spruce brush must never plant a pine mesh"
        );
    }

    let mut pine_core = core();
    assert!(paint_at(&mut pine_core, Vector2::ZERO, 64.0, PINE, 0, usize::MAX) > 0);
    let pines: Vec<_> = planted(&pine_core)
        .into_iter()
        .filter(|p| p.1 != 0)
        .collect();
    assert!(!pines.is_empty());
    for (species, variant, _) in &pines {
        assert_eq!(*species, 0);
        assert_ne!(
            (variant - 1) % 3,
            2,
            "a pine brush must never plant a spruce mesh"
        );
    }
    assert!(
        pines.iter().map(|p| p.1).collect::<HashSet<_>>().len() > 1,
        "one named tree still spans its own meshes, or a stand is a row of clones"
    );
}

#[test]
fn an_unpinned_preset_preserves_seeded_variants_and_scale_band() {
    let mut core = core();
    assert!(paint_at(&mut core, Vector2::ZERO, 64.0, CONIFER, 0, usize::MAX) > 0);
    for (_, variant, scale) in planted(&core) {
        assert_eq!(variant, 0, "conifer leaves the mesh to the appearance seed");
        assert!((0.75..=1.35).contains(&scale));
    }
}

#[test]
fn a_thinned_preset_plants_fewer_trees_and_thins_to_the_same_ones_every_time() {
    let mut dense = core();
    let mut sparse = core();
    let planted_dense = paint_at(&mut dense, Vector2::ZERO, 64.0, CONIFER, 0, usize::MAX);
    let planted_sparse = paint_at(&mut sparse, Vector2::ZERO, 64.0, MEADOW, 0, usize::MAX);
    assert!(planted_dense > 0 && planted_sparse > 0);
    assert!(
        (planted_sparse as f32) < planted_dense as f32 * 0.35,
        "the meadow preset must remain substantially sparser: {planted_sparse} of {planted_dense}"
    );

    // Thinning is a pure function of the cell, so a repeat finds every point already taken.
    assert_eq!(
        paint_at(&mut sparse, Vector2::ZERO, 64.0, MEADOW, 0, usize::MAX),
        0,
        "a repeated stroke must not thin to a different set and fill the gaps"
    );
    let mut repeat = core();
    assert_eq!(
        paint_at(&mut repeat, Vector2::ZERO, 64.0, MEADOW, 0, usize::MAX),
        planted_sparse
    );
    assert_eq!(planted(&repeat), planted(&sparse));
}

#[test]
fn held_stamps_thicken_a_stand_step_by_step_to_the_single_stamp_stand() {
    let mut full = core();
    let planted_full = paint_at(&mut full, Vector2::ZERO, 64.0, MIXED_FOREST, 0, usize::MAX);
    let limit = stamp_limit(MIXED_FOREST, 64.0);
    // 1.29 ha at 10 stems/ha per stamp.
    assert_eq!(limit, 13);
    let mut held = core();
    let mut total = 0;
    loop {
        let added = paint_at(&mut held, Vector2::ZERO, 64.0, MIXED_FOREST, 3, limit);
        assert!(added <= limit);
        total += added;
        if added < limit {
            break;
        }
    }
    assert!(total / limit > 20);
    assert_eq!(total, planted_full);
    assert_eq!(authored(&held), authored(&full));
    assert_eq!(paint_at(&mut held, Vector2::ZERO, 64.0, MIXED_FOREST, 3, limit), 0);
    assert!(held.undo_action_internal());
    assert!(authored(&held).is_empty());
}

#[test]
fn a_dwarfing_preset_moves_every_plant_into_its_own_scale_band() {
    let mut core = core();
    let untouched: HashSet<_> = planted(&core)
        .into_iter()
        .map(|p| (p.0, p.1, p.2.to_bits()))
        .collect();
    assert!(paint_at(&mut core, Vector2::ZERO, 64.0, NORTHERN_DWARF, 0, usize::MAX) > 0);
    let dwarfed: Vec<_> = planted(&core)
        .into_iter()
        .filter(|p| !untouched.contains(&(p.0, p.1, p.2.to_bits())))
        .collect();
    assert!(!dwarfed.is_empty(), "the stroke must add plants of its own");
    for (_, _, scale) in &dwarfed {
        assert!(
            (0.45..=0.75).contains(scale),
            "a dwarfed tree must not keep the generator's scale band: {scale}"
        );
    }
}

#[test]
fn a_mix_preset_plants_more_than_one_tree() {
    let mut core = core();
    assert!(paint_at(&mut core, Vector2::ZERO, 64.0, MIXED_FOREST, 0, usize::MAX) > 0);
    let kinds: HashSet<_> = planted(&core)
        .into_iter()
        .filter(|p| p.1 != 0)
        .map(|p| (p.0, (p.1 - 1) % 3 == 2))
        .collect();
    assert_eq!(
        kinds.len(),
        4,
        "a mixed forest must reach pine, spruce, birch and aspen: {kinds:?}"
    );
}

#[test]
fn painting_a_named_tree_over_a_cleared_one_authors_it_instead_of_regrowing_the_old_one() {
    let mut core = core();
    let standing = first_candidate(&core, true);
    let pos = Vector2::new(standing.x, standing.z);
    assert_eq!(remove_at(&mut core, pos, 0.01, 0), 1);
    // The generator chose this cell's species itself, so an unpinned brush of that same
    // species is the same edit and collapses back to no stored edit at all.
    assert_eq!(
        paint_at(&mut core, pos, 0.01, i64::from(standing.species), 0, usize::MAX),
        1
    );
    assert_eq!(core.vegetation_edits.len(), 0);

    assert_eq!(remove_at(&mut core, pos, 0.01, 0), 1);
    let named = if standing.species == 0 { SPRUCE } else { PINE };
    assert_eq!(paint_at(&mut core, pos, 0.01, named, 0, usize::MAX), 1);
    assert_eq!(
        core.vegetation_edits.len(),
        1,
        "a player who named the tree must get that tree authored over the tombstone"
    );
}

#[test]
fn a_hedge_line_lays_facing_modules_once_and_only_along_a_line() {
    let mut core = core();
    core.vegetation.config.enabled = false;
    let (from, to) = (Vector2::new(-4.0, 3.0), Vector2::new(5.0, 3.0));
    // Nine metres close with nine modules, each facing along the row at full size.
    assert_eq!(line_at(&mut core, from, to, 17, 1), 9);
    let records = scatter(&core, true);
    let hedge: Vec<_> = records.chunks_exact(6).filter(|r| r[5] != 0.0).collect();
    assert_eq!(hedge.len(), 9);
    for record in &hedge {
        // Level ground: no rise along the row.
        assert_eq!((record[3], record[4]), (0.0, 0.0));
        assert_eq!(record[5], f32::from((13_u8 << SPECIES_BITS) | 2));
    }
    // Redrawing the row stacks nothing, and neither a brush nor a tree preset lays a line.
    assert_eq!(line_at(&mut core, from, to, 17, 2), 0);
    assert_eq!(line_at(&mut core, from, to, 4, 3), 0);
    assert_eq!(paint_at(&mut core, Vector2::new(0.0, -20.0), 8.0, 17, 4, usize::MAX), 0);
}

#[test]
fn a_hedge_row_ends_flush_and_joins_the_hedge_it_is_drawn_onto() {
    let mut core = core();
    core.vegetation.config.enabled = false;
    // World centres of every hedge module, so a row is what one stroke added to them.
    let centres = |core: &SimCore| -> Vec<(f32, f32)> {
        let records = scatter(core, true);
        let hedge = records.chunks_exact(6).filter(|r| r[5] != 0.0);
        hedge.map(|r| (r[0] - 255.0, r[2] - 255.0)).collect()
    };
    let mut before = Vec::new();
    let mut row = |core: &SimCore| -> Vec<(f32, f32)> {
        let after = centres(core);
        let added = after.iter().filter(|p| !before.contains(*p)).copied().collect();
        before = after;
        added
    };
    let lay = |core: &mut SimCore, from: (f32, f32), to: (f32, f32), stroke: i64| {
        line_at(core, Vector2::new(from.0, from.1), Vector2::new(to.0, to.1), 17, stroke)
    };
    // 9.5 m lays ten modules whose outer faces sit exactly on the drawn ends.
    assert_eq!(lay(&mut core, (0.0, 0.0), (9.5, 0.0), 1), 10);
    let a = row(&core);
    assert!(a.iter().any(|&(x, _)| (x - 0.5).abs() < 1e-4));
    assert!(a.iter().any(|&(x, _)| (x - 9.0).abs() < 1e-4));
    // A corner drawn 0.5 m off the row's end moves onto it and runs on by half the low hedge's
    // 0.6 m width, so its first module's back face is flush with the first row's outer side.
    assert_eq!(hedge_end_at(&core, Vector2::new(9.8, 0.4)), Vector2::new(9.5, 0.0));
    assert_eq!(lay(&mut core, (9.8, 0.4), (9.5, 6.0), 2), 7);
    let b = row(&core);
    assert!(b.iter().all(|&(x, _)| (x - 9.5).abs() < 1e-4), "{b:?}");
    let z_min = b.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    assert!((z_min - (-0.3 + 0.5)).abs() < 1e-4, "{b:?}");
    // The buried corner is no longer an end: an end drawn beside the row's inner faces lands on
    // its side, not on the nearest face, and a T-junction runs on into the row it meets.
    assert_eq!(hedge_end_at(&core, Vector2::new(3.4, 0.5)), Vector2::new(3.4, 0.0));
    assert_eq!(lay(&mut core, (3.4, 5.0), (3.4, 0.5), 3), 6);
    let c = row(&core);
    let z_min = c.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    assert!((z_min - 0.2).abs() < 1e-4, "{c:?}");
    // Redrawing the first row snaps onto its own ends and stacks nothing.
    assert_eq!(lay(&mut core, (0.2, 0.3), (9.3, -0.2), 4), 0);
}

#[test]
fn hedge_rows_merge_into_runs_that_stop_at_their_ends_and_at_sixteen_metres() {
    let mut core = core();
    core.vegetation.config.enabled = false;
    // A 20 m row and a crossing 5 m row; the crossing row is its own run.
    assert_eq!(line_at(&mut core, Vector2::new(0.0, 0.0), Vector2::new(20.0, 0.0), 17, 1), 20);
    assert_eq!(line_at(&mut core, Vector2::new(30.0, 2.0), Vector2::new(30.0, 7.0), 18, 2), 5);
    let runs = hedge_runs::hedge_runs(&core, Vector2::new(-64.0, -64.0), 128.0);
    let mut runs: Vec<_> = runs.chunks_exact(7).map(|r| r.to_vec()).collect();
    runs.sort_by(|a, b| a[0].total_cmp(&b[0]));
    // The long row splits at 16 m and the two pieces cover exactly the drawn 20 m.
    assert_eq!(runs.len(), 3, "{runs:?}");
    assert!((runs[0][4] - 16.0).abs() < 1e-4 && (runs[0][0] - 8.0).abs() < 1e-4, "{runs:?}");
    assert!((runs[1][4] - 4.0).abs() < 1e-4 && (runs[1][0] - 18.0).abs() < 1e-4, "{runs:?}");
    assert_eq!((runs[0][6], runs[2][6]), (0.0, 1.0));
    assert!((runs[2][4] - 5.0).abs() < 1e-4 && (runs[2][2] - 4.5).abs() < 1e-4, "{runs:?}");
    // A patch only draws the modules whose centres it contains.
    let left = hedge_runs::hedge_runs(&core, Vector2::new(-64.0, -64.0), 74.0);
    assert_eq!(left.chunks_exact(7).map(|r| r[4]).sum::<f32>(), 10.0);
}

// Hedge modules standing anywhere in the edit store.
fn hedge_modules(core: &SimCore) -> Vec<Plant> {
    core.vegetation_edits
        .sorted_cells()
        .into_iter()
        .flat_map(|cell| core.vegetation_edits.cell(cell).1.to_vec())
        .filter(|plant| brush::hedge_index(plant.species, plant.variant).is_some())
        .collect()
}

// A yard event lining the square lot from `min` to `max` on all four sides.
fn square_yard(key: (u64, u32), min: Vector2, max: Vector2) -> YardEvent {
    use crate::simulation::buildings::allocator::yard::YardHedgeRowWorld;
    let corners = [min, Vector2::new(max.x, min.y), max, Vector2::new(min.x, max.y)];
    YardEvent::Placed {
        key,
        hedge: Some(crate::assets::asset::YardHedgeKind::Medium),
        rows: (0..4)
            .map(|i| YardHedgeRowWorld {
                from: corners[i],
                to: corners[(i + 1) % 4],
                join_from: true,
                join_to: true,
            })
            .collect(),
        planting: Vec::new(),
    }
}

#[test]
fn a_yard_hedge_shares_its_neighbours_line_and_leaves_with_its_building_unless_edited() {
    use crate::simulation::buildings::allocator::yard::YardEvent as Event;
    let mut core = core();
    core.vegetation.config.enabled = false;
    // Two 20 m yards one metre apart: the second shares the first's hedge on the line between.
    core.allocator.pending_yards.push(square_yard((1, 0), Vector2::new(0.0, 0.0), Vector2::new(20.0, 20.0)));
    core.allocator.pending_yards.push(square_yard((2, 0), Vector2::new(21.0, 0.0), Vector2::new(41.0, 20.0)));
    publish_yards(&mut core);
    let both = hedge_modules(&core);
    let first = core.vegetation_edits.take_yard_hedge((1, 0)).unwrap();
    let second = core.vegetation_edits.take_yard_hedge((2, 0)).unwrap();
    assert_eq!(both.len(), first.len() + second.len());
    // Its own side on that line was the first yard's hedge, so it laid none there; its front and
    // back rows ran on to the first yard's corners instead, joining the two yards' hedges.
    let on_shared_line = |plant: &Plant| plant.yaw.sin().abs() > 0.9 && plant.x < 21.6;
    assert!(!second.iter().any(|(_, plant)| on_shared_line(plant)), "{second:?}");
    assert!(second.iter().any(|(_, plant)| plant.yaw.sin().abs() < 0.1 && plant.x < 21.0));
    assert!(second.len() < first.len() - 15, "{} {}", first.len(), second.len());
    core.vegetation_edits.record_yard_hedge((1, 0), first.clone());
    core.vegetation_edits.record_yard_hedge((2, 0), second.clone());
    // The first yard goes whole; the second keeps every module it laid.
    core.allocator.pending_yards.push(Event::Removed((1, 0)));
    publish_yards(&mut core);
    assert_eq!(hedge_modules(&core).len(), second.len());
    // A yard whose hedge the player cut keeps the rest when its building goes, and forgets it.
    let (cell, cut) = second[3];
    core.vegetation_edits.remove_added(cell, |plant| (*plant == cut).then_some(0));
    core.allocator.pending_yards.push(Event::Removed((2, 0)));
    publish_yards(&mut core);
    assert_eq!(hedge_modules(&core).len(), second.len() - 1);
    assert!(core.vegetation_edits.take_yard_hedge((2, 0)).is_none());
}

#[test]
fn an_authored_plant_takes_a_yards_lawn_but_not_its_walls_or_paving() {
    use crate::simulation::buildings::allocator::BuildingSiteClient;
    let mut core = core();
    core.vegetation.config.enabled = false;
    let square = |a: f32, b: f32| {
        vec![Vector2::new(a, a), Vector2::new(a, b), Vector2::new(b, b), Vector2::new(b, a)]
    };
    let house = square(2.0, 10.0);
    core.allocator.building_sites.push(BuildingSiteClient {
        foundation_mesh: Default::default(),
        structure_world: vec![[house[0], house[1], house[2], house[3]]],
        planting_world: Vec::new(),
        footprint_world: square(0.0, 20.0),
        lot_footprint_world: [Vector2::ZERO; 4],
        support_height_m: 0.0,
        surfaces: vec![crate::simulation::buildings::allocator::BuildingSiteSurfaceClient {
            material: crate::assets::SiteSurfaceMaterial::Asphalt,
            name: String::new(),
            vertices_world: square(12.0, 14.0),
        }],
    });
    let shrub = |x: f32, z: f32| Plant {
        x,
        z,
        yaw: 0.0,
        scale: 1.0,
        species: SPECIES_BUSH as u8,
        variant: brush::LANDSCAPE_FIRST_VARIANT + 1,
    };
    assert!(placement::authored_clear(&core, &shrub(15.0, 15.0)), "the lawn takes a shrub");
    assert!(!placement::authored_clear(&core, &shrub(6.0, 6.0)), "the house does not");
    assert!(!placement::authored_clear(&core, &shrub(13.0, 13.0)), "nor its paving");
    // Wild vegetation still keeps off the whole flat support.
    assert!(!placement_clear(&core, 15.0, 15.0, VegetationLayer::Understory));
}
