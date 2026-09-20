// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic edit, persistence, patch ownership and bounded-cost regressions.

use super::*;
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
    scatter_layer(
        core,
        Vector2::new(-255.0, -255.0),
        510.0,
        cell_m,
        if canopy { CANOPY_SALT } else { UNDERSTORY_SALT },
        canopy,
    )
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
fn bulldoze_plant_lookup_is_bounded_deterministic_and_refuses_coincident_plants() {
    let mut core = core();
    core.vegetation.config.enabled = false;
    assert!(add_at(&mut core, Vector2::new(1.0, 1.0), 2));
    assert!(add_at(&mut core, Vector2::new(3.0, 1.0), 3));
    let picked = plant_at(&core, Vector2::new(2.0, 1.0)).unwrap();
    assert_eq!((picked.0.x, picked.0.z, picked.0.species), (1.0, 1.0, 2));
    assert!(picked.1 > 0.0);
    assert_eq!(plant_at(&core, Vector2::new(2.0, 1.0)), Some(picked));
    assert_eq!(plant_at(&core, Vector2::new(1.0, 1.0)), Some(picked));
    assert!(plant_at(&core, Vector2::new(1.0, 5.01)).is_none());
    assert!(plant_at(&core, Vector2::new(f32::NAN, 1.0)).is_none());
    assert!(add_at(&mut core, Vector2::new(1.0, 1.0), 0));
    assert!(plant_at(&core, Vector2::new(1.0, 1.0)).is_none());
    // Declining a bulldoze target must not alter the vegetation brush's area clear.
    assert_eq!(remove_at(&mut core, Vector2::new(1.0, 1.0), 0.0, 0), 2);
    assert_eq!(plant_at(&core, Vector2::new(3.0, 1.0)).unwrap().0.species, 3);
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
fn vegetation_paint_fills_rejected_candidates_and_leaves_generated_cells_alone() {
    let mut core = core();
    let generated = first_candidate(&core, true);
    assert_eq!(
        paint_at(&mut core, Vector2::new(generated.x, generated.z), 0.01, 1, 0),
        0
    );
    assert_eq!(core.vegetation_edits.len(), 0);
    let rejected = first_candidate(&core, false);
    // A point elsewhere in the same cell must not occupy the brush candidate's position.
    assert!(add_at(
        &mut core,
        Vector2::new(rejected.x + 1.0, rejected.z),
        3
    ));
    let before = scatter(&core, true);
    assert_eq!(
        paint_at(&mut core, Vector2::new(rejected.x, rejected.z), 0.01, 1, 0),
        1
    );
    let after = scatter(&core, true);
    assert_eq!(after.len(), before.len() + 6);
    assert!(after.chunks_exact(6).any(|r| r[0] == rejected.x + 255.0
        && r[2] == rejected.z + 255.0
        && r[3] == rejected.yaw
        && r[4] == rejected.scale
        && r[5] == 1.0));
    assert_eq!(
        paint_at(&mut core, Vector2::new(rejected.x, rejected.z), 0.01, 1, 0),
        0
    );
    assert_eq!(core.vegetation_edits.len(), 1);
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
    assert_eq!(paint_at(&mut core, pos, 0.01, i64::from(p.species), 0), 1);
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
    assert_eq!(paint_at(&mut core, pos, 0.01, i64::from(painted), 0), 1);
    let standing: Vec<_> = scatter(&core, true)
        .chunks_exact(6)
        .filter(|r| r[0] == p.x + 255.0 && r[2] == p.z + 255.0)
        .map(|r| r[5] as u8)
        .collect();
    assert_eq!(standing, vec![painted]);
    // The brush is idempotent: a second pass must not stack a second rock on the same cell.
    assert_eq!(paint_at(&mut core, pos, 0.01, i64::from(painted), 0), 0);
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
    assert_eq!(records[0], 0.0);
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
    core.invalidate_vegetation_over((
        Vector2::new(-120.0, -20.0),
        Vector2::new(-80.0, 20.0),
    ));
    assert_eq!(
        core.vegetation_edits.patch_generation(covered),
        covered_before + 1
    );
    assert_eq!(core.vegetation_edits.patch_generation(remote), remote_before);

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
    assert!(add_at(&mut core, Vector2::new(-1.0, 1.0), 1));
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
        assert_eq!(paint_at(&mut core, Vector2::ZERO, 8.0, option, 0), 0);
    }
    for radius in [-1.0, f32::NAN, f32::INFINITY, 1025.0] {
        assert_eq!(remove_at(&mut core, Vector2::ZERO, radius, 0), 0);
        assert_eq!(paint_at(&mut core, Vector2::ZERO, radius, 0, 0), 0);
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
    assert_eq!(paint_at(&mut core, Vector2::ZERO, 32.0, 0, 0), 0);
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
    assert!(paint_at(&mut core, Vector2::ZERO, 45.0, 1, 0) > 0);
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
        let expected_added = paint_at(&mut core, center, 64.0, 0, 0);
        group.bench_function(format!("paint_{background}"), |b| {
            b.iter_custom(|iterations| {
                let mut elapsed = std::time::Duration::ZERO;
                for _ in 0..iterations {
                    remove_at(&mut core, center, 64.0, 0);
                    let start = std::time::Instant::now();
                    let added = black_box(paint_at(&mut core, center, 64.0, 0, 0));
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
        let painted = paint_at(&mut core, center, 255.0, 0, 0);
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
    let expected = paint_at(&mut core, Vector2::ZERO, 64.0, 0, 0);
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
                let added = paint_at(&mut core, Vector2::ZERO, 64.0, 0, 0);
                elapsed += start.elapsed();
                assert_eq!(added, expected);
            }
            elapsed
        })
    });
    criterion.final_summary();
}

#[test]
fn vegetation_brush_fills_fine_grid_in_occupied_canopy_cells_and_is_idempotent() {
    let mut core = core();
    let before = scatter(&core, true);
    let count = paint_at(&mut core, Vector2::ZERO, 64.0, 3, 0);
    assert!(
        (800..880).contains(&count),
        "64 m disc must add about 625 stems/ha: {count}"
    );
    let after = scatter(&core, true);
    assert_eq!(after.len(), before.len() + count * 6);
    for cell in disc_cells(
        Vector2::ZERO,
        64.0,
        BRUSH_SPACING_M,
        VegetationLayer::Canopy,
    )
    .collect::<Vec<_>>()
    {
        let (_, salt) = grid(&core, VegetationLayer::Canopy);
        let [x, z, _, _] = brush_candidate(cell.x, cell.z, salt);
        if Vector2::new(x, z).length_squared() > 64.0 * 64.0 {
            continue;
        }
        let owner = cell_at(
            Vector2::new(x, z),
            VegetationLayer::Canopy,
            core.vegetation.canopy_cell_m,
        );
        assert!(
            core.vegetation_edits
                .cell(owner)
                .1
                .iter()
                .any(|p| p.x == x && p.z == z && p.species == 3)
        );
    }
    assert_eq!(paint_at(&mut core, Vector2::ZERO, 64.0, 3, 0), 0);
    assert_eq!(scatter(&core, true), after);
    // Removing the fine-grid plants must use those same canopy identities.
    remove_at(&mut core, Vector2::ZERO, 64.0, 0);
    assert!(
        scatter(&core, true)
            .chunks_exact(6)
            .all(|r| Vector2::new(r[0] - 255.0, r[2] - 255.0).length_squared() > 64.0 * 64.0)
    );
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
        pool.install(|| paint_at(&mut core, Vector2::new(-20.0, 15.0), 64.0, 2, 0));
        let records = scatter(&core, true);
        if let Some(expected) = &reference {
            assert_eq!(&records, expected);
        }
        reference = Some(records);
    }
    for canopy_m in [8.0, 16.0, 32.0] {
        let mut core = core();
        core.vegetation.config.enabled = false;
        core.vegetation.canopy_cell_m = canopy_m;
        paint_at(&mut core, Vector2::ZERO, 16.0, 1, 0);
        let (_, salt) = grid(&core, VegetationLayer::Canopy);
        let [x, z, _, _] = brush_candidate(-1, 0, salt);
        let owner = cell_at(Vector2::new(x, z), VegetationLayer::Canopy, canopy_m);
        assert!(
            core.vegetation_edits
                .cell(owner)
                .1
                .iter()
                .any(|p| p.x == x && p.z == z)
        );
        assert!(
            scatter(&core, true)
                .chunks_exact(6)
                .any(|r| r[0] == x + 255.0 && r[2] == z + 255.0)
        );
    }
}

