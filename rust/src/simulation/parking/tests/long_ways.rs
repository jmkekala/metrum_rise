// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-25` long ways at the parking system: storage beside the row, the shared pool's reuse
//! under edits, swap removal and retirement, the stored length, and the saved row. Nothing
//! produces a long way before `PARK-26`, so each test gives one to a building's place through
//! `replace_building`, the lot reconcile's own entry.

use super::*;
use crate::simulation::economy::agents::AgentHandle;

// A way of `n` points from a road connection at (x, 20) up to a stop at (x, 0), with a sideways
// step of 1 m every other point: `n - 2` bends.
fn way(x: f32, n: usize) -> Vec<[f32; 2]> {
    (0..n)
        .map(|i| {
            let z = 20.0 - 20.0 * i as f32 / (n - 1) as f32;
            let side = if i % 2 == 1 && i + 1 < n { 1.0 } else { 0.0 };
            [x + side, z]
        })
        .collect()
}

// The bay at the way's stop, its path the way's two first and two last points.
fn bay_on(points: &[[f32; 2]]) -> Bay {
    let n = points.len();
    let [x, z] = points[n - 1];
    Bay {
        path: [points[0], points[1], points[n - 2], points[n - 1]],
        ..bay(x, z)
    }
}

fn route(points: &[[f32; 2]]) -> DockRoute {
    DockRoute::new(points).unwrap()
}

// Building `b`'s places at x 0, 10, 20 by anchor, anchor 1 on a way of `n` points (none for 4).
fn publish(s: &mut ParkingSystem, b: u64, n: usize) {
    let mut bays = Vec::new();
    let mut ways = Vec::new();
    for anchor in 0..3u32 {
        let points = way(anchor as f32 * 10.0, if anchor == 1 { n } else { 4 });
        bays.push((anchor, bay_on(&points)));
        if anchor == 1 && n > 4 {
            ways.push((anchor, route(&points)));
        }
    }
    s.replace_building(b, &bays, &ways);
}

fn place(s: &ParkingSystem, building: u64, anchor: u32) -> Handle<Place> {
    s.sources[&PlaceSource::Building { building, anchor }][0]
}

// A four-point way is the row's path: the row holds no way, the pool stays empty, republishing
// it journals nothing, and the saved parking is byte for byte that of the same bays published
// without ways.
#[test]
fn a_four_point_way_is_stored_exactly_as_before() {
    let mut plain = ParkingSystem::default();
    let mut routed = ParkingSystem::default();
    let bays: Vec<_> = (0..3u32)
        .map(|a| (a, bay_on(&way(a as f32 * 10.0, 4))))
        .collect();
    plain.replace_building(7, &bays, &[]);
    let ways: Vec<_> = bays.iter().map(|&(a, bay)| (a, route(&bay.path))).collect();
    routed.replace_building(7, &bays, &ways);
    assert!((0..3).all(|a| routed.long_way(place(&routed, 7, a)).is_none()));
    assert_eq!(routed.check_car_ways(), Ok(0));
    // Published again, the same four points journal nothing for the doors.
    routed.place_changes.clear();
    routed.replace_building(7, &bays, &ways);
    assert!(routed.place_changes.is_empty());
    let saved = routed.save_json(|s| s).unwrap();
    assert_eq!(saved, plain.save_json(|s| s).unwrap());
    assert!(!saved.contains("route_in"));
    assert_eq!(routed.place_product_hash(), plain.place_product_hash());
}

// The way is kept whole beside the row, and the time doors sort the place by is its whole
// length at driveway speed, read as stored: the row's four points alone are shorter.
#[test]
fn a_long_way_is_stored_whole_and_timed_by_its_length() {
    let mut s = ParkingSystem::default();
    publish(&mut s, 7, 9);
    let h = place(&s, 7, 1);
    let points = way(10.0, 9);
    assert_eq!(s.long_way(h), Some(points.clone()));
    let p = s.place_handles.resolve(h).unwrap();
    assert_eq!(s.places.path[p], bay_on(&points).path);
    let speed = crate::config::AGENT_DRIVEWAY_SPEED_MS;
    assert_eq!(s.maneuver_seconds(h), ways::length(&points) / speed);
    assert!(s.maneuver_seconds(h) > ways::length(&s.places.path[p]) / speed + 0.1);
    assert_eq!(s.check_car_ways(), Ok(9));
}

