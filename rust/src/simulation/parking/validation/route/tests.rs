// SPDX-License-Identifier: GPL-3.0-or-later

//! The drive-through proof (`PARK-21`) on a yard paved 60 m square: a loading bay at the origin
//! facing +z (entry edge z = 0, stop (0, 4)), an arrival gate at x = 0 and an exit gate at x = 14
//! on the street edge z = -20, both facing +z into the lot.

use super::super::super::geometry::{overlaps, precise};
use super::*;

const FORWARD: Vector2 = Vector2::new(0.0, 1.0);

fn gate(x: f32) -> [Vector2; 2] {
    let at = Vector2::new(x, -20.0);
    [
        at - FORWARD,
        at + FORWARD * ProofClass::Freight.proof().lead,
    ]
}

fn prove(way: &DockWayInput, wall: [Vector2; 4]) -> Result<ProvenDock, Rejection> {
    let dock = AnchorBay {
        pos: Vector2::new(0.0, 4.0),
        forward: FORWARD,
        width: 3.5,
        length: 8.0,
    };
    let footprints = [rectangle(dock.pos, FORWARD, dock.width, dock.length)];
    let yard = precise(&rectangle(Vector2::new(7.0, 0.0), FORWARD, 60.0, 60.0));
    validate_dock(
        0,
        &[dock],
        &footprints,
        &[],
        &[yard],
        true,
        &[gate(0.0), gate(14.0)],
        None,
        way,
        |foot| overlaps(foot, &wall),
    )
}

fn nowhere() -> [Vector2; 4] {
    rectangle(Vector2::new(100.0, 100.0), FORWARD, 1.0, 1.0)
}

// Out of the bay to z = 12, a half circle of radius 7 about (7, 12) in `chords` chords, and
// back down to the exit gate at x = 14.
fn half_circle(chords: usize) -> Vec<Vector2> {
    (0..=chords)
        .map(|k| {
            let angle = std::f32::consts::PI * (1.0 - k as f32 / chords as f32);
            Vector2::new(7.0 + 7.0 * angle.cos(), 12.0 + 7.0 * angle.sin())
        })
        .collect()
}

fn reason(result: Result<ProvenDock, Rejection>) -> String {
    match result {
        Ok(_) => panic!("the dock proved"),
        Err(rejection) => rejection.reason,
    }
}

#[test]
fn a_half_circle_out_to_the_exit_gate_proves_and_names_both_gates() {
    let way = DockWayInput {
        route_out: half_circle(13),
        exit: Some(Ok(1)),
        ..DockWayInput::default()
    };
    let dock = prove(&way, nowhere()).unwrap_or_else(|r| panic!("{}", r.reason));
    assert_eq!((dock.arrive, dock.leave), (0, 1));
    // The derived way in, then out: the stop, 14 arc points, the exit's approach and connection.
    assert_eq!(dock.route_in.len(), 4);
    assert_eq!(
        dock.path,
        [
            dock.route_in[0],
            dock.route_in[1],
            dock.route_in[2],
            dock.route_in[3]
        ]
    );
    assert_eq!(dock.route_out.len(), 17);
    assert_eq!(dock.route_out[0], Vector2::new(0.0, 4.0));
    assert_eq!(dock.route_out[16], gate(14.0)[0]);
    // Without the exit the same bay is a stage A dock: no ways, and it leaves by its gate.
    let stage_a =
        prove(&DockWayInput::default(), nowhere()).unwrap_or_else(|r| panic!("{}", r.reason));
    assert!(stage_a.route_in.is_empty() && stage_a.route_out.is_empty());
    assert_eq!(
        (stage_a.arrive, stage_a.leave, stage_a.path),
        (0, 0, dock.path)
    );
}

#[test]
fn a_sharp_bend_needs_its_envelope_and_says_where() {
    // Out to (0, 14), then a right angle across to (14, 14): a 90° bend at waypoint 1.
    let way = DockWayInput {
        route_out: vec![Vector2::new(0.0, 14.0), Vector2::new(14.0, 14.0)],
        exit: Some(Ok(1)),
        ..DockWayInput::default()
    };
    assert!(prove(&way, nowhere()).is_ok(), "an open yard has the room");
    // A post inside the 8.45 m square at (0, 14), clear of both corridors (2.6 m wide).
    let post = rectangle(Vector2::new(-3.5, 18.0), FORWARD, 0.5, 0.5);
    let rejection = prove(&way, post).err().expect("the bend has no room");
    assert_eq!(
        rejection.reason,
        "no room to drive out: the bend at waypoint 1 of the way out hits the building or another structure (a truck needs about 8.5 m by 8.5 m of clear paving to change direction by more than 15°: bend by at most that much at each point, as chords of an arc, or give it the room)"
    );
    let shape = rejection.shape.expect("the failing square");
    assert!(
        (shape[0].distance_to(shape[1]) - 8.45).abs() < 1e-4,
        "{shape:?}"
    );
}

