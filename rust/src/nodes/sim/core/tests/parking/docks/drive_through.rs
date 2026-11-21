// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-21` drive-through docks in a placed city: two copies of the proven drive-through yard
//! (in at `truck_in`, out round the half circle to `truck_out`), the pose the snapshot draws, save
//! and reload at every phase, and the edits that retire a held dock or move its exit.

use super::*;
use crate::simulation::core::handles::Handle;
use crate::simulation::parking::ParkingTripPhase::{self, AtDock, PullIn, PullOut};
use crate::simulation::parking::Place;

// A truck from a second drive-through yard 60 m east to the first, just dispatched: standing in
// its source's dock, about to leave by that yard's exit.
fn dispatched_through_truck() -> (SimCore, u64, usize) {
    let (mut core, building, id) = place_drive_through();
    let source = core
        .place_service_building_internal(&id, 60.0, -40.0)
        .unwrap();
    core.publish_pending_building_site_changes();
    let carrier = dispatch_shipment(&mut core, 0, source);
    (core, building, carrier)
}

fn first_dock(core: &SimCore, building: u64) -> Handle<Place> {
    core.parking
        .building_docks(building)
        .next()
        .expect("a dock")
}

fn at(core: &SimCore, i: usize) -> Vector2 {
    Vector2::new(core.agents.pos_x[i], core.agents.pos_y[i])
}

