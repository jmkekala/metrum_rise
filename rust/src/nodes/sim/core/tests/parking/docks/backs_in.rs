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
        if let Some(back) = back
            && travel.length() > 1e-4
            && matches!(phase, PullIn | PullOut)
        {
            let along = travel.normalized().dot(back);
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

// `PARK-29`: a truck backing into its dock is drawn where one driving in nose first is, bit for
// bit, facing the other way, and one backing out of a dock likewise: the drawn turns round the
// polyline whichever way the truck faces along it (`turns` tests the arcs of a reversed route;
// this yard's route is straight).
#[test]
fn a_backed_into_dock_is_drawn_on_the_same_line_facing_back() {
    let (mut nosed, _, n) = dispatched(false);
    let (mut tailed, _, t) = dispatched(true);
    let (mut compared, mut pulled_in) = (0, false);
    for _ in 0..6_000 {
        tick_agents(&mut nosed);
        tick_agents(&mut tailed);
        match phase_of(&nosed, n) {
            PullIn => pulled_in = true,
            AtDock if pulled_in => break,
            _ => {}
        }
        let (Some(front), Some(back)) = (drawn_truck(&nosed, n), drawn_truck(&tailed, t)) else {
            continue;
        };
        assert_eq!((front.1.x.to_bits(), front.1.y.to_bits()), (back.1.x.to_bits(), back.1.y.to_bits()));
        let (a, b) = (front.2.expect("a facing"), back.2.expect("a facing"));
        assert!((a + b).length() < 1e-5, "{a:?} {b:?}");
        compared += 1;
    }
    assert!(pulled_in && compared > 20, "compared {compared}");
}

// `RENDER-66`: the truck leaves its source's tail-in dock forward with no reverse lamp, backs
// into the first yard's with its reverse lamp lit, no indicator and its front wheels straight
// (this yard's route is straight; the vehicle signals' tests steer a backing truck on an arc),
// and stands in the dock with every lamp dark.
#[test]
fn a_truck_backing_into_a_tail_in_dock_shows_its_reverse_lamp() {
    use crate::nodes::sim::core::snapshot::off_lane_car_drawn;
    use crate::nodes::sim::render::vehicle_catalogue::Axles;
    use crate::nodes::sim::render::vehicle_signals::{
        CarPath, LAMP_LEFT, LAMP_REVERSE, LAMP_RIGHT,
    };
    let (mut core, building, t) = dispatched(true);
    let truck = Axles {
        front_m: 2.55,
        rear_m: -1.95,
    };
    // Ticks seen backing in, standing, and leaving forward.
    let mut seen = [0; 3];
    let mut turned_back = false;
    for tick in 0..6_000 {
        let trip = core.agents.parking_trip[t];
        let off_lane = core.agents.current_lane_id[t] == usize::MAX;
        let lamps = core.posed_car_custom(t).map_or(0, |(_, custom)| custom[2] as u32 & 0xff);
        match trip.phase {
            PullIn if off_lane && trip.target == building => {
                assert_ne!(lamps & LAMP_REVERSE, 0, "tick {tick}");
                assert_eq!(lamps & (LAMP_LEFT | LAMP_RIGHT), 0, "tick {tick}");
                let drawn = off_lane_car_drawn(&core, t, core.agents.pos_x[t], core.agents.pos_y[t]);
                let path = CarPath::OffLane {
                    facing: drawn.facing,
                    curvature: drawn.curvature,
                };
                let (_, steer) = core.car_signals(t, Some(truck), 0, None, path);
                // Past the gate turn (`PARK-31`), which steers it next to the lane point.
                let lane_point = crate::nodes::sim::render::lane_pose::sample_lane_pose(
                    &core.transit_network.lane_system.lanes
                        [core.agents.planned_detach_lane_id[t] as usize],
                    core.agents.planned_detach_lane_d[t],
                )
                .map(|(p, _)| Vector2::new(p.x, p.z))
                .unwrap();
                if at(&core, t).distance_to(lane_point)
                    > crate::simulation::parking::turns::FREIGHT_GATE_RADIUS_M + 0.05
                {
                    assert_eq!((drawn.curvature, steer), (0.0, 0.0), "tick {tick}");
                }
                seen[0] += 1;
            }
            AtDock => {
                assert_eq!(lamps, 0, "tick {tick}");
                seen[1] += 1;
                if !turned_back {
                    assert!(check_truck(&mut core));
                    turned_back = true;
                    continue;
                }
            }
            PullOut if off_lane => {
                assert_eq!(lamps & LAMP_REVERSE, 0, "tick {tick}");
                seen[2] += 1;
            }
            _ => {}
        }
        if turned_back && !yard_phase(&core, t) {
            break;
        }
        tick_agents(&mut core);
    }
    assert!(seen.iter().all(|&ticks| ticks > 0), "{seen:?}");
}

// `PARK-31`: the truck's round trip near both gates as the snapshot poses it: out of the source's
// tail-in dock forward and onto its lane on the gate turn, along the road, and off it on the turn
// into the first yard, backing from the lane point on. Consecutive poses never jump in position
// and the drawn line never turns faster than a 1 m arc (the nose reverses only at the lane point
// going in, where the truck changes from driving to backing); on its lane it is drawn off the
// centre line (the lane half), within the lane offset bound, and off it within 3 m of where it is.
#[test]
fn a_truck_joins_and_leaves_its_lane_on_the_gate_turn() {
    use crate::nodes::sim::render::lane_pose::sample_lane_pose;
    use crate::simulation::parking::turns::{FREIGHT_GATE_RADIUS_M, GATE_LANE_OFFSET_M};
    let (mut core, _, t) = dispatched(true);
    let lane_point = |core: &SimCore, lane: u32, d: f32| {
        core.transit_network
            .lane_system
            .lanes
            .get(lane as usize)
            .and_then(|lane| sample_lane_pose(lane, d))
            .map(|(p, _)| Vector2::new(p.x, p.z))
    };
    let mut gates: [Option<Vector2>; 2] = [None, None];
    let mut previous: Option<(Vector2, Vector2, Vector2)> = None;
    let (mut on_arc, mut switches, mut last_lane, mut pulled_in) = ([0; 2], 0, None, false);
    for _ in 0..6_000 {
        tick_agents(&mut core);
        let a = &core.agents;
        let trip = a.parking_trip[t];
        match trip.phase {
            PullOut => gates[0] = lane_point(&core, a.planned_attach_lane_id[t], a.planned_attach_lane_d[t]),
            PullIn => pulled_in = true,
            AtDock if pulled_in => break,
            ParkingTripPhase::None if trip.freight_gate => {
                gates[1] = lane_point(&core, a.planned_detach_lane_id[t], a.planned_detach_lane_d[t])
            }
            _ => {}
        }
        let Some((drawn, nose, on_lane, _)) = core.posed_car_pose(t) else {
            previous = None;
            continue;
        };
        let lane = a.current_lane_id[t];
        let stored = core
            .transit_network
            .lane_system
            .lanes
            .get(lane)
            .and_then(|l| sample_lane_pose(l, a.lane_distance[t]))
            .map_or(at(&core, t), |(p, _)| Vector2::new(p.x, p.z));
        let near = gates
            .iter()
            .position(|g| g.is_some_and(|g| g.distance_to(stored) < FREIGHT_GATE_RADIUS_M + 4.0));
        let Some(gate) = near else {
            previous = None;
            continue;
        };
        let off = drawn.distance_to(stored);
        if on_lane {
            assert!(off <= GATE_LANE_OFFSET_M, "{off} m off its lane pose");
            on_arc[gate] += usize::from(off > 0.5);
        } else {
            assert!(off <= 3.0, "{off} m off where it is");
        }
        if last_lane.is_some_and(|last| last != on_lane) {
            switches += 1;
        }
        last_lane = Some(on_lane);
        if let Some((s0, d0, n0)) = previous {
            let moved = s0.distance_to(stored);
            assert!(drawn.distance_to(d0) <= 1.6 * moved + 0.05, "{d0:?} -> {drawn:?} for {moved} m");
            let turn = n0.dot(nose).abs().clamp(-1.0, 1.0).acos();
            assert!(
                turn <= 1.3 * moved + 5f32.to_radians() + 0.01,
                "turns {}° for {moved} m at {drawn:?}",
                turn.to_degrees()
            );
        }
        previous = Some((stored, drawn, nose));
    }
    assert!(pulled_in, "it pulled in");
    assert!(switches >= 2, "{switches}");
    assert!(on_arc[0] > 0 && on_arc[1] > 0, "{on_arc:?}");
}