#[test]
fn a_route_that_touches_itself_or_meets_the_bay_askew_is_refused() {
    let crossing = DockWayInput {
        route_in: [
            (0.0, -8.0),
            (6.0, -8.0),
            (6.0, -12.0),
            (-3.0, -12.0),
            (0.0, -2.0),
        ]
        .map(|(x, z)| Vector2::new(x, z))
        .to_vec(),
        ..DockWayInput::default()
    };
    assert_eq!(
        reason(prove(&crossing, nowhere())),
        "the way in touches itself: the way in from waypoint 3 to waypoint 4 comes within 0.1 m of the way in from the Driveway's approach to waypoint 1 (a way may not cross or touch itself; the way in and the way out may cross each other)"
    );
    let askew = DockWayInput {
        route_in: vec![Vector2::new(3.0, -4.0)],
        ..DockWayInput::default()
    };
    assert_eq!(
        reason(prove(&askew, nowhere())),
        "the way into the spot from waypoint 1 runs 21° off the spot's direction: a truck drives into its spot along it, so put the last waypoint in line behind the spot (within 15°)"
    );
    let backwards = DockWayInput {
        route_out: vec![Vector2::new(0.0, -6.0)],
        exit: Some(Ok(1)),
        ..DockWayInput::default()
    };
    assert_eq!(
        reason(prove(&backwards, nowhere())),
        "the way out from the spot to waypoint 1 runs 180° off the spot's direction: a truck leaves its spot driving forward, so put the first waypoint in line ahead of the spot (within 15°)"
    );
    let too_many = DockWayInput {
        route_in: (0..22)
            .map(|k| Vector2::new(0.0, -15.0 + 0.5 * k as f32))
            .collect(),
        ..DockWayInput::default()
    };
    assert_eq!(
        reason(prove(&too_many, nowhere())),
        "the way in has 25 points: at most 24, its Driveway's road connection and approach and the spot with at most 21 waypoints"
    );
    let narrow_exit = DockWayInput {
        exit: Some(Err("narrow".into())),
        ..DockWayInput::default()
    };
    assert_eq!(
        reason(prove(&narrow_exit, nowhere())),
        "its exit 'narrow' is not a usable gate: name a freight Driveway at least 4 m wide on the street edge, pointing into the lot"
    );
}

#[test]
fn every_dock_of_a_building_arrives_and_leaves_by_the_first_docks_gates() {
    // Dock 0 leaves by gate 1; dock 1 leaves by its arrival gate; dock 2 by gate 1 again.
    let leaves = [1, 0, 1];
    let proven = dock_ways(3, |slot, only| {
        let arrive = only.unwrap_or(0);
        Ok(ProvenDock {
            path: [Vector2::ZERO; 4],
            route_in: Vec::new(),
            route_out: Vec::new(),
            arrive,
            leave: if leaves[slot] == 1 { 1 } else { arrive },
        })
    });
    assert!(proven[0].is_ok() && proven[2].is_ok());
    assert_eq!(
        proven[1].as_ref().err().map(|r| r.reason.as_str()),
        Some(
            "its exit differs from the building's first loading bay's: every loading bay of a building leaves by the same Driveway"
        )
    );
}

// A car place's own route (`PARK-26`) on the same yard: a 2.5 by 5 m place at (10, 10) facing
// +z (entry edge z = 7.5, stop (10, 10)), reached from the car gate at x = 0 (approach z -17.5)
// along `route_in`, with one wall box.
fn car(route_in: &[Vector2], wall: [Vector2; 4]) -> Result<(Vec<Vector2>, usize), Rejection> {
    let place = AnchorBay {
        pos: Vector2::new(10.0, 10.0),
        forward: FORWARD,
        width: 2.5,
        length: 5.0,
    };
    let footprints = [rectangle(place.pos, FORWARD, place.width, place.length)];
    let yard = precise(&rectangle(Vector2::new(7.0, 0.0), FORWARD, 60.0, 60.0));
    let at = Vector2::new(0.0, -20.0);
    let car = [at - FORWARD, at + FORWARD * ProofClass::Car.proof().lead];
    validate_car_route(
        0,
        &[place],
        &footprints,
        &[yard],
        true,
        &[car],
        &[FORWARD],
        route_in,
        |foot| overlaps(foot, &wall),
    )
}