#[test]
fn vegetation_brush_bound_and_small_jitter() {
    let mut core = core();
    assert_eq!(
        paint_at(&mut core, Vector2::ZERO, MAX_PAINT_RADIUS_M + 0.01, 0, 0),
        0
    );
    assert_eq!(core.vegetation_edits.len(), 0);
    let fine = disc_cells(
        Vector2::ZERO,
        MAX_PAINT_RADIUS_M,
        BRUSH_SPACING_M,
        VegetationLayer::Canopy,
    )
    .len();
    let canopy = disc_cells(
        Vector2::ZERO,
        MAX_PAINT_RADIUS_M,
        8.0,
        VegetationLayer::Canopy,
    )
    .len();
    assert_eq!(fine + canopy, 20_866);
    let added = paint_at(&mut core, Vector2::ZERO, MAX_PAINT_RADIUS_M, 0, 0);
    assert!(added > 12_000 && added <= 20_866);
    for x in -100..100 {
        let p = brush_candidate(x, -7, 123);
        assert!((p[0] - (x as f32 + 0.5) * BRUSH_SPACING_M).abs() <= 0.201);
        assert!((p[1] - (-7.0 + 0.5) * BRUSH_SPACING_M).abs() <= 0.201);
        assert_eq!(p, brush_candidate(x, -7, 123));
        assert_ne!(p, brush_candidate(x, -7, 124));
    }
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
    assert!(paint_at(&mut core, center, 48.0, 1, 0) > 0);
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
    assert!(paint_at(&mut core, center, 48.0, 1, 0) > 0);

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
    assert!(paint_at(&mut spruce_core, Vector2::ZERO, 64.0, SPRUCE, 0) > 0);
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
    assert!(paint_at(&mut pine_core, Vector2::ZERO, 64.0, PINE, 0) > 0);
    let pines: Vec<_> = planted(&pine_core).into_iter().filter(|p| p.1 != 0).collect();
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
fn an_unpinned_preset_plants_exactly_what_it_planted_before_presets_existed() {
    let mut core = core();
    assert!(paint_at(&mut core, Vector2::ZERO, 64.0, CONIFER, 0) > 0);
    for (_, variant, scale) in planted(&core) {
        assert_eq!(variant, 0, "conifer leaves the mesh to the appearance seed");
        assert!((0.75..=1.35).contains(&scale));
    }
}

#[test]
fn a_thinned_preset_plants_fewer_trees_and_thins_to_the_same_ones_every_time() {
    let mut dense = core();
    let mut sparse = core();
    let planted_dense = paint_at(&mut dense, Vector2::ZERO, 64.0, CONIFER, 0);
    let planted_sparse = paint_at(&mut sparse, Vector2::ZERO, 64.0, MEADOW, 0);
    assert!(planted_dense > 0 && planted_sparse > 0);
    assert!(
        (planted_sparse as f32) < planted_dense as f32 * 0.35,
        "a meadow keeps a tenth of the lattice, not most of it: {planted_sparse} of {planted_dense}"
    );

    // Thinning is a pure function of the cell, so a repeat finds every point already taken.
    assert_eq!(
        paint_at(&mut sparse, Vector2::ZERO, 64.0, MEADOW, 0),
        0,
        "a repeated stroke must not thin to a different set and fill the gaps"
    );
    let mut repeat = core();
    assert_eq!(
        paint_at(&mut repeat, Vector2::ZERO, 64.0, MEADOW, 0),
        planted_sparse
    );
    assert_eq!(planted(&repeat), planted(&sparse));
}

#[test]
fn a_dwarfing_preset_moves_every_plant_into_its_own_scale_band() {
    let mut core = core();
    let untouched: HashSet<_> = planted(&core)
        .into_iter()
        .map(|p| (p.0, p.1, p.2.to_bits()))
        .collect();
    assert!(paint_at(&mut core, Vector2::ZERO, 64.0, NORTHERN_DWARF, 0) > 0);
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
    assert!(paint_at(&mut core, Vector2::ZERO, 64.0, MIXED_FOREST, 0) > 0);
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
        paint_at(&mut core, pos, 0.01, i64::from(standing.species), 0),
        1
    );
    assert_eq!(core.vegetation_edits.len(), 0);

    assert_eq!(remove_at(&mut core, pos, 0.01, 0), 1);
    let named = if standing.species == 0 { SPRUCE } else { PINE };
    assert_eq!(paint_at(&mut core, pos, 0.01, named, 0), 1);
    assert_eq!(
        core.vegetation_edits.len(),
        1,
        "a player who named the tree must get that tree authored over the tombstone"
    );
}
