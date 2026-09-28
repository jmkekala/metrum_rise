// SPDX-License-Identifier: GPL-2.0-only

//! Operation-isolated heap phases for the populated cell fixture; setup is outside markers.

use super::*;
use crate::simulation::zoning::cells::{CellKey, CellSelectionShape, CellStore, GridFrame};
use glam::DVec2;
use std::hint::black_box;

#[test]
#[ignore = "heap trace acceptance; run through benchmarks/zoning_memory.py"]
fn populated_cell_memory() {
    measure_populated_road_plan_scaling(false, false, false, CellScenario::Memory);
}

// glibc memusage records requested live heap after each successful heap event. A unique
// allocation/free pair bounds each phase without a custom allocator, FFI or timestamp guesses.
// Printing and marker allocation are outside the measured interior; returned products stay live.
fn phase<T>(
    ordinal: &mut usize,
    name: &str,
    background: usize,
    operations: usize,
    peak_budget: usize,
    zero_events: bool,
    action: impl FnOnce() -> T,
) -> T {
    let marker_bytes = 8 * 1024 * 1024 + *ordinal * 64 + 37;
    *ordinal += 1;
    println!(
        "CELL_HEAP_PHASE {}",
        serde_json::json!({
            "name": name, "background": background, "operations": operations,
            "marker_bytes": marker_bytes, "peak_budget": peak_budget, "zero_events": zero_events,
        })
    );
    let marker = Vec::<u8>::with_capacity(black_box(marker_bytes));
    black_box(&marker);
    let result = action();
    black_box(&marker);
    drop(marker);
    result
}

/// Verifies empty, retained-allocation and resize/free phase signatures before fixture work.
pub(super) fn calibrate(ordinal: &mut usize) {
    phase(ordinal, "calibrate_empty", 0, 1, 0, true, || black_box(()));
    let retained = phase(ordinal, "calibrate_retained", 0, 1, 12345, false, || {
        black_box(vec![0u8; 12345])
    });
    drop(retained);
    phase(ordinal, "calibrate_resize", 0, 1, 513, false, || {
        let mut value = black_box(vec![0u8; 257]);
        value.reserve_exact(256);
        black_box(&value);
        value.truncate(128);
        value.shrink_to_fit();
        black_box(&value);
        drop(value);
    });
}

/// Measures fixed native query/edit work against the parent's increasing distant population.
pub(super) fn measure(
    core: &mut SimCore,
    background: usize,
    ordinal: &mut usize,
) -> Vec<Vec<CellKey>> {
    core.prepare_cell_lots_internal();
    for chunk in [(-1, -1), (0, -1)] {
        assert!(core.prepare_cell_chunk_internal(chunk));
    }
    let point = DVec2::new(25.0, -25.0);
    let key = core.zoning.cells.pick(point).unwrap();
    assert_eq!(core.zoning.cells.profile(key), Some(0));
    assert!(!core.zoning.has_dirty_cell_lots());
    phase(ordinal, "idle_lots", background, 4096, 0, true, || {
        for _ in 0..4096 {
            black_box(core.prepare_cell_lots_internal());
        }
    });
    phase(ordinal, "idle_chunk", background, 256, 0, true, || {
        for _ in 0..256 {
            assert!(black_box(core.prepare_cell_chunk_internal((0, -1))));
        }
    });
    phase(ordinal, "pick", background, 4096, 0, true, || {
        for _ in 0..4096 {
            assert_eq!(
                black_box(core.zoning.cells.pick(black_box(point))),
                Some(key)
            );
        }
    });
    let mut products = Vec::new();
    for (name, shape, path) in [
        ("cell_preview", CellSelectionShape::Cell, vec![point]),
        (
            "marquee_preview",
            CellSelectionShape::Marquee,
            vec![point, DVec2::new(75.0, -55.0)],
        ),
        (
            "brush_preview",
            CellSelectionShape::Brush { radius_m: 20.0 },
            vec![point, DVec2::new(65.0, -25.0)],
        ),
        ("fill_preview", CellSelectionShape::Fill, vec![point]),
    ] {
        let expected = core.preview_cell_selection_internal(shape, &path);
        let result = phase(ordinal, name, background, 1, 256 * 1024, false, || {
            black_box(core.preview_cell_selection_internal(shape, &path))
        });
        assert_eq!(result.selection.cells, expected.selection.cells);
        assert!(!result.selection.cells.is_empty());
        products.push(result.selection.cells);
    }
    let undo_len = core.undo_stack.len();
    let paint = core.preview_cell_selection_internal(CellSelectionShape::Cell, &[point]);
    assert!(phase(
        ordinal,
        "paint",
        background,
        1,
        256 * 1024,
        false,
        || { core.apply_cell_selection_internal(&paint, 1) }
    ));
    let erase = core.preview_cell_selection_internal(CellSelectionShape::Cell, &[point]);
    assert!(phase(
        ordinal,
        "erase",
        background,
        1,
        256 * 1024,
        false,
        || { core.apply_cell_selection_internal(&erase, 0) }
    ));
    assert!(phase(
        ordinal,
        "undo_erase",
        background,
        1,
        256 * 1024,
        false,
        || { core.undo_action_internal() }
    ));
    assert!(phase(
        ordinal,
        "undo_paint",
        background,
        1,
        256 * 1024,
        false,
        || { core.undo_action_internal() }
    ));
    assert_eq!(core.undo_stack.len(), undo_len);
    assert_eq!(core.zoning.cells.profile(key), Some(0));
    core.prepare_cell_lots_internal();
    products
}

#[test]
#[ignore = "large local fill heap trace; run through benchmarks/zoning_memory.py"]
fn large_cell_fill_memory() {
    let mut ordinal = 0;
    calibrate(&mut ordinal);
    for columns in [128, 512, 1900] {
        // A six-deep painted frontage up to 19 km long; this measures store traversal,
        // independently of native cold-chunk generation and Godot bridge buffer conversion.
        let count = columns as usize * 6;
        let (cells, grid) = phase(
            &mut ordinal,
            "painted_store",
            count,
            1,
            256 * 1024 + 128 * count,
            false,
            || {
                let mut cells = CellStore::default();
                let grid =
                    cells.register_frame(GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap());
                for x in 0..columns {
                    for y in 0..6 {
                        assert!(cells.restore_saved_cell(CellKey { grid, x, y }, 1));
                    }
                }
                (cells, grid)
            },
        );
        let selected = phase(
            &mut ordinal,
            "large_fill",
            count,
            1,
            256 * 1024 + 128 * count,
            false,
            || cells.select(CellSelectionShape::Fill, &[DVec2::splat(5.0)]),
        );
        assert_eq!(selected.cells.len(), count);
        assert!(
            selected
                .cells
                .iter()
                .all(|key| key.grid == grid && key.y < 6)
        );
    }
}