fn car_reason(result: Result<(Vec<Vector2>, usize), Rejection>) -> String {
    match result {
        Ok(_) => panic!("the route proved"),
        Err(rejection) => rejection.reason,
    }
}

const HINT: &str = ". This spot follows its own route: move its waypoints, or choose Back to automatic";

// Up the gate line, across, and up into the place: two bends of 90°, each with its 4.85 m square;
// the row keeps the route as its four points and no long way; a third waypoint makes a long way.
#[test]
fn a_cars_own_route_with_two_bends_proves_through_its_driveway() {
    let two = [Vector2::new(0.0, 2.0), Vector2::new(10.0, 2.0)];
    let (route, gate) = car(&two, nowhere()).unwrap_or_else(|r| panic!("{}", r.reason));
    assert_eq!(gate, 0);
    assert_eq!(&route[1..], &[two[0], two[1], Vector2::new(10.0, 10.0)]);
    let (path, way) = car_row_path(&route);
    assert_eq!((path.as_slice(), way), (route.as_slice(), None));
    let three = [Vector2::new(0.0, 2.0), Vector2::new(5.0, 2.2), Vector2::new(10.0, 2.0)];
    let (route, _) = car(&three, nowhere()).unwrap_or_else(|r| panic!("{}", r.reason));
    let (path, way) = car_row_path(&route);
    assert_eq!(way, Some(route.as_slice()));
    assert_eq!(path, [route[0], route[1], route[3], route[4]]);
    // One waypoint: the leg into the spot split at its middle, the same line.
    let (route, _) = car(&[Vector2::new(10.0, 0.0)], nowhere()).unwrap_or_else(|r| panic!("{}", r.reason));
    let (path, way) = car_row_path(&route);
    assert_eq!((path[2], way), (Vector2::new(10.0, 5.0), None));
}

// A post inside the first bend's square: rejected naming the bend, with the car's room and the
// hint, and no other route is tried.
#[test]
fn a_cars_bend_needs_its_square_and_says_where() {
    let two = [Vector2::new(0.0, 2.0), Vector2::new(10.0, 2.0)];
    let post = rectangle(Vector2::new(-2.0, 0.5), FORWARD, 0.2, 0.2);
    assert_eq!(
        car_reason(car(&two, post)),
        format!("no room to drive in: the bend at waypoint 1 hits the building or another structure (a car needs about 5 m by 5 m of clear paving to change direction by more than 30°: bend by at most that much at each point, as chords of an arc, or give it the room){HINT}")
    );
    let rejection = car(&two, post).err().unwrap();
    assert_eq!(rejection.route.map(|r| r.len()), Some(4), "the route it was tried along");
}

// The ends: a last leg askew, a first waypoint behind the Driveway or off its line, a way that
// touches itself.
#[test]
fn a_cars_own_route_starts_ahead_of_a_driveway_and_ends_along_its_spot() {
    let askew = [Vector2::new(0.0, 2.0)];
    assert_eq!(
        car_reason(car(&askew, nowhere())),
        format!("the way into the spot from waypoint 1 runs 51° off the spot's direction: a car drives into its spot along it, so put the last waypoint in line behind the spot (within 30°){HINT}")
    );
    let start = format!("its own route starts at no Driveway: put waypoint 1 inside the lot straight ahead of a car Driveway that points into the lot (within 30° of the Driveway's direction){HINT}");
    // Behind the Driveway's mouth (z -20), and 45° off its line.
    assert_eq!(car_reason(car(&[Vector2::new(0.0, -20.5), Vector2::new(10.0, 2.0)], nowhere())), start);
    assert_eq!(car_reason(car(&[Vector2::new(5.0, -16.0), Vector2::new(10.0, 2.0)], nowhere())), start);
    let rejection = car(&[Vector2::new(5.0, -16.0), Vector2::new(10.0, 2.0)], nowhere()).err().unwrap();
    assert_eq!(rejection.leg, Some([Vector2::new(0.0, -21.0), Vector2::new(5.0, -16.0)]));
    let back = [Vector2::new(0.0, 2.0), Vector2::new(10.0, 2.0), Vector2::new(10.0, -10.0), Vector2::new(10.0, 0.0)];
    assert!(car_reason(car(&back, nowhere())).starts_with("the way in touches itself: the way into the spot from waypoint 4 comes within 0.1 m of the way in from waypoint 1 to waypoint 2 (a way may not cross or touch itself)."));
}
