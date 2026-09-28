// SPDX-License-Identifier: GPL-2.0-only

//! Cell geometry, sparse spatial lookup, selection and atomic painting regressions.

mod generation;
mod lots;
mod reservations;

use super::geometry::{interiors_overlap, segment_intersects_cell};
use super::*;
use glam::DVec2;

fn rectangular_cells(width: i32, depth: i32) -> (CellStore, u64) {
    let mut store = CellStore::default();
    let grid = store.register_frame(GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap());
    for x in 0..width {
        for y in 0..depth {
            assert!(store.insert(CellKey { grid, x, y }));
        }
    }
    (store, grid)
}

#[test]
fn perpendicular_reversed_and_translated_frames_share_identity() {
    let origin = DVec2::new(5.0, 5.0);
    let frame = GridFrame::new(origin, DVec2::X, 10.0).unwrap();
    for direction in [DVec2::X, DVec2::Y, -DVec2::X, -DVec2::Y] {
        assert_eq!(frame, GridFrame::new(origin, direction, 10.0).unwrap());
        assert_eq!(
            frame,
            GridFrame::new(origin + DVec2::new(30.0, -20.0), direction, 10.0).unwrap()
        );
    }
    assert_ne!(
        frame,
        GridFrame::new(origin + DVec2::X, DVec2::X, 10.0).unwrap()
    );
    let axis = DVec2::new(0.8, 0.6);
    let rotated = GridFrame::new(origin, axis, 10.0).unwrap();
    assert_eq!(
        rotated,
        GridFrame::new(origin, DVec2::new(-axis.y, axis.x), 10.0).unwrap()
    );
    assert_eq!(
        rotated,
        GridFrame::new(origin + 40.0 * axis, axis, 10.0).unwrap()
    );
}

#[test]
fn rotated_squares_share_vertices_without_positive_area_overlap() {
    for axis in [DVec2::X, DVec2::new(0.8, 0.6), DVec2::new(1.0, 1.0)] {
        let frame = GridFrame::new(DVec2::new(-3.2, 6.7), axis, 10.0).unwrap();
        for x in [-900, -1, 0, 900] {
            let a = frame.corners(x, -17);
            let b = frame.corners(x + 1, -17);
            assert_eq!(a[1], b[0]);
            assert_eq!(a[2], b[3]);
            assert!(!interiors_overlap(&a, &b));
            assert!(interiors_overlap(&a, &a));
            for i in 0..4 {
                assert!((a[i].distance(a[(i + 1) % 4]) - 10.0).abs() < 2e-6);
            }
        }
    }
}

#[test]
fn canonical_overlap_detects_sub_millimetre_intrusion_and_containment() {
    let frame = GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap();
    let a = frame.corners(900, -900);
    let b = frame.corners(901, -900);
    assert!(!interiors_overlap(&a, &b));
    assert!(interiors_overlap(
        &a,
        &b.map(|point| point - DVec2::new(0.000001, 0.0))
    ));
    assert!(!interiors_overlap(
        &a,
        &b.map(|point| point + DVec2::new(0.000001, 0.0))
    ));
    let centre = frame.world(900.5, -899.5);
    assert!(interiors_overlap(
        &a,
        &a.map(|point| centre + (point - centre) * 0.1)
    ));
    let crossing = [
        DVec2::new(-1.0, 4.0),
        DVec2::new(11.0, 4.0),
        DVec2::new(11.0, 6.0),
        DVec2::new(-1.0, 6.0),
    ];
    assert!(interiors_overlap(&frame.corners(0, 0), &crossing));
}

#[test]
fn negative_coordinates_and_chunk_boundaries_pick_once() {
    let (mut store, grid) = rectangular_cells(1, 1);
    for (x, y) in [(-1, -1), (-52, -52), (51, 51), (52, 51)] {
        let key = CellKey { grid, x, y };
        store.insert(key);
        assert_eq!(
            store.pick(DVec2::new(
                f64::from(x) * 10.0 + 5.0,
                f64::from(y) * 10.0 + 5.0
            )),
            Some(key)
        );
    }
    let mut keys = Vec::new();
    store.visit_in_bounds(
        CellBounds {
            min: DVec2::splat(505.0),
            max: DVec2::splat(535.0),
        },
        |key| keys.push(key),
    );
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            CellKey { grid, x: 51, y: 51 },
            CellKey { grid, x: 52, y: 51 }
        ]
    );
    assert!(store.remove_unreserved(CellKey {
        grid,
        x: -52,
        y: -52
    }));
    assert_eq!(store.pick(DVec2::splat(-515.0)), None);
}

