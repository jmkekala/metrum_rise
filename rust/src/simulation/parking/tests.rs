// SPDX-License-Identifier: GPL-2.0-only

//! Local store, query, towing and publication regressions.

use super::*;
fn bay(x: f32, z: f32) -> Bay {
    Bay {
        identity: [0, 0],
        position: [x, 0.0, z],
        path: [[x, z]; 4],
        heading: 0.0,
        width: 2.5,
        length: 6.0,
    }
}
fn source(building: usize) -> PlaceSource {
    PlaceSource::Building {
        building: building as u64,
        anchor: 0,
    }
}

#[test]
fn query_ranks_walk_then_drive_then_handle_with_400m_bound() {
    let mut s = ParkingSystem::default();
    let a = s.add_place(source(0), bay(-1.0, 0.0));
    let b = s.add_place(source(0), bay(1.0, 0.0));
    s.add_place(source(1), bay(400.01, 0.0));
    assert_eq!(s.nearest([0.0, 0.0]), Some(a));
    assert_eq!(
        s.nearest_with([0.0, 0.0], |h, _| Some(ParkingCost {
            walk_m: 1.0,
            walk_s: 1.0,
            drive_s: if h == b { 0.0 } else { 1.0 }
        })),
        Some(b)
    );
    assert!(
        s.nearest_with([0.0, 0.0], |_, _| Some(ParkingCost {
            walk_m: 400.1,
            walk_s: 1.0,
            drive_s: 0.0
        }))
        .is_none()
    );
    let c = s.add_place(source(2), bay(0.0, 400.0));
    assert_eq!(s.nearest([0.0, 800.0]), Some(c));
    assert!(s.nearest([0.0, 800.01]).is_none());
}

#[test]
fn towing_undo_and_reservations_conserve_cars() {
    let mut s = ParkingSystem::default();
    let a = s.add_place(source(0), bay(0.0, 0.0));
    s.fill_test_fleet();
    let car = s.vehicle_handles.handle(0);
    let b = s.add_place(source(1), bay(12.0, 0.0));
    s.remove_place(a);
    assert_eq!(s.vehicles.place[0], b);
    s.remove_place(b);
    assert!(s.vehicles.place[0].is_none());
    s.replace_source(source(0), &[bay(0.0, 0.0)]);
    assert!(
        s.vehicles.place[0].is_none(),
        "restored supply must wait for a driven arrival"
    );
    assert_eq!(s.vehicles.len(), 1);
    let free = s.add_place(source(2), bay(20.0, 0.0));
    assert!(s.reserve(car, free));
    assert!(!s.reserve(car, free));
    s.remove_place(free);
    assert!(s.vehicles.reserved[0].is_none());
    assert!(!s.occupy(car, s.vehicles.place[0]));
    assert_eq!(s.place_handles.resolve(a), None);
}

#[test]
fn save_round_trip_and_chunk_acknowledgements() {
    let mut s = ParkingSystem::default();
    s.add_place(source(0), bay(-512.0, 4.0));
    s.fill_test_fleet();
    s.build_dirty_chunks();
    let key = (-1, 0);
    let first = s.pending[&key].clone();
    s.build_dirty_chunks();
    assert!(Arc::ptr_eq(&first.chunk, &s.pending[&key].chunk));
    let json = s.save_json(|s| s).unwrap();
    let mut loaded = ParkingSystem::load_json(&json).unwrap();
    loaded.build_dirty_chunks();
    assert_eq!(loaded.save_json(|s| s).unwrap(), json);
    assert_eq!(loaded.pending[&key].chunk.buckets, first.chunk.buckets);
    let pos = s.release(s.vehicle_handles.handle(0)).unwrap();
    s.rehouse_near(pos);
    s.build_dirty_chunks();
    // An answer for the older publication keeps the newer chunk; a repeat changes nothing.
    let published = s.publication();
    assert!(published.offer(false));
    assert_eq!(published.unseen().count(), 1);
    published.acknowledge(first.serial);
    s.take_renderer_answers();
    assert!(!s.pending.is_empty());
    published.acknowledge(published.serial);
    published.acknowledge(first.serial);
    assert!(!published.offer(false));
    assert_eq!(published.unseen().count(), 0);
    s.take_renderer_answers();
    assert!(s.pending.is_empty());
    assert!(s.publication().updates.is_empty());
    s.build_dirty_chunks();
    assert!(s.pending.is_empty());
    let broken = json.replace("\"occupant\":0", "\"occupant\":4294967296");
    assert!(ParkingSystem::load_json(&broken).is_err());
}

