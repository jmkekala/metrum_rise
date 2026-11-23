// SPDX-License-Identifier: GPL-3.0-or-later

//! Drive-through docks (`PARK-21`) on the minimal straight road: a truck drives its dock's way
//! in from the arrival gate, stands, and leaves forward along the way out to the exit gate, where
//! it joins the lane and frees the dock. A dispatch spawned in such a dock leaves by the exit, a
//! truck returning to it arrives by the way in, and recovery plans from the exit.

use super::*;
use crate::simulation::parking::{DockRoute, DockWay};

// The exit gate's road connection: 16 m east of the arrival gate's.
const EXIT_OFFSET: f32 = 16.0;
const EXIT_X: f32 = GATE_X + EXIT_OFFSET;

// Building `building`'s one drive-through dock, its access and its gate record, as the site
// reconcile makes them (`SimCore::publish_docks`, `rebuild_freight_gate`): in at the gate 20 m
// east of the door, 20 m into the yard to the stop, on 8 m, round a U of 45° chords and back to the
// exit 16 m east of the gate. Returns the way.
fn publish_drive_through(yard: &mut Yard, building: usize) -> DockWay {
    publish_drive_through_deep(yard, building, 20.0)
}

// [`publish_drive_through`] with the stop `depth` metres into the yard from the gate (more than
// 14): its way in holds `floor((depth - 4) / 9)` trucks waiting behind the bay (`PARK-36`), one
// for 20 m.
fn publish_drive_through_deep(yard: &mut Yard, building: usize, depth: f32) -> DockWay {
    let handle = yard.allocator.building_handles.handle(building).to_bits();
    let entrance = yard.allocator.entrances[building].clone();
    let k = entrance.curb_pos.y;
    let g = yard.allocator.buildings[building].center_x + GATE_OFFSET;
    let e = g + EXIT_OFFSET;
    let d = depth;
    let route_in = [[g, k], [g, k + 4.25], [g, k + 14.0], [g, k + d]];
    let route_out = [
        [g, k + d],
        [g, k + d + 8.0],
        [g + 4.0, k + d + 12.0],
        [g + 12.0, k + d + 12.0],
        [e, k + d + 8.0],
        [e, k + 4.25],
        [e, k],
    ];
    let way = DockWay {
        route_in: DockRoute::new(&route_in).unwrap(),
        route_out: DockRoute::new(&route_out).unwrap(),
    };
    let bay = Bay {
        identity: [handle, 0],
        position: [g, 0.0, k + d],
        path: route_in,
        heading: 0.0,
        width: 3.5,
        length: 8.0,
    };
    yard.parking.replace_docks(handle, &[(0, bay)], &[(0, way)], &[]);
    yard.parking
        .refresh_dock_access(handle, &yard.allocator, &yard.network, &yard.graph);
    let b = &yard.allocator.buildings[building];
    let access = |x: f32| {
        BuildingAllocator::parking_access_at(
            entrance.door_pos,
            Vector2::new(x, k),
            b.edge_idx,
            b.side,
            &yard.graph,
            &yard.network.lane_system,
        )
    };
    let gate = FreightGate {
        arrive: access(g),
        leave: access(e),
        elsewhere: Vec::new(),
    };
    yard.allocator.freight_gates.insert(handle, gate);
    way
}

// The grocery with a drive-through dock; the processor with one too (`source`), with a stage A
// dock (`stage_a`), or with none.
fn yard_through(source: bool, stage_a: bool) -> (Yard, DockWay) {
    let mut yard = yard_with(u32::from(stage_a), 0);
    if source {
        publish_drive_through(&mut yard, 0);
    }
    let way = publish_drive_through(&mut yard, 1);
    (yard, way)
}

fn points(route: &DockRoute) -> Vec<Vector2> {
    route
        .as_slice()
        .iter()
        .map(|&[x, z]| Vector2::new(x, z))
        .collect()
}