#[test]
fn single_click_at_shared_vertex_has_one_stable_target() {
    let (store, grid) = rectangular_cells(2, 2);
    let selection = store.select(CellSelectionShape::Cell, &[DVec2::splat(10.0)]);
    assert_eq!(selection.cells, vec![CellKey { grid, x: 0, y: 0 }]);
    assert_eq!(
        store.pick(DVec2::splat(10.0 + 0.4e-6)),
        Some(CellKey { grid, x: 0, y: 0 })
    );
}

#[test]
fn fill_crosses_storage_tiles_but_stops_at_gaps_profiles_and_grid_changes() {
    let (mut store, grid) = rectangular_cells(18, 2);
    for y in 0..2 {
        assert!(store.remove_unreserved(CellKey { grid, x: 10, y }));
    }
    let barrier = CellSelection {
        revision: store.revision(),
        cells: (0..2).map(|y| CellKey { grid, x: 3, y }).collect(),
    };
    store.paint(&barrier, 1).unwrap();
    let other = store.register_frame(GridFrame::new(DVec2::new(0.0, 1.0), DVec2::X, 10.0).unwrap());
    store.insert(CellKey {
        grid: other,
        x: 11,
        y: 2,
    });
    let diagonal = CellKey { grid, x: 18, y: 2 };
    store.insert(diagonal);
    let left = store.select(CellSelectionShape::Fill, &[DVec2::new(5.0, 5.0)]);
    assert_eq!(left.cells.len(), 6);
    let middle = store.select(CellSelectionShape::Fill, &[DVec2::new(45.0, 5.0)]);
    assert_eq!(middle.cells.len(), 12);
    assert!(middle.cells.iter().any(|key| key.x == 8));
    let right = store.select(CellSelectionShape::Fill, &[DVec2::new(115.0, 5.0)]);
    assert_eq!(right.cells.len(), 14);
    assert!(
        right
            .cells
            .iter()
            .all(|key| key.grid == grid && *key != diagonal)
    );
}

#[test]
fn continuous_cell_and_brush_strokes_do_not_skip_fast_input() {
    let (store, _) = rectangular_cells(100, 6);
    let short = [DVec2::new(5.0, 15.0), DVec2::new(995.0, 15.0)];
    let dense: Vec<_> = (0..100)
        .map(|x| DVec2::new(5.0 + f64::from(x) * 10.0, 15.0))
        .collect();
    for (shape, expected) in [
        (CellSelectionShape::Cell, 100),
        (CellSelectionShape::Brush { radius_m: 10.0 }, 300),
    ] {
        let a = store.select(shape, &short);
        let b = store.select(shape, &dense);
        assert_eq!(a, b);
        assert_eq!(a.cells.len(), expected);
    }
    let corners = GridFrame::new(DVec2::ZERO, DVec2::X, 10.0)
        .unwrap()
        .corners(0, 0);
    assert!(!segment_intersects_cell(
        &corners,
        DVec2::new(20.0, 0.0),
        DVec2::new(30.0, 0.0)
    ));
}

#[test]
fn marquee_uses_starting_frame_and_includes_other_grids_by_cell_centre() {
    let mut store = CellStore::default();
    let frame = GridFrame::new(DVec2::ZERO, DVec2::new(0.8, 0.6), 10.0).unwrap();
    let grid = store.register_frame(frame);
    for x in 0..4 {
        for y in 0..4 {
            store.insert(CellKey { grid, x, y });
        }
    }
    let other = store
        .register_frame(GridFrame::new(DVec2::new(1.0, 0.0), DVec2::new(0.8, 0.6), 10.0).unwrap());
    store.insert(CellKey {
        grid: other,
        x: 5,
        y: 0,
    });
    let selected = store.select(
        CellSelectionShape::Marquee,
        &[frame.world(0.1, 0.1), frame.world(6.0, 2.0)],
    );
    assert_eq!(selected.cells.len(), 9);
    assert!(selected.cells.contains(&CellKey {
        grid: other,
        x: 5,
        y: 0
    }));
    assert!(
        selected
            .cells
            .iter()
            .filter(|key| key.grid == grid)
            .all(|key| key.y < 2)
    );
}