#[test]
fn a_full_request_lists_every_chunk_again_in_a_later_snapshot() {
    let mut s = ParkingSystem::default();
    s.add_place(source(0), bay(-512.0, 4.0));
    s.add_place(source(1), bay(512.0, 4.0));
    s.fill_test_fleet();
    s.build_dirty_chunks();
    let listed = s.publication();
    listed.acknowledge(listed.serial);
    s.take_renderer_answers();
    assert!(s.publication().updates.is_empty());
    assert!(!s.publication().offer(true), "nothing new, the request is for the next snapshot");
    s.take_renderer_answers();
    let full = s.publication();
    assert!(full.holds_full());
    assert_eq!(full.unseen().count(), 2);
    assert!(full.offer(true));
    s.take_renderer_answers();
    assert_eq!(s.publication().full_serial, full.full_serial, "served once");
}

// A rebuild is a copy: only a place with no pose for its car's model reaches the ground solve.
#[test]
fn a_chunk_rebuild_solves_no_unchanged_place() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let solves = AtomicUsize::new(0);
    let solve = |model: u8, position: [f32; 3], _: f32| {
        solves.fetch_add(1, Ordering::Relaxed);
        [f32::from(model), position[0], position[2], 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
    };
    let count = |s: &mut ParkingSystem| {
        solves.store(0, Ordering::Relaxed);
        s.build_dirty_chunks_with(solve);
        solves.load(Ordering::Relaxed)
    };
    let mut s = ParkingSystem::default();
    let places: Vec<_> = (0..8).map(|i| s.add_place(source(i), bay(i as f32 * 8.0, 4.0))).collect();
    s.fill_test_fleet();
    assert_eq!(count(&mut s), 8, "each parked car solves its new place once");
    assert_eq!(count(&mut s), 0, "nothing dirty");
    let free = s.add_place(source(8), bay(100.0, 4.0));
    let spare = s.add_place(source(9), bay(108.0, 4.0));
    assert_eq!(count(&mut s), 0, "an empty new bay solves nothing");
    // A departure dirties the chunk: seven cars stay, and none is solved again.
    let leaver = s.places.occupant[s.place_handles.resolve(places[0]).unwrap()];
    s.set_moving(leaver, true);
    assert_eq!(count(&mut s), 0);
    s.set_moving(leaver, false);
    assert_eq!(count(&mut s), 0);
    // The same model comes back to a bay it stood on; a new bay takes the one solve.
    s.release(leaver);
    assert!(s.occupy(leaver, free));
    assert_eq!(count(&mut s), 1, "only the arrival's new place");
    s.release(leaver);
    assert!(s.occupy(leaver, places[0]));
    assert_eq!(count(&mut s), 0, "its pose is still there");
    // Another model on the same bay stands differently: one solve, for that place alone.
    let other = s.places.occupant[s.place_handles.resolve(places[1]).unwrap()];
    assert_ne!(
        s.vehicles.model[s.vehicle_handles.resolve(leaver).unwrap()],
        s.vehicles.model[s.vehicle_handles.resolve(other).unwrap()]
    );
    s.release(leaver);
    assert!(s.occupy(leaver, spare));
    s.release(other);
    assert!(s.occupy(other, places[0]));
    assert_eq!(count(&mut s), 2, "the two arrivals, nothing else in the chunk");
    assert_eq!(s.pending.len(), 1);
}

