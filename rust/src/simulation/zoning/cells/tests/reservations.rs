// SPDX-License-Identifier: GPL-2.0-only

//! Exact external reservations, concave field notches and erased occupied coverage.

use super::*;
use crate::simulation::agriculture::PolygonFootprint;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector3;

#[test]
fn concave_reservations_keep_notches_and_shared_boundaries_free() {
    let (mut store, grid) = rectangular_cells(4, 4);
    let key = CellKey { grid, x: 1, y: 1 };
    store.set_profile(key, 1);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    zoning.cells = store;
    let notch = PolygonFootprint::from_precise_points(
        [
            [0.0, 0.0],
            [30.0, 0.0],
            [30.0, 10.0],
            [10.0, 10.0],
            [10.0, 30.0],
            [0.0, 30.0],
        ]
        .into_iter(),
    );
    assert!(!zoning.overlaps_reservation(&notch));
    let overlap = PolygonFootprint::from_precise_points(
        [
            [0.0, 0.0],
            [30.0, 0.0],
            [30.0, 10.001],
            [10.0, 10.001],
            [10.0, 30.0],
            [0.0, 30.0],
        ]
        .into_iter(),
    );
    assert!(zoning.overlaps_reservation(&overlap));
    let lot = CellLot::new(key, 1, 1, CellFrontage::MinY).unwrap();
    assert!(zoning.cells.claim_lot(lot, 7, 1));
    zoning.cells.set_profile(key, 0);
    assert!(!zoning.cells.has_paint());
    assert!(zoning.cells.has_reservations());
    assert!(zoning.overlaps_reservation(&overlap));
    assert!(zoning.cells.release_lot(lot, 7));
    assert!(!zoning.cells.has_reservations());
    assert!(!zoning.overlaps_reservation(&overlap));
}

#[test]
fn road_contact_uses_precision_bound_without_allowing_millimetre_encroachment() {
    let (mut store, grid) = rectangular_cells(4, 2);
    store.set_profile(CellKey { grid, x: 1, y: 0 }, 1);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    zoning.cells = store;
    for (z, overlaps) in [(-5.0, false), (-4.999, true)] {
        assert_eq!(
            zoning.cells_overlap_road_corridor(
                &[Vector3::new(0.0, 0.0, z), Vector3::new(30.0, 0.0, z),],
                5.0
            ),
            overlaps
        );
    }
}

#[test]
fn borrowed_road_checks_preserve_precise_concavity_contact_and_large_coordinates() {
    use crate::simulation::network::surface::RoadSurfaceVisualPolygon;
    use glam::DVec3;

    for origin in [DVec2::splat(-512.0), DVec2::new(10000.25, -10000.75)] {
        for angle in [0.0_f64, 0.35, -0.55] {
            let frame = GridFrame::new(origin, DVec2::new(angle.cos(), angle.sin()), 10.0).unwrap();
            let mut zoning = ZoningSystem::new(&WorldConfig::default());
            let grid = zoning.cells.register_frame(frame);
            let base = frame.local(origin).floor();
            let key = CellKey {
                grid,
                x: base.x as i32 + 1,
                y: base.y as i32 + 1,
            };
            assert!(zoning.cells.insert(key));
            zoning.cells.set_profile(key, 1);
            let reserved = PolygonFootprint::from_precise_points(
                road_contact_interior(&frame.corners(key.x, key.y))
                    .into_iter()
                    .map(|p| [p.x, p.y]),
            );
            for (extent, expected) in [(1.0, false), (1.001, true), (2.0, true)] {
                // A concave L leaves the cell in its notch, then intrudes across its front.
                let points = [
                    [0.0, 0.0],
                    [3.0, 0.0],
                    [3.0, extent],
                    [1.0, extent],
                    [1.0, 3.0],
                    [0.0, 3.0],
                ]
                .map(|[x, y]| frame.world(base.x + x, base.y + y));
                let owned =
                    PolygonFootprint::from_precise_points(points.into_iter().map(|p| [p.x, p.y]));
                assert_eq!(owned.overlaps(&reserved), expected);
                let polygon = RoadSurfaceVisualPolygon {
                    points_world: points
                        .into_iter()
                        .map(|p| DVec3::new(p.x, 0.0, p.y))
                        .collect(),
                    triangles_world: Vec::new(),
                };
                assert_eq!(zoning.cells_overlap_road_polygon(&polygon), expected);
            }
            let empty = RoadSurfaceVisualPolygon {
                points_world: Vec::new(),
                triangles_world: Vec::new(),
            };
            assert!(!zoning.cells_overlap_road_polygon(&empty));
            // Global reservations must not cause a distant polygon to enter the exact path.
            let distant = RoadSurfaceVisualPolygon {
                points_world: frame
                    .corners(key.x + 1000, key.y + 1000)
                    .into_iter()
                    .map(|p| DVec3::new(p.x, 0.0, p.y))
                    .collect(),
                triangles_world: Vec::new(),
            };
            assert!(!zoning.cells_overlap_road_polygon(&distant));
        }
    }
}

#[test]
fn reserved_footprint_visits_match_direct_geometry_across_blocks_and_chunks() {
    for angle in [0.0_f64, 0.35, -0.55] {
        let frame = GridFrame::new(
            DVec2::new(3.2, 6.7),
            DVec2::new(angle.cos(), angle.sin()),
            10.0,
        )
        .unwrap();
        let mut store = CellStore::default();
        let grid = store.register_frame(frame);
        let mut reserved = Vec::new();
        for base in [-53, 49, 1000] {
            for x in base..base + 12 {
                for y in base..base + 12 {
                    let key = CellKey { grid, x, y };
                    assert!(store.insert(key));
                    if (x + y) % 3 == 0 {
                        store.set_profile(key, 1);
                        reserved.push(key);
                    }
                }
            }
        }
        for key in &reserved {
            let corners = frame.corners(key.x, key.y);
            for bounds in [
                CellBounds::from_points(corners),
                CellBounds {
                    min: corners[0],
                    max: corners[0],
                },
                CellBounds::from_points(corners).expanded(10.0),
            ] {
                let mut expected: Vec<_> = reserved
                    .iter()
                    .copied()
                    .filter_map(|key| {
                        let corners = frame.corners(key.x, key.y);
                        CellBounds::from_points(corners)
                            .intersects(bounds)
                            .then_some((key, corners))
                    })
                    .collect();
                let mut actual = Vec::new();
                store.visit_reserved_footprints_in_bounds(bounds, |key, corners| {
                    actual.push((key, corners))
                });
                expected.sort_by_key(|(key, _)| *key);
                actual.sort_by_key(|(key, _)| *key);
                assert_eq!(actual, expected);
            }
        }
    }
}