// Edits reuse the pool: a changed way of the same length takes its own run back, another length
// takes the run another place freed, and none grows the pool past the ways alive at once. A swap
// removal moves a way with its row; a retired row keeps its way for the car still on it.
#[test]
fn edits_and_removals_free_and_reuse_the_pool() {
    let mut s = ParkingSystem::default();
    publish(&mut s, 7, 9);
    publish(&mut s, 8, 6);
    assert_eq!(s.check_car_ways(), Ok(15));
    // The same way again: nothing changes, nothing is journalled.
    s.place_changes.clear();
    publish(&mut s, 7, 9);
    assert!(s.place_changes.is_empty());
    assert_eq!(s.check_car_ways(), Ok(15));
    // Another way of nine points under the kept place: its run, journalled for the doors.
    let mut moved = way(10.0, 9);
    moved[3][0] += 0.25;
    let bays: Vec<_> = (0..3u32)
        .map(|a| {
            (
                a,
                if a == 1 {
                    bay_on(&moved)
                } else {
                    bay_on(&way(a as f32 * 10.0, 4))
                },
            )
        })
        .collect();
    s.replace_building(7, &bays, &[(1, route(&moved))]);
    assert_eq!(s.long_way(place(&s, 7, 1)), Some(moved));
    assert_eq!(s.place_changes.len(), 1);
    assert_eq!(s.check_car_ways(), Ok(15));
    // Back to four points: the run is freed, and a new six-point way elsewhere takes building
    // 8's after that one goes.
    publish(&mut s, 7, 4);
    assert!(s.long_way(place(&s, 7, 1)).is_none());
    assert_eq!(s.check_car_ways(), Ok(15));
    s.replace_building(8, &[], &[]);
    publish(&mut s, 9, 6);
    publish(&mut s, 7, 9);
    assert_eq!(s.check_car_ways(), Ok(15), "both runs reused");
    // Removing an earlier row swaps a later one into its slot: the way goes with its row.
    let first = place(&s, 7, 0);
    let last_row = s.places.len() - 1;
    let tail = s.place_handles.handle(last_row);
    let tail_way = s.long_way(tail);
    s.remove_place(first);
    assert_eq!(s.long_way(tail), tail_way);
    assert_eq!(s.long_way(place(&s, 7, 1)), Some(way(10.0, 9)));
    // A place moved while a car holds it is retired for that car and keeps its way until the
    // car lets it go.
    let held = place(&s, 9, 1);
    let car = s.assign_household(
        1,
        crate::simulation::zoning::profiles::ZoneDensity::Low,
        [10.0, 0.0],
    );
    assert!(s.occupy(car, place(&s, 7, 2)));
    assert!(s.reserve_trip(car, AgentHandle::from_parts(1, 0), held));
    s.replace_building(9, &[], &[]);
    assert_eq!(s.long_way(held), Some(way(10.0, 6)));
    assert_eq!(s.check_car_ways(), Ok(15));
    s.clear_retired_places(car);
    assert!(s.place_handles.resolve(held).is_none());
    assert_eq!(s.check_car_ways(), Ok(15));
}

// A saved long way loads as it was saved, beside its row, and saves the same bytes again; only
// its row writes a way; and a row whose path is not its way's ends refuses the save.
#[test]
fn a_long_way_round_trips_a_save() {
    let mut s = ParkingSystem::default();
    publish(&mut s, 7, 10);
    let json = s.save_json(|s| s).unwrap();
    assert_eq!(json.matches("route_in").count(), 1);
    let loaded = ParkingSystem::load_json(&json).unwrap();
    assert_eq!(loaded.long_way(place(&loaded, 7, 1)), Some(way(10.0, 10)));
    assert_eq!(loaded.check_car_ways(), Ok(10));
    assert_eq!(loaded.save_json(|s| s).unwrap(), json);
    assert_eq!(loaded.place_product_hash(), s.place_product_hash());
    let stop = format!("[{:?},{:?}]", way(10.0, 10)[9][0], way(10.0, 10)[9][1]);
    let askew = json.replacen(&format!("{stop}]"), "[10.0,1.0]]", 1);
    assert_ne!(askew, json);
    assert!(ParkingSystem::load_json(&askew).is_err());
}
