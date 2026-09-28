// SPDX-License-Identifier: GPL-2.0-only

//! Cold gesture queries must produce the same geometry and selections as fully generated views.

use super::*;

mod dirty;

fn signature(store: &CellStore, selection: &CellSelection) -> Vec<(GridFrame, i32, i32)> {
    let mut result: Vec<_> = selection
        .cells
        .iter()
        .map(|key| (store.frame(key.grid).unwrap(), key.x, key.y))
        .collect();
    result.sort_unstable();
    result
}

#[test]
fn fill_materializes_across_chunks_and_stops_at_a_painted_barrier() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 1600.0, 0.0);
    road(&mut graph, a, b);
    let config = WorldConfig::default();
    let mut cells = CellStore::default();
    let first = cells.select_generated(
        &graph,
        &config,
        CellSelectionShape::Fill,
        &[DVec2::new(25.0, 20.0)],
        |_| false,
    );
    assert_eq!(first.cells.len(), 160 * 6);
    assert_eq!(first.revision, cells.revision());
    let barrier = cells.select_generated(
        &graph,
        &config,
        CellSelectionShape::Marquee,
        &[DVec2::new(800.1, 5.1), DVec2::new(810.0, 65.0)],
        |_| false,
    );
    assert_eq!(barrier.cells.len(), 6);
    cells.paint(&barrier, 2).unwrap();
    let bounded = cells.select_generated(
        &graph,
        &config,
        CellSelectionShape::Fill,
        &[DVec2::new(25.0, 20.0)],
        |_| false,
    );
    assert_eq!(bounded.cells.len(), 80 * 6);
    assert!(
        bounded
            .cells
            .iter()
            .all(|&key| cells.profile(key) == Some(0))
    );
    let reverse = cells.select_generated(
        &graph,
        &config,
        CellSelectionShape::Fill,
        &[DVec2::new(1505.0, 20.0)],
        |_| false,
    );
    assert_eq!(reverse.cells.len(), 79 * 6);
}

#[test]
fn gesture_and_view_order_do_not_change_competing_grid_ownership() {
    let mut graph = RegionGraph::new();
    for (a, b) in [
        ((-800.0, 450.0), (800.0, 570.0)),
        ((-700.0, 565.0), (900.0, 460.0)),
    ] {
        let a = node(&mut graph, a.0, a.1);
        let b = node(&mut graph, b.0, b.1);
        road(&mut graph, a, b);
    }
    let config = WorldConfig::default();
    let bounds = CellBounds {
        min: DVec2::new(-1000.0, 300.0),
        max: DVec2::new(1000.0, 750.0),
    };
    let mut complete = CellStore::default();
    complete.generate_in_bounds(&graph, &config, bounds, |_| false);
    let seed = keys(&complete, bounds).into_iter().next().unwrap();
    let start = complete
        .frame(seed.grid)
        .unwrap()
        .world(f64::from(seed.x) + 0.5, f64::from(seed.y) + 0.5);
    for shape in [
        CellSelectionShape::Cell,
        CellSelectionShape::Marquee,
        CellSelectionShape::Fill,
        CellSelectionShape::Brush { radius_m: 25.0 },
    ] {
        let path = [start, DVec2::new(700.0, 520.0)];
        let expected = complete.select(shape, &path);
        for warm in [false, true] {
            let mut actual = CellStore::default();
            if warm {
                actual.generate_in_bounds(
                    &graph,
                    &config,
                    CellBounds {
                        min: DVec2::new(500.0, 500.0),
                        max: DVec2::new(600.0, 600.0),
                    },
                    |_| false,
                );
            }
            let selection = actual.select_generated(&graph, &config, shape, &path, |_| false);
            assert_eq!(
                signature(&actual, &selection),
                signature(&complete, &expected),
                "{shape:?}, warm={warm}"
            );
        }
    }
}

#[test]
fn invalid_queries_never_materialize_cells() {
    let mut cells = CellStore::default();
    let graph = RegionGraph::new();
    for (shape, point) in [
        (CellSelectionShape::Cell, DVec2::splat(f64::NAN)),
        (
            CellSelectionShape::Brush {
                radius_m: f64::INFINITY,
            },
            DVec2::ZERO,
        ),
        (CellSelectionShape::Brush { radius_m: -1.0 }, DVec2::ZERO),
    ] {
        let result =
            cells.select_generated(&graph, &WorldConfig::default(), shape, &[point], |_| false);
        assert!(result.cells.is_empty());
        assert_eq!(result.revision, 0);
    }
}

#[test]
fn cached_geometry_and_overlay_versions_are_local_to_the_changed_chunk() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 1600.0, 0.0);
    road(&mut graph, a, b);
    let mut cells = CellStore::default();
    let config = WorldConfig::default();
    for point in [DVec2::new(25.0, 20.0), DVec2::new(1205.0, 20.0)] {
        cells.select_generated(&graph, &config, CellSelectionShape::Cell, &[point], |_| {
            false
        });
    }
    let remote = cells.chunk_state((2, 0));
    let local = cells.chunk_state((0, 0));
    assert!(local.1 && remote.1);
    let selection = cells.select_generated(
        &graph,
        &config,
        CellSelectionShape::Cell,
        &[DVec2::new(25.0, 20.0)],
        |_| panic!("idle query must reuse the completed chunk"),
    );
    cells.paint(&selection, 1).unwrap();
    assert_ne!(cells.chunk_state((0, 0)).0, local.0);
    assert!(cells.chunk_state((0, 0)).1);
    assert_eq!(cells.chunk_state((2, 0)), remote);
    cells.invalidate_generated_cells(CellBounds {
        min: DVec2::ZERO,
        max: DVec2::splat(50.0),
    });
    assert!(!cells.chunk_state((0, 0)).1);
    assert_eq!(cells.chunk_state((2, 0)), remote);
    cells.select_generated(
        &graph,
        &config,
        CellSelectionShape::Cell,
        &[DVec2::new(25.0, 20.0)],
        |corners| corners.iter().all(|p| p.x < 50.0),
    );
    assert_eq!(
        cells.profile(selection.cells[0]),
        Some(1),
        "paint remains pinned when its supporting empty geometry changes"
    );
    assert!(cells.pick(DVec2::new(15.0, 20.0)).is_none());
    assert_eq!(cells.chunk_state((2, 0)), remote);
    let mut owned = std::collections::HashSet::new();
    for x in -1..=4 {
        for y in -1..=0 {
            cells.visit_chunk_cells((x, y), |key| {
                assert!(owned.insert(key), "crossing cells upload once")
            });
        }
    }
    assert_eq!(
        owned.len(),
        keys(
            &cells,
            CellBounds {
                min: DVec2::new(-10.0, -100.0),
                max: DVec2::new(1700.0, 100.0)
            }
        )
        .len()
    );
}
