// SPDX-License-Identifier: GPL-2.0-only

//! Partial invalidation must retain full-rebuild geometry, frontage and occupied claims.

use super::*;

fn box_at(min: f64, max: f64) -> CellBounds {
    CellBounds {
        min: DVec2::splat(min),
        max: DVec2::splat(max),
    }
}

#[test]
fn invalidation_unions_warm_bounds_without_completing_cold_chunks() {
    let mut store = CellStore::default();
    store.complete_generated_chunk((0, 0), CellStore::chunk_bounds((0, 0)));
    store.complete_generated_chunk((-1, -1), CellStore::chunk_bounds((-1, -1)));
    store.invalidate_generated_cells(box_at(-20.0, 30.0));
    store.invalidate_generated_cells(box_at(400.0, 600.0));
    assert_eq!(
        store.chunk_generation_bounds((0, 0)),
        Some(box_at(0.0, 512.0))
    );
    assert_eq!(
        store.chunk_generation_bounds((-1, -1)),
        Some(box_at(-20.0, 0.0))
    );
    assert_eq!(
        store.chunk_generation_bounds((1, 1)),
        Some(box_at(512.0, 1024.0))
    );
    store.complete_generated_chunk((0, 0), CellStore::chunk_bounds((0, 0)));
    assert_eq!(store.chunk_generation_bounds((0, 0)), None);
    store.complete_generated_chunk((-1, -1), box_at(-20.0, 0.0));
    store.invalidate_generated_cells(box_at(10.0, 20.0));
    assert_eq!(
        store.chunk_generation_bounds((0, 0)),
        Some(box_at(10.0, 20.0))
    );
    store.invalidate_all_generated_cells();
    store.invalidate_generated_cells(box_at(10.0, 20.0));
    assert_eq!(
        store.chunk_generation_bounds((0, 0)),
        Some(box_at(0.0, 512.0))
    );
}

#[test]
fn small_dirty_batches_complete_neighbours_without_expanding_large_edits() {
    let mut store = CellStore::default();
    for x in -1..=0 {
        for y in -1..=0 {
            store.complete_generated_chunk((x, y), CellStore::chunk_bounds((x, y)));
        }
    }
    store.invalidate_generated_cells(box_at(-100.0, 150.0));
    let bounds = store.chunk_generation_bounds((0, 0)).unwrap();
    assert_eq!(bounds, box_at(-100.0, 150.0));
    store.complete_generated_chunk((0, 0), bounds);
    for x in -1..=0 {
        for y in -1..=0 {
            assert_eq!(store.chunk_generation_bounds((x, y)), None);
        }
    }
    store.invalidate_generated_cells(box_at(-300.0, 300.0));
    let bounds = store.chunk_generation_bounds((0, 0)).unwrap();
    assert_eq!(bounds, box_at(0.0, 300.0));
    store.complete_generated_chunk((0, 0), bounds);
    assert!(
        !store.chunk_state((-1, -1)).1,
        "partly covered work cannot become current"
    );
    assert!(!store.chunk_state((-1, 0)).1);
    assert!(!store.chunk_state((0, -1)).1);
    assert_eq!(
        store.chunk_generation_bounds((1, 1)),
        Some(box_at(512.0, 1024.0))
    );
}

fn settle(
    zoning: &mut ZoningSystem,
    graph: &RegionGraph,
    config: &WorldConfig,
    blocked: impl Fn(&[DVec2; 4]) -> bool + Sync,
) {
    // Materialize every chunk, including empty chunks and competing grids.
    for x in -2..2 {
        for y in -2..2 {
            zoning.cells.select_generated(
                graph,
                config,
                CellSelectionShape::Cell,
                &[DVec2::new(
                    x as f64 * 512.0 + 256.0,
                    y as f64 * 512.0 + 256.0,
                )],
                &blocked,
            );
        }
    }
}

fn same_products(actual: &CellStore, expected: &CellStore) {
    let bounds = box_at(-1000.0, 1000.0);
    let actual_keys = keys(actual, bounds);
    let expected_keys = keys(expected, bounds);
    assert_eq!(
        actual_keys, expected_keys,
        "partial and full rebuild retain the same cell addresses"
    );
    for key in actual_keys {
        assert_eq!(actual.frame(key.grid), expected.frame(key.grid));
        assert_eq!(actual.profile(key), expected.profile(key));
        assert_eq!(actual.lot(key), expected.lot(key));
        assert_eq!(
            actual.frontages(key),
            expected.frontages(key),
            "frontage for {key:?}"
        );
    }
}