// Where `p` lies along the polyline `line`: the distance from its start to `p`'s nearest point
// on it, and how far `p` is from it.
fn along(line: &[Vector2], p: Vector2) -> (f32, f32) {
    let mut best = (0.0, f32::INFINITY);
    let mut start = 0.0;
    for w in line.windows(2) {
        let ab = w[1] - w[0];
        let t = ((p - w[0]).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
        let off = p.distance_to(w[0] + ab * t);
        if off < best.1 - 1e-6 {
            best = (start + ab.length() * t, off);
        }
        start += ab.length();
    }
    best
}

// Ticks truck `i` along the way `line` (lane point included) while `phase` holds, asserting each
// tick that it is on the way, never goes back along it, faces its travel (the next point of the
// way is ahead of it, as the snapshot draws it), holds `dock` and moves at most the driveway
// speed. Returns the positions it took, the last one after the phase ended.
fn drive_along(
    yard: &mut Yard,
    i: usize,
    phase: ParkingTripPhase,
    dock: crate::simulation::core::handles::Handle<crate::simulation::parking::Place>,
    line: &[Vector2],
) -> Vec<Vector2> {
    let at = |y: &Yard| Vector2::new(y.agents.agents.pos_x[i], y.agents.agents.pos_y[i]);
    let mut taken = vec![at(yard)];
    let (mut done, off) = along(line, taken[0]);
    assert!(off < 1e-3, "it starts on its way: {:?}", taken[0]);
    for _ in 0..6_000 {
        yard.tick();
        let now = at(yard);
        let last = *taken.last().unwrap();
        taken.push(now);
        if yard.phase(i) != phase {
            return taken;
        }
        assert!(yard.holds(i, dock), "the dock is its own on its way");
        let (progress, off) = along(line, now);
        // The walk takes the next leg within 5 cm of a corner (`local_access_target_segment`).
        assert!(off < 0.05, "off its way at {now:?}");
        assert!(progress >= done - 1e-4, "it went back: {last:?} -> {now:?}");
        assert!(now.distance_to(last) <= 0.1 * crate::config::FREIGHT_YARD_SPEED_MS + 1e-4);
        if now.distance_to(last) > 1e-4 {
            // The way's next point, which the drawn pose faces, lies ahead of its travel.
            let next =
                crate::simulation::economy::agents::tick::parking_path_target_segment(now, line)
                    .map_or(line[line.len() - 1], |k| line[k]);
            let travel = (now - last).normalized();
            if next.distance_to(now) > 1e-3 {
                assert!(
                    travel.dot((next - now).normalized()) > 0.7,
                    "facing {next:?}, travelling {travel:?}"
                );
            }
        }
        done = progress;
    }
    panic!("never left {phase:?}");
}

// Truck `i`'s way out of `way` and on to its planned lane point.
fn way_out(yard: &Yard, i: usize, way: &DockWay) -> Vec<Vector2> {
    let mut line = points(&way.route_out);
    line.push(attach_point(yard, i));
    line
}

// Truck `i`'s way in from its planned lane point along `way`.
fn way_in(yard: &Yard, i: usize, way: &DockWay) -> Vec<Vector2> {
    let mut line = vec![detach_point(yard, i)];
    line.extend(points(&way.route_in));
    line
}

// A truck with a drive-through dock arrives at the arrival gate, drives the way in, stands, and
// after delivering leaves forward by the way out to the exit gate, the dock its own until it joins
// the lane there. Its return then runs to its source's door, as before.
#[test]
fn a_truck_drives_through_its_dock_and_leaves_forward_by_the_exit() {
    let (mut yard, way) = yard_through(false, false);
    let carrier = yard.dispatch(0);
    assert!(
        (detach_point(&yard, carrier).x - GATE_X).abs() < 0.5,
        "it arrives at the gate"
    );
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::PullIn);
    let dock = yard.dock(0);
    let line = way_in(&yard, carrier, &way);
    let taken = drive_along(&mut yard, carrier, ParkingTripPhase::PullIn, dock, &line);
    assert_eq!(yard.phase(carrier), ParkingTripPhase::AtDock);
    let stop = way.route_in.as_slice()[3];
    assert_eq!(*taken.last().unwrap(), Vector2::new(stop[0], stop[1]));

    assert!(yard.check(0));
    let trip = yard.agents.agents.parking_trip[carrier];
    assert_eq!((trip.phase, trip.source), (ParkingTripPhase::PullOut, dock));
    assert!(yard.parking.dock_leaves_forward(dock));
    let exit = attach_point(&yard, carrier);
    assert!(
        (exit.x - EXIT_X).abs() < 0.5,
        "it leaves by the exit: {exit:?}"
    );
    let line = way_out(&yard, carrier, &way);
    let taken = drive_along(&mut yard, carrier, ParkingTripPhase::PullOut, dock, &line);
    // At the exit's lane point: the yard trip is over and the dock free.
    assert!(
        taken.last().unwrap().distance_to(exit) < 1e-3,
        "{:?}",
        taken.last()
    );
    assert!(
        !yard
            .agents
            .holds_freight_dock(yard.agents.handles.handle(carrier), dock)
    );
    assert!(idle(yard.agents.agents.parking_trip[carrier]));
    yard.tick_until(|y| y.at_kerb(carrier, 0));
    assert!(!yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Fulfilled);
}