// The way a dock's truck leaves by: its way out, world points.
fn way_out(core: &SimCore, dock: Handle<Place>) -> Vec<Vector2> {
    let way = core.parking.dock_way(dock).expect("a drive");
    way.route_out
        .as_slice()
        .iter()
        .map(|&[x, z]| Vector2::new(x, z))
        .collect()
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

// Ticks truck `i` while it is in `phase`, asserting each tick that it holds `dock` and that the
// drawn pose faces its travel (never backwards). Returns its positions.
fn drive_facing_travel(
    core: &mut SimCore,
    i: usize,
    phase: ParkingTripPhase,
    dock: Handle<Place>,
) -> Vec<Vector2> {
    let mut taken = vec![at(core, i)];
    for _ in 0..6_000 {
        let before = at(core, i);
        let facing = nose(core, i);
        tick_agents(core);
        let now = at(core, i);
        taken.push(now);
        if phase_of(core, i) != phase {
            return taken;
        }
        assert!(
            core.agents
                .holds_freight_dock(core.agents.handles.handle(i), dock),
            "{phase:?}"
        );
        if now.distance_to(before) > 1e-4 {
            let travel = (now - before).normalized();
            let facing = facing.expect("a pose");
            assert!(
                travel.dot(facing) > 0.7,
                "{phase:?}: travel {travel:?}, nose {facing:?} at {before:?}"
            );
        }
    }
    panic!("never left {phase:?}");
}

// A delivered truck leaves its drive-through dock forward along the proven way out, the snapshot
// drawing it nose first all the way, and joins the road at the exit gate's lane point, where the
// dock is freed. A dispatch spawned in the source's dock leaves the same way; a truck arriving
// drives the way in nose first.
#[test]
fn a_drive_through_truck_faces_its_travel_and_leaves_by_the_exit() {
    let (mut core, building, carrier) = dispatched_through_truck();
    let home = core.agents.parking_trip[carrier].source;
    assert!(core.parking.dock_leaves_forward(home));
    let out = way_out(&core, home);
    let taken = drive_facing_travel(&mut core, carrier, PullOut, home);
    let joined = taken[taken.len() - 2];
    assert!(
        joined.distance_to(out[out.len() - 1]) < 6.0,
        "it left by its exit: {joined:?}, {out:?}"
    );
    assert!(
        !core
            .agents
            .holds_freight_dock(core.agents.handles.handle(carrier), home)
    );

    tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == PullIn);
    let dock = first_dock(&core, building);
    drive_facing_travel(&mut core, carrier, PullIn, dock);
    assert_eq!(phase_of(&core, carrier), AtDock);
    let standing = nose(&core, carrier).unwrap();
    assert!(check_truck(&mut core));
    assert!(
        (nose(&core, carrier).unwrap() - standing).length() < 1e-5,
        "it turned on the spot"
    );
    let out = way_out(&core, dock);
    let taken = drive_facing_travel(&mut core, carrier, PullOut, dock);
    let joined = taken[taken.len() - 2];
    assert!(
        joined.distance_to(out[out.len() - 1]) < 6.0,
        "it left by its exit: {joined:?}"
    );
    // Every point it took lies on its way out or the last leg to the lane.
    let lane = *taken.last().unwrap();
    let mut line = out.clone();
    line.push(lane);
    for p in &taken[..taken.len() - 1] {
        let off = line
            .windows(2)
            .map(|w| {
                let ab = w[1] - w[0];
                let t = ((*p - w[0]).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                p.distance_to(w[0] + ab * t)
            })
            .fold(f32::INFINITY, f32::min);
        assert!(off < 0.05, "off its way at {p:?}");
    }
}

// Save and reload at every phase of a drive-through trip: on the road to the gate, on the way in,
// standing, on the way out after a delivery and after a dispatch. The live city and the loaded
// one continue tick for tick alike.
#[test]
fn a_drive_through_truck_round_trips_a_save_at_every_phase() {
    let (mut core, _, carrier) = dispatched_through_truck();
    for _ in 0..20 {
        tick_agents(&mut core);
    }
    assert_eq!(phase_of(&core, carrier), PullOut);
    let live = save_and_continue(&mut core, "through_spawned", 300, 0, |_| {});
    assert!(
        live.iter().any(|t| !t.3.contains("PullOut")),
        "it left its source's yard"
    );

    let (mut core, _, carrier) = dispatched_through_truck();
    tick_truck_until(&mut core, carrier, |core, i| {
        phase_of(core, i) == ParkingTripPhase::None
            && core.agents.transit[i] == crate::simulation::economy::agents::TRANSIT_NETWORK
    });
    let live = save_and_continue(&mut core, "through_on_the_road", 400, 0, |_| {});
    assert!(
        live.iter().any(|t| t.3.contains("PullIn")),
        "it reached the gate"
    );

    let (mut core, _, carrier) = dispatched_through_truck();
    tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == PullIn);
    for _ in 0..10 {
        tick_agents(&mut core);
    }
    let live = save_and_continue(&mut core, "through_pulling_in", 200, 0, |_| {});
    assert!(
        live[0].3.contains("PullIn") && live[199].3.contains("AtDock"),
        "{live:?}"
    );

    let live = save_and_continue(&mut core, "through_standing", 50, 0, |_| {});
    assert!(live.iter().all(|t| t == &live[0]), "it stands");

    assert!(check_truck(&mut core));
    for _ in 0..40 {
        tick_agents(&mut core);
    }
    assert_eq!(phase_of(&core, carrier), PullOut);
    let live = save_and_continue(&mut core, "through_leaving", 300, 0, |_| {});
    assert!(!live[299].3.contains("PullOut"), "it left the yard");
}