#[test]
fn partial_paint_erase_and_external_exclusion_match_full_chunk_rebuilds() {
    for angle in [0.0_f64, 0.35] {
        for shape in 0..4 {
            let direction = DVec2::new(angle.cos(), angle.sin());
            let point = |x: f64, y: f64| direction * x + direction.perp() * y;
            let mut graph = RegionGraph::new();
            let mut add = |coordinates: &[(f64, f64)]| {
                let points: Vec<_> = coordinates
                    .iter()
                    .map(|&(x, y)| {
                        let p = point(x, y);
                        Vector3::new(p.x as f32, 0.0, p.y as f32)
                    })
                    .collect();
                let a = node(&mut graph, points[0].x, points[0].z);
                let b = node(
                    &mut graph,
                    points.last().unwrap().x,
                    points.last().unwrap().z,
                );
                polyline(&mut graph, a, b, points);
            };
            if shape == 3 {
                add(&[
                    (-280.0, -30.0),
                    (-140.0, 60.0),
                    (0.0, 80.0),
                    (140.0, 60.0),
                    (280.0, -30.0),
                ]);
            } else {
                add(&[(-280.0, 0.0), (280.0, 0.0)]);
                if shape == 1 {
                    add(&[(0.0, 0.0), (0.0, 280.0)]);
                } else if shape == 2 {
                    add(&[(-280.0, 85.0), (280.0, 55.0)]);
                }
            }
            let config = WorldConfig::default();
            let mut actual = ZoningSystem::new(&config);
            settle(&mut actual, &graph, &config, |_| false);
            let all = keys(&actual.cells, box_at(-1000.0, 1000.0));
            let seed = *all
                .iter()
                .min_by(|&&a, &&b| {
                    let centre = |key: CellKey| {
                        actual
                            .cells
                            .frame(key.grid)
                            .unwrap()
                            .world(key.x as f64 + 0.5, key.y as f64 + 0.5)
                    };
                    centre(a)
                        .distance_squared(point(25.0, 25.0))
                        .total_cmp(&centre(b).distance_squared(point(25.0, 25.0)))
                })
                .unwrap();
            let centre = actual
                .cells
                .frame(seed.grid)
                .unwrap()
                .world(seed.x as f64 + 0.5, seed.y as f64 + 0.5);
            for profile in [1, 2, 0] {
                let selection = actual.cells.select(CellSelectionShape::Cell, &[centre]);
                actual.paint_cells(&selection, profile, |_| false).unwrap();
                let mut expected = actual.clone();
                expected.cells.invalidate_all_generated_cells();
                settle(&mut actual, &graph, &config, |_| false);
                settle(&mut expected, &graph, &config, |_| false);
                same_products(&actual.cells, &expected.cells);
            }
            let selection = actual.cells.select(CellSelectionShape::Cell, &[centre]);
            actual.paint_cells(&selection, 1, |_| false).unwrap();
            let lot = CellLot::new(seed, 1, 1, CellFrontage::MinY).unwrap();
            assert!(actual.cells.claim_lot(lot, 99, 1));
            let selection = actual.cells.select(CellSelectionShape::Cell, &[centre]);
            actual.paint_cells(&selection, 0, |_| false).unwrap();
            // External exclusions affect empty candidates while erased occupied land stays pinned.
            let exclusion = CellBounds {
                min: centre - DVec2::splat(15.0),
                max: centre + DVec2::splat(15.0),
            };
            let blocked =
                |corners: &[DVec2; 4]| CellBounds::from_points(*corners).intersects(exclusion);
            actual.mark_cell_lots_dirty(exclusion);
            let mut expected = actual.clone();
            expected.cells.invalidate_all_generated_cells();
            settle(&mut actual, &graph, &config, blocked);
            settle(&mut expected, &graph, &config, blocked);
            same_products(&actual.cells, &expected.cells);
            assert_eq!(actual.cells.lot(seed), Some(99));
            assert_disjoint(&actual.cells, box_at(-1000.0, 1000.0));
        }
    }
}