// A dispatch from a building with a free drive-through dock spawns its truck in the dock, which
// leaves forward by the exit; bound for the grocery's yard it drives through that too, and back
// at its source it arrives by the way in and stands in its dock, where the last check removes it.
#[test]
fn a_dispatch_in_a_drive_through_dock_leaves_by_the_exit_and_returns_by_the_way_in() {
    let (mut yard, way) = yard_through(true, false);
    let source_way = {
        let mut way = way;
        let shift = |route: &DockRoute| {
            let p: Vec<_> = route
                .as_slice()
                .iter()
                .map(|&[x, z]| [x - (YARD_X - SOURCE_X), z])
                .collect();
            DockRoute::new(&p).unwrap()
        };
        way.route_in = shift(&way.route_in);
        way.route_out = shift(&way.route_out);
        way
    };
    let carrier = yard.dispatch(0);
    let home = yard.dock_of(0, 0);
    let trip = yard.agents.agents.parking_trip[carrier];
    assert_eq!((trip.phase, trip.source), (ParkingTripPhase::PullOut, home));
    let exit = SOURCE_X + GATE_OFFSET + EXIT_OFFSET;
    assert!(
        (attach_point(&yard, carrier).x - exit).abs() < 0.5,
        "it leaves by the exit"
    );
    let line = way_out(&yard, carrier, &source_way);
    drive_along(&mut yard, carrier, ParkingTripPhase::PullOut, home, &line);
    let trip = yard.agents.agents.parking_trip[carrier];
    assert!(
        trip.freight_gate && trip.phase == ParkingTripPhase::None,
        "on to the yard's gate"
    );

    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
    assert!(yard.check(0));
    assert!(
        (detach_point(&yard, carrier).x - (SOURCE_X + GATE_OFFSET)).abs() < 0.5,
        "bound for its source's arrival gate"
    );
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::PullIn);
    assert_eq!(yard.agents.agents.parking_trip[carrier].destination, home);
    let line = way_in(&yard, carrier, &source_way);
    drive_along(&mut yard, carrier, ParkingTripPhase::PullIn, home, &line);
    assert_eq!(yard.phase(carrier), ParkingTripPhase::AtDock);
    let removed = yard.handle(carrier);
    assert!(!yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Fulfilled);
    assert!(yard.agents.resolve_agent(removed).is_none());
}

