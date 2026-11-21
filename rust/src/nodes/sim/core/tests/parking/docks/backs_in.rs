// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-23` loading bays a truck backs into: the yard of `docks.rs` with its bay marked
//! `stand = "tail_in"`. The truck takes the same positions tick for tick as at the same dock
//! driven into nose first; only the pose the snapshot draws turns about in the yard. Saves, the
//! inspector line and a stand changed under a truck.

use super::*;
use crate::assets::asset::BayStand;
use crate::simulation::core::handles::Handle;
use crate::simulation::parking::ParkingTripPhase::{AtDock, PullIn, PullOut};
use crate::simulation::parking::{ParkingTripPhase, Place};

// The yard asset with its lane and a 6 m gate, its bay backed into when `tail_in`.
fn yard_with_stand(tail_in: bool) -> AssetManifest {
    let mut manifest = yard_manifest(true, "6");
    if tail_in {
        manifest.anchors[3].stand = Some(BayStand::TailIn);
    }
    manifest
}

// [`dispatched_truck`] with the yard's bay backed into or not: a truck spawned in the dock of a
// second yard 60 m east, bound for the first.
fn dispatched(tail_in: bool) -> (SimCore, u64, usize) {
    let manifest = yard_with_stand(tail_in);
    let id = manifest.qualified_id("freight");
    let mut core = core();
    core.allocator.registry.register("freight", manifest, String::new());
    let index = core.place_service_building_internal(&id, 0.0, -40.0).unwrap();
    core.publish_pending_building_site_changes();
    let building = core.allocator.building_handles.handle(index).to_bits();
    let source = core.place_service_building_internal(&id, 60.0, -40.0).unwrap();
    core.publish_pending_building_site_changes();
    let carrier = dispatch_shipment(&mut core, 0, source);
    (core, building, carrier)
}

fn at(core: &SimCore, i: usize) -> Vector2 {
    Vector2::new(core.agents.pos_x[i], core.agents.pos_y[i])
}

// The snapshot's facing of truck `i` (`access_phase_direction`), on the ground plane.
fn nose(core: &SimCore, i: usize) -> Option<Vector2> {
    crate::nodes::sim::core::snapshot::access_phase_direction(
        core,
        i,
        core.agents.pos_x[i],
        core.agents.pos_y[i],
    )
    .map(|d| Vector2::new(d.x, d.z))
}

// From a dock's stop towards the point before it on its route, out of the bay: where the nose of
// a truck standing tail in points.
fn out_of_bay(core: &SimCore, dock: Handle<Place>) -> Vector2 {
    let [.., [ax, az], [sx, sz]] = core.parking.dock_route(dock).unwrap();
    (Vector2::new(ax, az) - Vector2::new(sx, sz)).normalized()
}

fn yard_phase(core: &SimCore, i: usize) -> bool {
    let trip = core.agents.parking_trip[i];
    trip.freight_gate && matches!(trip.phase, PullIn | AtDock | PullOut)
}

