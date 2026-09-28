// SPDX-License-Identifier: GPL-2.0-only

//! Cell lot coverage, atomic claims and shared parcel occupancy/redevelopment regressions.

use super::*;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::{ParcelId, ParcelPlacementError, ZoningSystem};

fn fixture() -> (ZoningSystem, CellLot, ParcelId) {
    let (store, grid) = rectangular_cells(8, 6);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    zoning.cells = store;
    let paint = zoning
        .cells
        .select(CellSelectionShape::Fill, &[DVec2::splat(5.0)]);
    zoning.paint_cells(&paint, 1, |_| false).unwrap();
    let lot = CellLot::new(CellKey { grid, x: 0, y: 0 }, 2, 3, CellFrontage::MinY).unwrap();
    let id = zoning
        .install_prevalidated_cell_lot(lot, 0, -1, 0.2, 1)
        .unwrap();
    (zoning, lot, id)
}

#[test]
fn coverage_geometry_respects_all_frontages_and_rotated_frames() {
    for direction in [DVec2::X, DVec2::new(0.8, 0.6), DVec2::new(0.2, 1.0)] {
        let frame = GridFrame::new(DVec2::splat(5.0), direction, 10.0).unwrap();
        for frontage in [
            CellFrontage::MinY,
            CellFrontage::MaxX,
            CellFrontage::MaxY,
            CellFrontage::MinX,
        ] {
            let lot = CellLot::new(
                CellKey {
                    grid: 1,
                    x: -14,
                    y: -9,
                },
                2,
                3,
                frontage,
            )
            .unwrap();
            let geometry = lot.geometry(frame, 123, -1, 0.73);
            assert_eq!(lot.cells().count(), 6);
            assert_eq!(geometry.frontage_m, 20.0);
            assert_eq!(geometry.depth_m, 30.0);
            assert!(
                (geometry.corners[1] - geometry.corners[0] - geometry.tangent * 20.0).length()
                    < 0.0001
            );
            assert!(
                (geometry.corners[2] - geometry.corners[1] - geometry.normal * 30.0).length()
                    < 0.0001
            );
            let corners: Vec<_> = lot
                .cells()
                .flat_map(|cell| frame.corners(cell.x, cell.y))
                .collect();
            for corner in geometry.corners {
                assert!(
                    corners
                        .iter()
                        .any(|p| corner.x == p.x as f32 && corner.y == p.y as f32)
                );
            }
            assert_eq!(geometry.corners, lot.geometry(frame, 456, 1, 0.12).corners);
        }
    }
}

#[test]
fn invalid_or_partial_claims_do_not_write_any_cells() {
    let (mut store, grid) = rectangular_cells(4, 6);
    let origin = CellKey { grid, x: 0, y: 0 };
    assert!(CellLot::new(origin, 0, 3, CellFrontage::MinY).is_none());
    assert!(CellLot::new(origin, 2, 7, CellFrontage::MinY).is_none());
    assert!(
        CellLot::new(
            CellKey {
                x: i32::MAX,
                ..origin
            },
            2,
            3,
            CellFrontage::MinY
        )
        .is_none()
    );
    let lot = CellLot::new(origin, 2, 3, CellFrontage::MinY).unwrap();
    let revision = store.revision();
    assert!(!store.claim_lot(lot, 1, 0));
    assert!(!store.claim_lot(lot, 1, 1));
    assert_eq!(store.revision(), revision);
    let selection = store.select(CellSelectionShape::Fill, &[DVec2::splat(5.0)]);
    store.paint(&selection, 1).unwrap();
    assert!(store.claim_lot(lot, 1, 1));
    let conflict = CellLot::new(CellKey { x: 1, ..origin }, 2, 3, CellFrontage::MinY).unwrap();
    let revision = store.revision();
    assert!(!store.claim_lot(conflict, 2, 1));
    assert!(!store.release_lot(lot, 2));
    assert_eq!(store.revision(), revision);
    assert_eq!(store.lot(CellKey { x: 2, ..origin }), Some(0));
    assert!(store.release_lot(lot, 1));
    assert!(
        lot.cells()
            .all(|key| store.lot(key) == Some(0) && store.profile(key) == Some(1))
    );
}