// Two trucks through one drive-through dock: the first drives through and stands, the second
// finds the dock held at the gate and waits at the kerb stop (`PARK-36`) until the first has
// driven out, then docks and delivers; both go home. Every product is the same at 1 and 8
// workers.
#[test]
fn drive_through_products_are_equal_on_any_worker_count() {
    let run = |workers: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let (mut yard, _) = yard_through(true, false);
            let home = yard.dock_of(0, 0);
            let first = yard.dispatch(0);
            let state = |y: &Yard, i: usize| {
                let a = &y.agents.agents;
                (
                    a.pos_x[i].to_bits(),
                    a.pos_y[i].to_bits(),
                    a.transit[i],
                    format!("{:?}", a.parking_trip[i]),
                )
            };
            let mut trace = Vec::new();
            yard.tick_until(|y| {
                trace.push(state(y, first));
                y.phase(first) == ParkingTripPhase::AtDock
            });
            let second = yard.dispatch(1);
            // Out of its source's dock and on to the yard's gate, where the commit answers.
            yard.tick_until(|y| {
                matches!(
                    y.phase(second),
                    ParkingTripPhase::PullIn | ParkingTripPhase::ToKerb | ParkingTripPhase::AtKerb
                )
            });
            let refused = (yard.phase(second), yard.agents.agents.parking_trip[second].destination);
            yard.tick_until(|y| y.phase(second) == ParkingTripPhase::AtKerb);
            assert!(yard.check(0));
            yard.tick_until(|y| {
                trace.push(state(y, first));
                trace.push(state(y, second));
                assert!(!y.inside_grocery(second));
                y.phase(second) == ParkingTripPhase::AtDock
            });
            assert!(yard.check(1));
            // Both home: standing in the source's dock, or at its kerb stop when it was held.
            yard.tick_until(|y| {
                trace.push(state(y, first));
                trace.push(state(y, second));
                [first, second].iter().all(|&i| {
                    let trip = y.agents.agents.parking_trip[i];
                    (trip.phase == ParkingTripPhase::AtDock && trip.destination == home)
                        || trip.phase == ParkingTripPhase::AtKerb
                })
            });
            (trace, refused, yard.status(0), yard.status(1))
        })
    };
    let serial = run(1);
    let yard = yard_through(true, false).0;
    assert_eq!(
        serial.1,
        (ParkingTripPhase::AtKerb, yard.dock(0)),
        "the held dock stands it on the way in (`PARK-36`)"
    );
    assert_eq!(run(8), serial);
}

// A truck on its way out whose plan a road edit drops, or whose way onto the road goes stale,
// stops where it is, keeps its dock, and the parking phase plans its way out again from the exit
// in the same tick; it goes on forward from where it stood. On the way in, a dropped plan ends it
// in its bay as in stage A.
#[test]
fn a_drive_through_truck_whose_plan_goes_replans_from_the_exit() {
    for stale in [false, true] {
        let (mut yard, way) = yard_through(false, false);
        let carrier = yard.dispatch(0);
        yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
        assert!(yard.check(0));
        let dock = yard.dock(0);
        // Into the U behind the bay.
        for _ in 0..40 {
            yard.tick();
        }
        let a = &yard.agents.agents;
        let stood = Vector2::new(a.pos_x[carrier], a.pos_y[carrier]);
        let attach = (
            a.planned_attach_lane_id[carrier],
            a.planned_attach_lane_d[carrier].to_bits(),
        );
        if stale {
            yard.agents.agents.planned_attach_lane_id[carrier] = u32::MAX - 1;
        } else {
            yard.agents.agents.access_flags[carrier] = 0;
        }
        yard.tick();
        let a = &yard.agents.agents;
        assert_eq!(
            Vector2::new(a.pos_x[carrier], a.pos_y[carrier]),
            stood,
            "it stopped"
        );
        assert!(yard.holds(carrier, dock));
        assert_eq!(
            (
                a.planned_attach_lane_id[carrier],
                a.planned_attach_lane_d[carrier].to_bits()
            ),
            attach,
            "planned again from the exit: stale {stale}"
        );
        let line = way_out(&yard, carrier, &way);
        let (progress, off) = along(&line, stood);
        assert!(off < 1e-3 && progress > 8.0, "it stood on its way out");
        drive_along(&mut yard, carrier, ParkingTripPhase::PullOut, dock, &line);
        assert!(
            !yard
                .agents
                .holds_freight_dock(yard.agents.handles.handle(carrier), dock)
        );
        let lines = &yard.agents.trip_failures.lines;
        assert!(
            lines
                .iter()
                .all(|l| !l.contains("sent home") && !l.contains("sent to border")),
            "{lines:#?}"
        );
    }

    let (mut yard, way) = yard_through(false, false);
    let carrier = yard.dispatch(0);
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::PullIn);
    yard.tick();
    yard.agents.agents.access_flags[carrier] = 0;
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
    let stop = way.route_in.as_slice()[3];
    let a = &yard.agents.agents;
    assert_eq!(
        [a.pos_x[carrier], a.pos_y[carrier]],
        stop,
        "it ended in its bay"
    );
}