// An edit to the yard's asset under a truck that holds its drive-through dock: the exit and way
// out removed (the dock now backed out of through `truck_in`, so the building's departure gate
// moves to it), or the dock dropped (the lane passage removed, so the gate record goes). The held
// row is retired with its way, and the truck finishes its trip on it in every phase: on the way
// in it ends in the bay; standing, turned back, it plans from the dock's own exit (the gate
// record's departure is no longer its own) and leaves forward by it; on the way out it goes on.
// The next reconcile after it left removes the row.
#[test]
fn an_edit_under_a_drive_through_truck_keeps_its_way_for_it() {
    for drop_dock in [false, true] {
        for phase in [PullIn, AtDock, PullOut] {
            let (mut core, building, carrier) = dispatched_through_truck();
            let reached = if phase == PullOut { AtDock } else { phase };
            tick_truck_until(&mut core, carrier, |core, i| {
                phase_of(core, i) == reached && core.agents.parking_trip[i].target == building
            });
            if phase == PullOut {
                assert!(check_truck(&mut core));
                for _ in 0..30 {
                    tick_agents(&mut core);
                }
            }
            let dock = first_dock(&core, building);
            let out = way_out(&core, dock);
            let manifest = if drop_dock {
                let mut manifest = drive_through_manifest(true);
                for part in &mut manifest.mesh_parts {
                    part.passages.clear();
                }
                manifest
            } else {
                drive_through_manifest(false)
            };
            reload_yard(&mut core, manifest);
            let label = format!("{phase:?}, dock dropped {drop_dock}");
            let active: Vec<_> = core.parking.building_docks(building).collect();
            assert!(!active.contains(&dock), "{label}: the held row is retired");
            assert_eq!(way_out(&core, dock), out, "{label}: with its way");
            assert_eq!(
                core.allocator.freight_gates.contains_key(&building),
                !drop_dock,
                "{label}"
            );
            if !drop_dock {
                assert!(
                    !core.parking.dock_leaves_forward(active[0]),
                    "{label}: the new dock backs out"
                );
                let gate = &core.allocator.freight_gates[&building];
                assert_ne!(
                    gate.leave.curb_pos,
                    out[out.len() - 1],
                    "{label}: the departure moved"
                );
                assert_eq!(gate.elsewhere, [dock.to_bits()], "{label}");
            }
            if phase != PullOut {
                tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == AtDock);
                assert!(check_truck(&mut core), "{label}: turned back");
                assert_eq!(phase_of(&core, carrier), PullOut);
                tick_agents(&mut core);
                let trip = core.agents.parking_trip[carrier];
                assert!(
                    !trip.ready && !trip.retry_pending,
                    "{label}: planned by the commit: {trip:?}"
                );
            }
            let taken = drive_facing_travel(&mut core, carrier, PullOut, dock);
            let joined = taken[taken.len() - 2];
            assert!(
                joined.distance_to(out[out.len() - 1]) < 6.0,
                "{label}: left by its old exit at {joined:?}"
            );
            assert!(
                !core
                    .agents
                    .holds_freight_dock(core.agents.handles.handle(carrier), dock)
            );
            reload_yard(&mut core, drive_through_manifest(true));
            // One row again, held by nobody: the retired row, matching the dock again by its
            // pose and way, is taken up as the A2 rule keeps a row, or removed.
            let rows = core.parking.source_bays(PlaceSource::Dock {
                building,
                anchor: 4,
            });
            assert_eq!(rows.len(), 1, "{label}: {rows:?}");
            let row = Handle::from_bits(rows[0].0);
            assert_eq!(
                core.parking.building_docks(building).collect::<Vec<_>>(),
                [row],
                "{label}"
            );
            assert!(core.parking.dock_holder(row).is_none(), "{label}");
        }
    }
}

// A revalidation that leaves a drive-through dock as it was keeps the row, its way and its truck;
// one whose way is unchanged but whose truck stands keeps it too.
#[test]
fn a_revalidation_leaves_a_held_drive_through_dock_as_it_was() {
    let (mut core, building, carrier) = dispatched_through_truck();
    tick_truck_until(&mut core, carrier, |core, i| {
        phase_of(core, i) == AtDock && core.agents.parking_trip[i].target == building
    });
    let dock = first_dock(&core, building);
    let way = *core.parking.dock_way(dock).unwrap();
    reload_yard(&mut core, drive_through_manifest(true));
    assert_eq!(first_dock(&core, building), dock);
    assert_eq!(core.parking.dock_way(dock), Some(&way));
    assert!(core.allocator.freight_gates[&building].elsewhere.is_empty());
    assert!(check_truck(&mut core));
    drive_facing_travel(&mut core, carrier, PullOut, dock);
}