// A moved ground forgets the poses inside it, and only those; the chunk republishes.
#[test]
fn invalidated_ground_solves_its_places_again_and_no_others() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let solves = AtomicUsize::new(0);
    let mut s = ParkingSystem::default();
    for i in 0..4 {
        s.add_place(source(i), bay(i as f32 * 100.0, 4.0));
    }
    s.fill_test_fleet();
    let solve = |_: u8, _: [f32; 3], _: f32| {
        solves.fetch_add(1, Ordering::Relaxed);
        [0.0; 12]
    };
    s.build_dirty_chunks_with(solve);
    assert_eq!(solves.swap(0, Ordering::Relaxed), 4);
    s.invalidate_ground([90.0, 0.0, 210.0, 8.0]);
    assert!(s.has_dirty_chunks(), "a parked car stands there");
    s.build_dirty_chunks_with(solve);
    assert_eq!(solves.swap(0, Ordering::Relaxed), 2);
    s.invalidate_ground([1000.0, 0.0, 1100.0, 8.0]);
    assert!(!s.has_dirty_chunks(), "no bay there");
    s.invalidate_all_ground();
    s.build_dirty_chunks_with(solve);
    assert_eq!(solves.swap(0, Ordering::Relaxed), 4);
}

#[test]
fn a_destination_reservation_survives_vacating_the_origin() {
    let mut s = ParkingSystem::default();
    s.add_place(source(0), bay(0.0, 0.0));
    s.fill_test_fleet();
    let car = s.vehicle_handles.handle(0);
    let destination = s.add_place(source(1), bay(12.0, 0.0));
    assert!(s.reserve(car, destination));
    s.release(car);
    assert_eq!(s.vehicles.reserved[0], destination);
    assert!(s.occupy(car, destination));
    assert!(s.vehicles.reserved[0].is_none());
    ParkingSystem::load_json(&s.save_json(|s| s).unwrap()).unwrap();
}

#[test]
fn edit_towing_is_identical_after_loading_rebuilt_source_lists() {
    let mut s = ParkingSystem::default();
    let mut home = Vec::new();
    for x in [0.0, 6.0, 12.0, 18.0] {
        home.push(s.add_place(source(0), bay(x, 0.0)));
    }
    for x in [24.0, 30.0] {
        s.add_place(source(1), bay(x, 0.0));
    }
    s.fill_test_fleet();
    s.remove_place(home[1]);
    for x in [40.0, 46.0, 52.0] {
        s.add_place(source(2), bay(x, 0.0));
    }
    s.rehouse_near([0.0, 0.0]);
    let json = s.save_json(|s| s).unwrap();
    let mut loaded = ParkingSystem::load_json(&json).unwrap();
    // Dense rows survive the save, but the derived per-source swap-remove lists need not.
    assert_ne!(s.sources[&source(0)], loaded.sources[&source(0)]);
    s.replace_source(source(0), &[]);
    loaded.replace_source(source(0), &[]);
    assert_eq!(
        s.save_json(|s| s).unwrap(),
        loaded.save_json(|s| s).unwrap()
    );
}

#[test]
fn stored_allocation_order_is_stable_across_insertion_order_and_save() {
    use crate::simulation::zoning::profiles::ZoneDensity;
    let make = |reverse: bool| {
        let mut s = ParkingSystem::default();
        let cars: Vec<_> = (0..24)
            .map(|owner| {
                s.assign_household(
                    owner,
                    ZoneDensity::Low,
                    [if owner % 2 == 0 { -10.0 } else { 10.0 }, 0.0],
                )
            })
            .collect();
        for i in 0..cars.len() {
            let car = cars[if reverse { cars.len() - 1 - i } else { i }];
            s.park_new_car(car, 0);
        }
        s.rehouse_near([0.0, 0.0]);
        s
    };
    let mut a = make(false);
    let mut b = make(true);
    let key = (0, 0);
    for _ in 0..8 {
        assert_eq!(a.next_stored_car(key), b.next_stored_car(key));
    }
    let mut loaded = ParkingSystem::load_json(&a.save_json(|s| s).unwrap()).unwrap();
    let mut count = 8;
    while let Some(car) = a.next_stored_car(key) {
        assert_eq!(Some(car), b.next_stored_car(key));
        assert_eq!(Some(car), loaded.next_stored_car(key));
        count += 1;
    }
    assert_eq!(count, 24);
    assert!(loaded.next_stored_car(key).is_none());
    assert!(a.vehicles.place.iter().all(|p| p.is_none()));
}