// With no road from the exit, a truck on its way out stops where it is and waits on the parking
// retry wheel, holding its dock, never moved; once the road is back it plans from the exit and
// leaves forward.
#[test]
fn a_drive_through_truck_with_no_way_out_waits_where_it_stopped() {
    let (mut yard, way) = yard_through(false, false);
    let carrier = yard.dispatch(0);
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
    assert!(yard.check(0));
    let dock = yard.dock(0);
    for _ in 0..40 {
        yard.tick();
    }
    let handle = yard.handle(carrier);
    let at = |y: &Yard| {
        Vector2::new(
            y.agents.agents.pos_x[carrier],
            y.agents.agents.pos_y[carrier],
        )
    };
    let stood = at(&yard);
    let door = yard.allocator.entrances[0].clone();
    yard.allocator.entrances[0].edge_idx = usize::MAX;
    yard.agents.agents.access_flags[carrier] = 0;
    yard.tick();
    assert_eq!(
        std::mem::take(&mut yard.agents.parking_retries),
        [(handle, 5)]
    );
    for _ in 0..300 {
        yard.tick();
        assert_eq!(at(&yard), stood, "it waits where it stopped");
        assert!(yard.holds(carrier, dock));
    }
    yard.allocator.entrances[0] = door;
    let i = yard.agents.resolve_agent(handle).unwrap();
    yard.agents.agents.parking_trip[i].retry_pending = false;
    yard.agents.agents.next_replan_time[i] = 0.0;
    yard.agents.wake_parking_agent(handle);
    yard.tick();
    assert!(
        (attach_point(&yard, carrier).x - EXIT_X).abs() < 0.5,
        "planned from the exit"
    );
    let line = way_out(&yard, carrier, &way);
    drive_along(&mut yard, carrier, ParkingTripPhase::PullOut, dock, &line);
    yard.tick_until(|y| y.at_kerb(carrier, 0));
}

// A removed truck holds its drive-through dock no more, in every phase that holds one: on the
// way in, standing, and on the way out after a delivery or a dispatch.
#[test]
fn a_removed_drive_through_truck_frees_its_dock_in_every_phase() {
    use ParkingTripPhase::{AtDock, PullIn, PullOut};
    for (phase, spawned) in [
        (PullIn, false),
        (AtDock, false),
        (PullOut, false),
        (PullOut, true),
    ] {
        let (mut yard, _) = yard_through(true, false);
        let carrier = yard.dispatch(0);
        let (building, dock) = if spawned {
            (0, yard.dock_of(0, 0))
        } else {
            yard.tick_until(|y| y.phase(carrier) == if phase == PullOut { AtDock } else { phase });
            if phase == PullOut {
                assert!(yard.check(0));
                for _ in 0..30 {
                    yard.tick();
                }
            }
            (1, yard.dock(0))
        };
        assert_eq!(yard.phase(carrier), phase);
        assert!(yard.holds(carrier, dock));
        let handle = yard.allocator.building_handles.handle(building).to_bits();
        assert_eq!(yard.agents.first_free_dock(handle, &yard.parking), None);
        yard.agents.remove_freight_carrier(carrier, 0);
        assert_eq!(
            yard.agents.first_free_dock(handle, &yard.parking),
            Some(dock),
            "{phase:?}"
        );
    }
}

// A stage A dock in the same city behaves as before: the truck spawned in the source's stage A
// dock backs out along its route reversed to the gate it came in by, then drives through the
// grocery's drive-through dock.
#[test]
fn a_stage_a_dock_beside_a_drive_through_backs_out_as_before() {
    let (mut yard, way) = yard_through(false, true);
    let carrier = yard.dispatch(0);
    let home = yard.dock_of(0, 0);
    assert!(!yard.parking.dock_leaves_forward(home));
    assert!((attach_point(&yard, carrier).x - (SOURCE_X + GATE_OFFSET)).abs() < 0.5);
    pull_out(&mut yard, carrier, home);
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::PullIn);
    let line = way_in(&yard, carrier, &way);
    let dock = yard.dock(0);
    drive_along(&mut yard, carrier, ParkingTripPhase::PullIn, dock, &line);
    assert_eq!(yard.phase(carrier), ParkingTripPhase::AtDock);
}