#[test]
fn partial_erase_releases_empty_lot_and_undo_restores_identity_and_generation() {
    let (mut zoning, lot, id) = fixture();
    zoning.restore_parcel_build_generation(id.raw(), 17);
    let other = CellLot::new(
        CellKey {
            x: 4,
            ..lot.origin()
        },
        2,
        3,
        CellFrontage::MinY,
    )
    .unwrap();
    let other_id = zoning
        .install_prevalidated_cell_lot(other, 0, -1, 0.6, 1)
        .unwrap();
    let selection = zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::splat(5.0)]);
    let edit = zoning.paint_cells(&selection, 0, |_| false).unwrap();
    assert_eq!(edit.changed_lots().collect::<Vec<_>>(), [id]);
    assert!(zoning.parcels.get(id).is_none());
    assert!(lot.cells().all(|key| zoning.cells.lot(key) == Some(0)));
    assert_eq!(
        zoning.cells.profile(CellKey {
            y: 1,
            ..lot.origin()
        }),
        Some(1)
    );
    // Removing a dense slot must repair the moved id lookup and both spatial memberships.
    assert_eq!(
        zoning
            .parcel_at(godot::prelude::Vector2::new(45.0, 5.0))
            .unwrap()
            .id(),
        other_id
    );
    assert!(
        zoning
            .parcel_at(godot::prelude::Vector2::new(5.0, 5.0))
            .is_none()
    );
    assert!(zoning.undo_cell_paint(&edit));
    assert_eq!(zoning.parcels.get(id).unwrap().build_generation(), 17);
    assert_eq!(
        zoning
            .parcel_at(godot::prelude::Vector2::new(5.0, 5.0))
            .unwrap()
            .id(),
        id
    );
    assert!(
        lot.cells()
            .all(|key| zoning.cells.lot(key) == Some(id.raw())
                && zoning.cells.profile(key) == Some(1))
    );
    assert!(!zoning.undo_cell_paint(&edit));
}

#[test]
fn occupied_partial_erase_retains_claim_until_lifecycle_releases_building() {
    let (mut zoning, lot, id) = fixture();
    assert!(zoning.occupy_parcel(id.raw(), 7));
    let selection = zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::splat(5.0)]);
    let edit = zoning.paint_cells(&selection, 0, |_| false).unwrap();
    let parcel = zoning.parcels.get(id).unwrap();
    assert_eq!(parcel.occupied_building(), Some(7));
    assert_eq!(parcel.zone_profile_runtime_id(), 0);
    assert!(
        lot.cells()
            .all(|key| zoning.cells.lot(key) == Some(id.raw()))
    );
    assert!(zoning.undo_cell_paint(&edit));
    assert_eq!(zoning.parcels.get(id).unwrap().zone_profile_runtime_id(), 1);
    let selection = zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::splat(5.0)]);
    let edit = zoning.paint_cells(&selection, 0, |_| false).unwrap();
    assert!(zoning.clear_parcel_occupancy(id.raw()));
    assert!(zoning.parcels.get(id).is_none());
    assert!(lot.cells().all(|key| zoning.cells.lot(key) == Some(0)));
    assert!(!zoning.undo_cell_paint(&edit));
}

#[test]
fn compatible_repaint_and_demolition_preserve_lot_and_redevelopment_generation() {
    let (mut zoning, lot, id) = fixture();
    assert!(zoning.occupy_parcel(id.raw(), 3));
    for profile in [0, 1] {
        let selection = zoning
            .cells
            .select(CellSelectionShape::Cell, &[DVec2::splat(5.0)]);
        zoning.paint_cells(&selection, profile, |_| false).unwrap();
    }
    assert_eq!(zoning.parcels.get(id).unwrap().zone_profile_runtime_id(), 1);
    assert!(zoning.clear_parcel_occupancy(id.raw()));
    let parcel = zoning.parcels.get(id).unwrap();
    assert!(parcel.is_available());
    assert_eq!(parcel.build_generation(), 1);
    assert!(
        lot.cells()
            .all(|key| zoning.cells.lot(key) == Some(id.raw()))
    );
    assert!(zoning.occupy_parcel(id.raw(), 8));
}