#[test]
fn painting_and_erasing_share_selection_and_local_undo() {
    let (mut store, grid) = rectangular_cells(6, 6);
    let path = [DVec2::splat(25.0)];
    let shape = CellSelectionShape::Brush { radius_m: 10.0 };
    let selection = store.select(shape, &path);
    assert_eq!(selection.cells.len(), 5);
    let revision = store.revision();
    assert_eq!(store.profile(CellKey { grid, x: 2, y: 2 }), Some(0));
    assert_eq!(store.revision(), revision); // Preview and cancellation do not write.
    let paint = store.paint(&selection, 3).unwrap();
    assert_eq!(paint.previous.len(), 5);
    let erase_selection = store.select(shape, &path);
    assert_eq!(selection.cells, erase_selection.cells);
    let erase = store.paint(&erase_selection, 0).unwrap();
    for &key in &selection.cells {
        assert_eq!(store.profile(key), Some(0));
    }
    assert!(store.undo_paint(&erase));
    for &key in &selection.cells {
        assert_eq!(store.profile(key), Some(3));
    }
    assert!(!store.undo_paint(&paint)); // An obsolete inverse cannot overwrite newer work.
}

#[test]
fn stale_or_partially_invalid_selection_does_not_write_any_cells() {
    let (mut store, grid) = rectangular_cells(4, 2);
    let selection = store.select(CellSelectionShape::Fill, &[DVec2::splat(5.0)]);
    store.remove_unreserved(CellKey { grid, x: 3, y: 0 });
    assert!(store.paint(&selection, 4).is_none());
    let mut invalid = selection.clone();
    invalid.revision = store.revision();
    assert!(store.paint(&invalid, 4).is_none());
    assert_eq!(store.profile(CellKey { grid, x: 0, y: 0 }), Some(0));
}

#[test]
fn unpainted_cells_do_not_reserve_land_but_paint_does() {
    let (mut store, grid) = rectangular_cells(2, 2);
    let frame = store.frame(grid).unwrap();
    let polygon = frame.corners(0, 0);
    assert!(!store.overlaps_reserved(&polygon));
    let selection = store.select(CellSelectionShape::Cell, &[DVec2::splat(5.0)]);
    store.paint(&selection, 1).unwrap();
    assert!(store.overlaps_reserved(&polygon));
    assert!(!store.overlaps_reserved(&frame.corners(1, 0)));
    assert!(!store.remove_unreserved(CellKey { grid, x: 0, y: 0 }));
    let erase = store.select(CellSelectionShape::Cell, &[DVec2::splat(5.0)]);
    store.paint(&erase, 0).unwrap();
    assert!(!store.overlaps_reserved(&polygon));
}

#[test]
fn invalid_geometry_and_nonfinite_gestures_are_rejected() {
    assert!(GridFrame::new(DVec2::ZERO, DVec2::ZERO, 10.0).is_none());
    assert!(GridFrame::new(DVec2::ZERO, DVec2::splat(1e308), 10.0).is_none());
    assert!(GridFrame::new(DVec2::splat(f64::NAN), DVec2::X, 10.0).is_none());
    assert!(GridFrame::new(DVec2::ZERO, DVec2::X, 0.0).is_none());
    let (store, _) = rectangular_cells(1, 1);
    for shape in [
        CellSelectionShape::Cell,
        CellSelectionShape::Fill,
        CellSelectionShape::Marquee,
        CellSelectionShape::Brush { radius_m: 2.0 },
    ] {
        assert!(
            store
                .select(shape, &[DVec2::splat(f64::NAN)])
                .cells
                .is_empty()
        );
    }
    assert!(
        store
            .select(
                CellSelectionShape::Brush {
                    radius_m: f64::INFINITY
                },
                &[DVec2::splat(5.0)]
            )
            .cells
            .is_empty()
    );
}