// Place `k` of the line on the way in of `way`'s straight way in (`PARK-36`): `k` spacings back
// from the stop towards the gate.
fn way_in_place(way: &DockWay, k: usize) -> Vector2 {
    let line = points(&way.route_in);
    let stop = line[line.len() - 1];
    stop + (line[0] - stop).normalized() * (k as f32 * crate::config::TRUCK_BODY_LENGTH_M + k as f32)
}

// The truck now carrying shipment `id` (removals swap agents).
fn carrier(yard: &Yard, id: u64) -> usize {
    let idx = yard.shipments.shipment_index(id).unwrap();
    yard.agents.resolve_agent(yard.shipments.shipments[idx].carrier).unwrap()
}

// A yard whose drive-through way in holds three trucks behind the bay (36 m from the gate to the
// stop), shipment 0's truck standing in the bay and shipments 1 to 3 dispatched one after
// another, each once the last reached its place: all three wait on the way in, never at the kerb.
fn way_in_full() -> (Yard, DockWay) {
    let mut yard = yard_with(0, 0);
    let way = publish_drive_through_deep(&mut yard, 1, 36.0);
    let dock = yard.dock(0);
    let first = yard.dispatch(0);
    yard.tick_until(|y| y.phase(first) == ParkingTripPhase::AtDock);
    for id in 1..=3 {
        let i = yard.dispatch(id);
        yard.tick_until(|y| {
            assert_ne!(y.phase(i), ParkingTripPhase::ToKerb, "truck {id} is sent to the kerb");
            y.phase(i) == ParkingTripPhase::AtKerb
        });
        let trip = yard.agents.agents.parking_trip[i];
        assert_eq!(trip.destination, dock, "truck {id} waits on the way in");
        assert!(trip.ready && trip.freight_gate && !yard.holds(i, dock));
        let place = way_in_place(&way, id as usize);
        yard.tick_until(|y| at(y, i).distance_to(place) < 1e-3);
    }
    (yard, way)
}

// Trucks for a drive-through yard whose bay is held wait on its way in, off the road, one behind
// another in the order they reached the gate, 9 m apart, and move up as each before them pulls
// on into the bay; a truck arriving with the way in full waits at the kerb stop instead. The
// dock goes to each in arrival order, the kerb's truck last.
#[test]
fn trucks_wait_on_the_way_in_in_arrival_order_and_the_kerb_takes_the_rest() {
    let (mut yard, way) = way_in_full();
    let dock = yard.dock(0);
    let waiting = [1, 2, 3].map(|id| carrier(&yard, id));
    for (k, &i) in waiting.iter().enumerate() {
        let a = &yard.agents.agents;
        assert_eq!(a.current_lane_id[i], usize::MAX, "out of the driving lane");
        assert!(crate::simulation::economy::agents::transit_is_visible(a.transit[i]));
        assert!(at(&yard, i).distance_to(way_in_place(&way, k + 1)) < 1e-3);
    }
    // The way in is full: the fifth waits in the line at the kerb before the gate (`PARK-36`).
    let fifth = yard.dispatch(4);
    yard.tick_until(|y| {
        y.phase(fifth) == ParkingTripPhase::AtKerb
            && at(y, fifth).distance_to(line_place(y, 1, 0)) < 0.05
    });
    assert!(yard.agents.agents.parking_trip[fifth].destination.is_none());
    // The first delivers and leaves by the exit; the dock goes to each in turn.
    let first = carrier(&yard, 0);
    let mut order = Vec::new();
    for (id, i) in [(0, first), (1, waiting[0]), (2, waiting[1]), (3, waiting[2]), (4, fifth)] {
        yard.tick_until(|y| {
            for &j in &[waiting[0], waiting[1], waiting[2], fifth] {
                assert!(!y.inside_grocery(j), "no truck takes the door");
                if y.phase(j) == ParkingTripPhase::PullIn && !order.contains(&j) {
                    order.push(j);
                }
            }
            y.phase(i) == ParkingTripPhase::AtDock
        });
        assert!(yard.holds(i, dock));
        assert!(yard.check(id));
    }
    assert_eq!(order, [waiting[0], waiting[1], waiting[2], fifth]);
}