// The whole round trip, spawned in the source's dock, on the road, backing into the yard's dock,
// standing, turned back and out to the road, run once with the bay driven into nose first and
// once backed into: every tick the same position, transit and trip row (bit for bit), and the
// same pose but in the yard, where the backed-into truck's is the reverse. Its nose points along
// its travel on the way out, against it on the way in, and out of the bay while it stands.
#[test]
fn a_backed_into_dock_moves_its_truck_as_before_and_turns_only_its_pose() {
    let (mut nosed, building, n) = dispatched(false);
    let (mut tailed, _, t) = dispatched(true);
    assert_eq!(n, t);
    let home = tailed.agents.parking_trip[t].source;
    assert!(tailed.parking.dock_backs_in(home));
    assert!(!nosed.parking.dock_backs_in(nosed.agents.parking_trip[n].source));
    let mut phases: Vec<ParkingTripPhase> = Vec::new();
    let mut turned_back = false;
    for tick in 0..6_000 {
        let (a, b) = (&nosed, &tailed);
        assert_eq!(
            (a.agents.pos_x[n].to_bits(), a.agents.pos_y[n].to_bits(), a.agents.transit[n]),
            (b.agents.pos_x[t].to_bits(), b.agents.pos_y[t].to_bits(), b.agents.transit[t]),
            "tick {tick}"
        );
        let trip = format!("{:?}", a.agents.parking_trip[n]);
        assert_eq!(trip, format!("{:?}", b.agents.parking_trip[t]), "tick {tick}");
        let phase = phase_of(a, n);
        let (front, back) = (nose(a, n), nose(b, t));
        if yard_phase(a, n) {
            let (front, back) = (front.expect("a pose"), back.expect("a pose"));
            assert!((front + back).length() < 1e-5, "tick {tick}: {front:?}, {back:?}");
            if phases.last() != Some(&phase) {
                phases.push(phase);
            }
        } else {
            assert_eq!(front, back, "tick {tick}");
        }
        let trip = a.agents.parking_trip[n];
        if phase == AtDock && trip.target == building && !turned_back {
            let dock = trip.destination;
            assert!(tailed.parking.dock_backs_in(dock));
            assert!(back.unwrap().dot(out_of_bay(&tailed, dock)) > 0.999, "it stands tail in");
            assert!(check_truck(&mut nosed) && check_truck(&mut tailed));
            turned_back = true;
            continue;
        }
        if turned_back && !yard_phase(&nosed, n) {
            break;
        }
        let before = at(&tailed, t);
        tick_agents(&mut nosed);
        tick_agents(&mut tailed);
        let travel = at(&tailed, t) - before;
        if travel.length() > 1e-4 && matches!(phase, PullIn | PullOut) && back.is_some() {
            let along = travel.normalized().dot(back.unwrap());
            if phase == PullIn {
                assert!(along < -0.7, "tick {tick}: it backs in ({along})");
            } else {
                assert!(along > 0.7, "tick {tick}: it leaves forward ({along})");
            }
        }
    }
    assert!(turned_back, "it reached the dock");
    assert_eq!(phases, [PullOut, PullIn, AtDock, PullOut], "every yard phase was compared");
}

// A dispatch from a yard whose bay is backed into spawns its truck standing tail in, nose out of
// the bay, and it leaves forward, nose first, along its route to the gate's lane point, where the
// dock is freed.
#[test]
fn a_truck_spawned_in_a_backed_into_dock_stands_tail_in_and_leaves_forward() {
    let (mut core, _, carrier) = dispatched(true);
    let trip = core.agents.parking_trip[carrier];
    assert_eq!(trip.phase, PullOut);
    let dock = trip.source;
    assert!(nose(&core, carrier).unwrap().dot(out_of_bay(&core, dock)) > 0.999);
    let mut moved = 0;
    for _ in 0..6_000 {
        if phase_of(&core, carrier) != PullOut {
            break;
        }
        let (before, facing) = (at(&core, carrier), nose(&core, carrier).unwrap());
        tick_agents(&mut core);
        let travel = at(&core, carrier) - before;
        if phase_of(&core, carrier) == PullOut && travel.length() > 1e-4 {
            assert!(travel.normalized().dot(facing) > 0.7, "nose first");
            moved += 1;
        }
    }
    assert_ne!(phase_of(&core, carrier), PullOut, "it left the yard");
    assert!(moved > 10);
    assert!(!core.agents.holds_freight_dock(core.agents.handles.handle(carrier), dock));
}