#[test]
fn historical_cell_gesture_undo_revalidates_all_cells_before_any_restore() {
    let (mut zoning, _, _) = fixture();
    let selection = zoning.cells.select(
        CellSelectionShape::Brush { radius_m: 15.0 },
        &[DVec2::splat(15.0)],
    );
    let edit = zoning.paint_cells(&selection, 0, |_| false).unwrap();
    let before = zoning.cells.saved_cells();
    let revision = zoning.cells.revision();
    assert!(
        zoning
            .restore_cell_gesture(&edit, |corners| corners[0].x >= 10.0)
            .is_none()
    );
    assert_eq!(zoning.cells.saved_cells(), before);
    assert_eq!(zoning.cells.revision(), revision);
    assert!(zoning.restore_cell_gesture(&edit, |_| false).is_some());
    assert!(
        edit.paint
            .previous
            .iter()
            .all(|&(key, profile)| zoning.cells.profile(key) == Some(profile))
    );
}

#[test]
fn parcel_tools_cannot_rezone_cell_lots_independently_of_their_paint() {
    let (mut zoning, lot, id) = fixture();
    assert_eq!(
        zoning.place_or_rezone_parcel_at(5.0, 5.0, 0, 20.0, 20.0, &RegionGraph::new()),
        Err(ParcelPlacementError::OverlapsExistingParcel)
    );
    assert!(zoning.rezone_stroke(5.0, 5.0, 15.0, 15.0, 0).is_err());
    assert!(
        zoning
            .preview_rezone_stroke(5.0, 5.0, 15.0, 15.0)
            .is_empty()
    );
    assert!(zoning.parcel_geometry_at(5.0, 5.0).is_none());
    assert!(!zoning.has_parcel_at(5.0, 5.0));
    assert_eq!(zoning.parcels.get(id).unwrap().zone_profile_runtime_id(), 1);
    assert!(lot.cells().all(|key| zoning.cells.profile(key) == Some(1)));
}

#[test]
fn road_parcel_removal_and_undo_release_and_restore_cell_claims() {
    let (mut zoning, lot, id) = fixture();
    let undo = zoning.capture_parcel_removal_undo(0);
    assert_eq!(zoning.remove_parcels_attached_to_edge(0), 1);
    assert!(lot.cells().all(|key| zoning.cells.lot(key) == Some(0)));
    assert!(zoning.can_restore_parcel_removal_undo(&undo));
    zoning.restore_parcel_removal_undo(undo);
    assert!(
        lot.cells()
            .all(|key| zoning.cells.lot(key) == Some(id.raw()))
    );
    assert_eq!(
        zoning.remove_parcels_by_raw_ids(&std::collections::HashSet::from([id.raw()])),
        1
    );
    assert!(lot.cells().all(|key| zoning.cells.lot(key) == Some(0)));
}

#[test]
fn stale_undo_after_building_claim_is_atomic() {
    let (mut zoning, lot, id) = fixture();
    let key = CellKey {
        x: 7,
        y: 5,
        ..lot.origin()
    };
    let selection = CellSelection {
        revision: zoning.cells.revision(),
        cells: vec![key],
    };
    let edit = zoning.paint_cells(&selection, 0, |_| false).unwrap();
    assert!(zoning.occupy_parcel(id.raw(), 3));
    let revision = zoning.cells.revision();
    assert!(!zoning.undo_cell_paint(&edit));
    assert_eq!(zoning.cells.revision(), revision);
    assert_eq!(zoning.cells.profile(key), Some(0));
}

fn add_remote_lots(zoning: &mut ZoningSystem, grid: u64, count: usize) {
    for index in 0..count {
        let lot = CellLot::new(
            CellKey {
                grid,
                x: 200 + (index % 128) as i32 * 6,
                y: 200 + (index / 128) as i32 * 6,
            },
            2,
            3,
            CellFrontage::MinY,
        )
        .unwrap();
        for key in lot.cells() {
            assert!(zoning.cells.insert(key));
            zoning.cells.set_profile(key, 1);
        }
        zoning
            .install_prevalidated_cell_lot(lot, 0, -1, 0.5, 1)
            .unwrap();
    }
}