// The exit moved (`truck_out` 1 m west, not removed) under a truck holding its drive-through
// dock: the way out changes, so the held row is retired with its old way and a new row takes the
// new one; the building's departure moves to the new exit and the retired row is `elsewhere`. The
// truck finishes on its old way in every phase, nose first, and leaves by the old exit; the next
// reconcile leaves the new row alone.
#[test]
fn an_exit_moved_under_a_drive_through_truck_keeps_its_old_way_for_it() {
    for phase in [PullIn, AtDock, PullOut] {
        let (mut core, building, carrier) = dispatched_through_truck();
        let reached = if phase == PullOut { AtDock } else { phase };
        tick_truck_until(&mut core, carrier, |core, i| {
            phase_of(core, i) == reached && core.agents.parking_trip[i].target == building
        });
        if phase == PullOut {
            assert!(check_truck(&mut core));
            for _ in 0..30 {
                tick_agents(&mut core);
            }
        }
        let dock = first_dock(&core, building);
        let out = way_out(&core, dock);
        reload_yard(&mut core, drive_through_manifest_at(true, "6.5"));
        let label = format!("{phase:?}");
        let active: Vec<_> = core.parking.building_docks(building).collect();
        assert!(!active.contains(&dock) && active.len() == 1, "{label}: the held row is retired");
        assert_eq!(way_out(&core, dock), out, "{label}: with its way");
        let moved = way_out(&core, active[0]);
        assert!(
            (moved[moved.len() - 1].x - out[out.len() - 1].x + 1.0).abs() < 1e-3,
            "{label}: the new way leaves by the moved exit: {moved:?}"
        );
        assert!(core.parking.dock_leaves_forward(active[0]), "{label}");
        let gate = &core.allocator.freight_gates[&building];
        let curb = gate.leave.curb_pos;
        assert!(
            curb.distance_to(moved[moved.len() - 1]) < curb.distance_to(out[out.len() - 1]),
            "{label}: the departure moved to the new exit: {curb:?}"
        );
        assert_eq!(gate.elsewhere, [dock.to_bits()], "{label}");
        if phase != PullOut {
            tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == AtDock);
            assert!(check_truck(&mut core), "{label}: turned back");
            tick_agents(&mut core);
            let trip = core.agents.parking_trip[carrier];
            assert!(
                trip.phase == PullOut && !trip.ready && !trip.retry_pending,
                "{label}: planned by the commit: {trip:?}"
            );
        }
        let taken = drive_facing_travel(&mut core, carrier, PullOut, dock);
        let joined = taken[taken.len() - 2];
        assert!(
            joined.distance_to(out[out.len() - 1]) < joined.distance_to(moved[moved.len() - 1]),
            "{label}: left by its old exit at {joined:?}"
        );
        reload_yard(&mut core, drive_through_manifest_at(true, "6.5"));
        let rows = core.parking.source_bays(PlaceSource::Dock {
            building,
            anchor: 4,
        });
        // One row again, active, held by nobody, with the new way (the A2 rule takes up the
        // first row whose pose matches and removes the other).
        assert_eq!(rows.len(), 1, "{label}: {rows:?}");
        let row = Handle::from_bits(rows[0].0);
        assert_eq!(core.parking.building_docks(building).collect::<Vec<_>>(), [row], "{label}");
        assert_eq!(way_out(&core, row), moved, "{label}");
        assert!(core.parking.dock_holder(row).is_none(), "{label}");
    }
}

// The building inspector's dock lines name a drive-through's exit, and say a dock backed out of
// backs out (`PARK-21`).
#[test]
fn the_inspector_says_how_each_dock_is_driven() {
    let (mut core, _, _) = place_drive_through();
    assert_eq!(
        core.dock_drive_lines(0),
        ["bay: drive-through, leaves forward by 'truck_out'"]
    );
    reload_yard(&mut core, drive_through_manifest(false));
    assert_eq!(core.dock_drive_lines(0), ["bay: backs out through its gate"]);
}