// A save made while a truck stands tail in, and one made while it leaves forward, continue tick
// for tick as the live city; the loaded dock still backs in. A save of format 124 (before stands
// were saved state) loads every dock nose in, as its format could hold no other.
#[test]
fn a_backed_into_dock_round_trips_a_save_standing_and_leaving() {
    let (mut core, building, carrier) = dispatched(true);
    tick_truck_until(&mut core, carrier, |core, i| {
        phase_of(core, i) == AtDock && core.agents.parking_trip[i].target == building
    });
    let dock = core.agents.parking_trip[carrier].destination;
    let standing = nose(&core, carrier).unwrap();
    let live = save_and_continue(&mut core, "backed_in_standing", 50, 0, |_| {});
    assert!(live.iter().all(|t| t == &live[0]), "it stands");
    assert!(core.parking.dock_backs_in(dock));
    let carrier = core.agents.resolve_agent(core.logistics.shipments[0].carrier).unwrap();
    assert!((nose(&core, carrier).unwrap() - standing).length() < 1e-6);
    assert!(check_truck(&mut core));
    for _ in 0..20 {
        tick_agents(&mut core);
    }
    assert_eq!(phase_of(&core, carrier), PullOut);
    let facing = nose(&core, carrier).unwrap();
    let path = std::env::temp_dir().join(format!("metrum_backs_in_{}.sqlite", std::process::id()));
    let path_text = path.to_str().expect("a temporary path");
    core.save_game_settled_at(path_text, None, 0).expect("the yard saves");
    core.load_game_internal(path_text).expect("the yard loads");
    let carrier = core.agents.resolve_agent(core.logistics.shipments[0].carrier).unwrap();
    assert_eq!(nose(&core, carrier), Some(facing), "it still faces out");
    assert_save_continues_equally(core, "backed_in_leaving", 200);

    let (mut core, building, _) = dispatched(false);
    core.save_game_settled_at(path_text, None, 0).expect("the yard saves");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(&format!(
            "UPDATE save_meta SET version = {};",
            crate::simulation::save::schema::PRE_STANDS_SAVE_VERSION
        ))
        .unwrap();
    let rows = core.parking.dock_rows();
    core.load_game_internal(path_text).expect("format 124 loads");
    let _ = std::fs::remove_file(&path);
    assert_eq!(core.parking.dock_rows(), rows);
    assert!(core.parking.building_dock_rows(building).all(|d| !core.parking.dock_backs_in(d)));
}

// The building inspector says a backed-into dock backs in and leaves forward (`PARK-23`).
#[test]
fn the_inspector_says_a_backed_into_dock_backs_in() {
    let (mut core, _, _) = dispatched(true);
    assert_eq!(
        core.dock_drive_lines(0),
        ["bay: backs in, leaves forward through its gate"]
    );
    reload_yard(&mut core, yard_with_stand(false));
    assert_eq!(core.dock_drive_lines(0), ["bay: backs out through its gate"]);
}

// The bay marked backed into while a truck stands in it nose first: the held row is retired with
// its stand, so the truck keeps its pose and backs out as it came in; the new row is backed into,
// and the next reconcile after the truck left leaves only it.
#[test]
fn a_stand_changed_under_a_standing_truck_keeps_its_pose() {
    let (mut core, building, carrier) = dispatched(false);
    tick_truck_until(&mut core, carrier, |core, i| {
        phase_of(core, i) == AtDock && core.agents.parking_trip[i].target == building
    });
    let dock = core.agents.parking_trip[carrier].destination;
    let standing = nose(&core, carrier).unwrap();
    reload_yard(&mut core, yard_with_stand(true));
    let active: Vec<_> = core.parking.building_docks(building).collect();
    assert_eq!(active.len(), 1);
    assert_ne!(active[0], dock, "the held row is retired");
    assert!(core.parking.dock_backs_in(active[0]) && !core.parking.dock_backs_in(dock));
    assert_eq!(nose(&core, carrier), Some(standing));
    assert!(check_truck(&mut core));
    let from = at(&core, carrier);
    for _ in 0..10 {
        tick_agents(&mut core);
    }
    let travel = (at(&core, carrier) - from).normalized();
    assert!(travel.dot(nose(&core, carrier).unwrap()) < -0.999, "it backs out");
    tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) != PullOut);
    reload_yard(&mut core, yard_with_stand(true));
    let rows = yard_docks(&core, building);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].1 && core.parking.dock_backs_in(rows[0].0));
}