// A truck in the middle of the line on the way in whose shipment's leg runs out leaves the line as
// a truck waiting at the kerb does: the check expires the shipment, refunds the buyer and removes
// the truck where it stands. Nobody waits on it: the one behind moves up into its place, and the
// dock goes on in arrival order.
#[test]
fn a_truck_timed_out_on_the_way_in_leaves_the_line_and_the_rest_move_up() {
    let (mut yard, way) = way_in_full();
    let budget = yard.allocator.buildings[1].operating_budget;
    yard.shipments.hour_ticks = 1_000;
    assert!(!yard.check(2));
    yard.shipments.hour_ticks = 0;
    assert_eq!(yard.status(2), ShipmentStatus::Expired);
    assert_eq!(yard.allocator.buildings[1].operating_budget, budget + 600.0, "the buyer is refunded");
    let idx = yard.shipments.shipment_index(2).unwrap();
    assert!(yard.agents.resolve_agent(yard.shipments.shipments[idx].carrier).is_none());
    let third = carrier(&yard, 3);
    yard.tick_until(|y| at(y, third).distance_to(way_in_place(&way, 2)) < 1e-3);
    let mut order = Vec::new();
    for id in [0, 1, 3] {
        yard.tick_until(|y| {
            for other in [1, 3] {
                let j = carrier(y, other);
                if y.phase(j) == ParkingTripPhase::PullIn && !order.contains(&other) {
                    order.push(other);
                }
            }
            y.phase(carrier(y, id)) == ParkingTripPhase::AtDock
        });
        assert!(yard.check(id));
    }
    assert_eq!(order, [1, 3]);
}

// Two trucks dispatched from one building without a dock at the same moment start one place apart
// along its kerb, never one over the other, and both pull out and leave (`PARK-36`).
#[test]
fn trucks_dispatched_together_start_one_place_apart_at_the_kerb() {
    let mut yard = yard(0);
    let first = yard.dispatch(0);
    let second = yard.dispatch(1);
    assert!(at(&yard, first).distance_to(kerb_stop(&yard, 0)) < 0.05);
    let place = crate::simulation::economy::agents::tick::kerb_queue_entrance(
        &yard.allocator.entrances[0],
        &yard.graph,
        1,
    )
    .unwrap()
    .door_pos;
    assert!(at(&yard, second).distance_to(place) < 0.05, "{:?} {place:?}", at(&yard, second));
    yard.tick_until(|y| {
        [first, second]
            .iter()
            .all(|&i| y.agents.agents.transit[i] == crate::simulation::economy::agents::TRANSIT_NETWORK)
    });
    // A dispatch at a later moment starts at the kerb stop again.
    let third = yard.dispatch(2);
    assert!(at(&yard, third).distance_to(kerb_stop(&yard, 0)) < 0.05);
}

// Two trucks dispatched together arrive at a drive-through yard one behind the other: the first
// takes the free bay and pulls in at the yard speed, the second waits on the way in and follows
// it up, never nearer than a body and 1 m behind it, until it stands at the first place (`PARK-36`).
#[test]
fn a_truck_on_the_way_in_holds_short_of_the_truck_ahead() {
    let mut yard = yard_with(0, 0);
    let way = publish_drive_through_deep(&mut yard, 1, 36.0);
    let trucks = [yard.dispatch(0), yard.dispatch(1)];
    yard.tick_until(|y| trucks.iter().any(|&i| y.phase(i) == ParkingTripPhase::AtKerb));
    let [lead, follow] = if yard.phase(trucks[1]) == ParkingTripPhase::AtKerb { trucks } else { [trucks[1], trucks[0]] };
    assert_eq!(yard.phase(lead), ParkingTripPhase::PullIn, "the first pulls in");
    let line = way_in(&yard, follow, &way);
    let start = along(&line, at(&yard, follow)).0;
    let mut closest = f32::INFINITY;
    yard.tick_until(|y| {
        let (a, off_a) = along(&line, at(y, lead));
        let (b, off_b) = along(&line, at(y, follow));
        if off_a < 0.05 && off_b < 0.05 && b > start + 1e-3 {
            closest = closest.min(a - b);
        }
        y.phase(lead) == ParkingTripPhase::AtDock
            && at(y, follow).distance_to(way_in_place(&way, 1)) < 1e-3
    });
    assert!(closest.is_finite(), "it moved up behind the first");
    assert!(closest >= 9.0 - 1e-3, "{closest} m behind the truck ahead");
}