fn erase_and_undo(zoning: &mut ZoningSystem) {
    let selection = zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::splat(5.0)]);
    let edit = zoning.paint_cells(&selection, 0, |_| false).unwrap();
    assert_eq!(edit.paint.previous.len(), 1);
    assert_eq!(edit.changed_lots().count(), 1);
    assert!(zoning.undo_cell_paint(&edit));
}

#[test]
fn remote_lots_do_not_change_local_paint_or_undo_products() {
    for count in [0, 1_024] {
        let (mut zoning, lot, id) = fixture();
        add_remote_lots(&mut zoning, lot.origin().grid, count);
        let geometry = zoning.parcels.get(id).unwrap().corners();
        erase_and_undo(&mut zoning);
        assert_eq!(zoning.parcels().len(), count + 1);
        assert_eq!(zoning.parcels.get(id).unwrap().corners(), geometry);
        assert!(
            lot.cells()
                .all(|key| zoning.cells.lot(key) == Some(id.raw())
                    && zoning.cells.profile(key) == Some(1))
        );
    }
}

#[test]
#[ignore = "manual matched release timing of local cell paint, lot release and inverse"]
fn benchmark_cell_lot_edit_locality() {
    use std::hint::black_box;
    use std::time::Instant;
    for count in [0, 1_024, 10_000] {
        let (mut zoning, lot, id) = fixture();
        add_remote_lots(&mut zoning, lot.origin().grid, count);
        for _ in 0..64 {
            erase_and_undo(&mut zoning);
        }
        let mut samples = [0.0_f64; 31];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..512 {
                erase_and_undo(black_box(&mut zoning));
            }
            *sample = start.elapsed().as_secs_f64() * 1e6 / 512.0;
        }
        samples.sort_unstable_by(f64::total_cmp);
        assert_eq!(zoning.parcels().len(), count + 1);
        assert!(
            lot.cells()
                .all(|key| zoning.cells.lot(key) == Some(id.raw())
                    && zoning.cells.profile(key) == Some(1))
        );
        println!(
            "remote_lots={count} remote_cells={} erase_undo_us={:.3}",
            count * 6,
            samples[15]
        );
    }
}

#[test]
#[ignore = "release occupancy-clear locality; background setup and dirty-region drain excluded"]
fn benchmark_cell_occupancy_release_locality() {
    use std::hint::black_box;
    use std::time::Instant;

    for count in [0, 1_024, 10_000] {
        let (mut zoning, lot, id) = fixture();
        add_remote_lots(&mut zoning, lot.origin().grid, count);
        zoning.take_dirty_cell_lot_regions();
        let run = |zoning: &mut ZoningSystem| {
            assert!(zoning.occupy_parcel(id.raw(), 0));
            assert!(zoning.clear_parcel_occupancy(id.raw()));
        };
        for _ in 0..64 {
            run(&mut zoning);
        }
        let mut samples = [0.0_f64; 31];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..512 {
                run(black_box(&mut zoning));
            }
            *sample = start.elapsed().as_secs_f64() * 1e6 / 512.0;
        }
        samples.sort_unstable_by(f64::total_cmp);
        assert_eq!(zoning.parcels().len(), count + 1);
        assert!(zoning.parcels.get(id).unwrap().is_available());
        assert!(
            lot.cells()
                .all(|key| zoning.cells.lot(key) == Some(id.raw()))
        );
        let regions = zoning.take_dirty_cell_lot_regions();
        assert_eq!(regions.len(), 1, "only the released footprint is queued");
        assert_eq!(regions[0].min, DVec2::ZERO);
        println!(
            "remote_lots={count} remote_cells={} occupy_clear_us={:.3} dirty_regions={}",
            count * 6,
            samples[15],
            regions.len(),
        );
    }
}
