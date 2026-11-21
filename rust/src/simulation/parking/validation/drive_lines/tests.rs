// SPDX-License-Identifier: GPL-3.0-or-later

//! Drive-line routes (`PARK-24`) on whole lots, proven as the editor proves them: the street at
//! +z, every Driveway on the street edge pointing in, its road connection 5 cm into the street.

use super::super::super::geometry::{overlaps, precise, rectangle};
use super::super::{AnchorBay, ProofClass, validate_bay};
use super::DriveLines;
use godot::prelude::Vector2;

const UP: Vector2 = Vector2::new(0.0, 1.0);
const DOWN: Vector2 = Vector2::new(0.0, -1.0);

fn v(x: f32, z: f32) -> Vector2 {
    Vector2::new(x, z)
}

// A box from its two corners.
fn rect(a: Vector2, b: Vector2) -> Vec<Vector2> {
    vec![a, v(b.x, a.y), b, v(a.x, b.y)]
}

// A lot in its own frame: half width and depth, the street beyond z = half.y.
#[derive(Clone)]
struct Lot {
    half: Vector2,
    paving: Vec<Vec<Vector2>>,
    walls: Vec<Vec<Vector2>>,
    // Driveway positions on the street edge, each pointing -z.
    gates: Vec<Vector2>,
    // Parking places, 2.5 m by 5 m: entry edge centre and forward.
    places: Vec<(Vector2, Vector2)>,
}

// A route proven: its four points and its gate.
type Proven = Result<([Vector2; 4], Option<usize>), String>;

impl Lot {
    // Every place's proof, the lot turned by `angle` radians, routed along the drive lines or
    // pinned to the straight path.
    fn prove(&self, angle: f32, routes: bool) -> Vec<Proven> {
        let (sin, cos) = angle.sin_cos();
        let turn = |p: Vector2| v(p.x * cos - p.y * sin, p.x * sin + p.y * cos);
        let bays: Vec<_> = self
            .places
            .iter()
            .map(|&(entry, forward)| AnchorBay {
                pos: turn(entry + forward * 2.5),
                forward: turn(forward),
                width: 2.5,
                length: 5.0,
            })
            .collect();
        let footprints: Vec<_> = bays
            .iter()
            .map(|b| rectangle(b.pos, b.forward, b.width, b.length))
            .collect();
        // The paving and the open street beyond the street edge.
        let street = rect(
            v(-self.half.x - 8.0, self.half.y),
            v(self.half.x + 8.0, self.half.y + 8.0),
        );
        let support: Vec<_> = self
            .paving
            .iter()
            .chain([&street])
            .map(|p| precise(&p.iter().copied().map(turn).collect::<Vec<_>>()))
            .collect();
        let walls: Vec<Vec<Vector2>> = self
            .walls
            .iter()
            .map(|w| w.iter().copied().map(turn).collect())
            .collect();
        let paths: Vec<_> = self
            .gates
            .iter()
            .map(|&g| [turn(v(g.x, self.half.y + 0.05)), turn(g + DOWN * 2.5)])
            .collect();
        let inward = vec![turn(DOWN); self.gates.len()];
        let lines = routes.then_some(DriveLines {
            inward: &inward,
            axes: [turn(v(1.0, 0.0)), turn(UP)],
        });
        (0..bays.len())
            .map(|slot| {
                validate_bay(
                    ProofClass::Car,
                    slot,
                    &bays,
                    &footprints,
                    &[],
                    &support,
                    !self.gates.is_empty(),
                    &paths,
                    None,
                    lines,
                    |foot| walls.iter().any(|w| overlaps(foot, w)),
                )
                .map_err(|r| r.reason)
            })
            .collect()
    }

    fn valid(&self, routes: bool) -> Vec<bool> {
        self.prove(0.0, routes).iter().map(Result::is_ok).collect()
    }
}

// The owner's grocery (`PARK-24`): a 40 m lot paved to the street, the shop across the back,
// one 6 m gate at the left, a row of eight places nose to the shop front (entries at z 8.05) and
// six places right of the gate nose to the street (entries at z 15): a 6.95 m aisle between.
fn grocery() -> Lot {
    let front = (0..8).map(|k| (v(-7.5 + 2.5 * k as f32, 8.05), DOWN));
    let street = (0..6).map(|k| (v(-4.0 + 2.5 * k as f32, 15.0), UP));
    Lot {
        half: v(20.0, 20.0),
        paving: vec![rect(v(-19.5, -18.35), v(19.5, 20.0))],
        walls: vec![rect(v(-16.0, -18.0), v(18.0, 2.9))],
        gates: vec![v(-10.75, 20.0)],
        places: front.chain(street).collect(),
    }
}

// The rejection of a place no route reaches, from the straight path's rejection on a pinned lot:
// that path's failure, then the drive lines' (`PARK-24`).
fn no_way(straight: &str) -> String {
    let failure = straight
        .strip_prefix("no room to drive in: ")
        .and_then(|rest| rest.split(" (").next())
        .unwrap();
    format!(
        "no way to drive to this spot: straight from the Driveway, {failure}, and no way along the row or straight down from the Driveway is clear (a car needs a 1.8 m strip of clear paving to drive on and about 5 m by 5 m of it where it turns): clear the way or move the spot"
    )
}

fn assert_points(path: [Vector2; 4], expected: [Vector2; 3]) {
    for (p, e) in path[1..].iter().zip(expected) {
        assert!((*p - e).length() < 1e-4, "{path:?} vs {expected:?}");
    }
}

#[test]
fn the_grocery_reaches_both_rows_from_one_gate_along_the_aisle() {
    let lot = grocery();
    let pinned = lot.prove(0.0, false);
    let routed = lot.prove(0.0, true);
    // Straight from the gate the far front places cross the street row, and the far street
    // places cross their neighbours.
    let rejected: Vec<_> = (0..14).filter(|&i| pinned[i].is_err()).collect();
    assert!(rejected.len() >= 6, "{rejected:?}");
    assert!(routed.iter().all(Result::is_ok), "{routed:?}");
    // Every place the straight path proves keeps it bit for bit.
    for (before, after) in pinned.iter().zip(&routed) {
        if let Ok((path, gate)) = before {
            let (again, same_gate) = after.as_ref().unwrap();
            assert_eq!(
                path.map(|p| [p.x.to_bits(), p.y.to_bits()]),
                again.map(|p| [p.x.to_bits(), p.y.to_bits()])
            );
            assert_eq!(gate, same_gate);
        }
    }
    // The far front place: down the gate line to the front row's line (2.5 m before its entry
    // edges), along it, and in.
    let (far_front, gate) = routed[7].clone().unwrap();
    assert_eq!(gate, Some(0));
    assert_points(far_front, [v(-10.75, 10.55), v(10.0, 10.55), v(10.0, 5.55)]);
    assert_eq!(far_front[0], v(-10.75, 20.05));
    // The far street place along its own row's line, 2.5 m before the street row.
    let (far_street, _) = routed[13].clone().unwrap();
    assert_points(far_street, [v(-10.75, 12.5), v(8.5, 12.5), v(8.5, 17.5)]);
}

#[test]
fn the_grocery_proves_the_same_places_at_eight_angles() {
    let lot = grocery();
    let level: Vec<_> = lot.prove(0.0, true).iter().map(Result::is_ok).collect();
    for k in 1..8 {
        let angle = k as f32 * std::f32::consts::FRAC_PI_4;
        let turned: Vec<_> = lot.prove(angle, true).iter().map(Result::is_ok).collect();
        assert_eq!(turned, level, "turned by {k} × 45°");
    }
}

// A detached house on a 20 m lot: the house at the right, a 3 m drive down the left edge and a
// 10 m by 9 m pad at its end, with two places side by side nose to the house's side.
fn house() -> Lot {
    Lot {
        half: v(10.0, 15.0),
        paving: vec![
            rect(v(-9.0, 5.0), v(-6.0, 15.0)),
            rect(v(-10.0, -4.0), v(0.0, 5.0)),
        ],
        walls: vec![rect(v(0.5, -10.0), v(9.0, 6.0))],
        gates: vec![v(-7.5, 15.0)],
        places: vec![(v(-5.0, 2.0), v(1.0, 0.0)), (v(-5.0, -0.5), v(1.0, 0.0))],
    }
}

#[test]
fn a_side_drive_reaches_two_places_beside_the_house() {
    let lot = house();
    // Straight, the car needs a 5 m square just inside the 3 m drive.
    assert_eq!(lot.valid(false), [false, false]);
    let routed = lot.prove(0.0, true);
    // Down the drive to where it crosses the place's own line, its aisle point, turned there on
    // the pad, and in: no row line crosses the drive, so the turn is candidate 4's.
    let (first, gate) = routed[0].clone().unwrap();
    assert_eq!(gate, Some(0));
    assert_points(first, [v(-7.5, 2.0), v(-5.0, 2.0), v(-2.5, 2.0)]);
    let (second, _) = routed[1].clone().unwrap();
    assert_points(second, [v(-7.5, -0.5), v(-5.0, -0.5), v(-2.5, -0.5)]);
    // A pad a metre narrower leaves no room to turn: rejected, naming the straight way's failure.
    let mut narrow = lot.clone();
    narrow.paving[1] = rect(v(-9.0, -4.0), v(0.0, 5.0));
    assert_eq!(narrow.valid(true), [false, false]);
    for (pinned, routed) in narrow.prove(0.0, false).iter().zip(narrow.prove(0.0, true)) {
        assert_eq!(routed.err(), Some(no_way(pinned.as_ref().unwrap_err())));
    }
}

#[test]
fn a_place_in_line_behind_a_drive_needs_no_square_at_the_gate() {
    let lot = Lot {
        half: v(10.0, 15.0),
        paving: vec![rect(v(-9.0, -5.0), v(-6.0, 15.0))],
        walls: vec![rect(v(-4.0, -10.0), v(9.0, 6.0))],
        gates: vec![v(-7.5, 15.0)],
        places: vec![(v(-7.5, 2.0), DOWN)],
    };
    assert_eq!(lot.valid(false), [false]);
    let (path, _) = lot.prove(0.0, true)[0].clone().unwrap();
    // Two points at thirds of the straight way from the gate to the stop, no bend anywhere.
    assert_points(
        path,
        [
            v(-7.5, 15.0 - 15.5 / 3.0),
            v(-7.5, 15.0 - 31.0 / 3.0),
            v(-7.5, -0.5),
        ],
    );
}

// Negative controls on the grocery: what blocks the aisle keeps the far places rejected, naming
// the straight path's failure.
#[test]
fn a_blocked_row_line_an_unpaved_leg_or_a_structure_keeps_the_rejection() {
    let mut parked = grocery();
    // A place painted across the front row's line, nose to the street, between x 1 and 3.5.
    parked.places.push((v(2.25, 9.0), UP));
    let mut unpaved = grocery();
    // Grass across the aisle from x 0 to 1.
    unpaved.paving = vec![
        rect(v(-19.5, -18.35), v(19.5, 9.0)),
        rect(v(-19.5, 14.0), v(19.5, 20.0)),
        rect(v(-19.5, 9.0), v(0.0, 14.0)),
        rect(v(1.0, 9.0), v(19.5, 14.0)),
    ];
    let mut kiosk = grocery();
    // A kiosk standing on the front row's line.
    kiosk.walls.push(rect(v(4.0, 10.0), v(5.0, 11.0)));
    for (name, lot, reaches) in [
        // Front places left of x 1 reach their line; the street row's line is clear.
        (
            "parked",
            &parked,
            [true, true, true, false, false, false, false, false],
        ),
        // Nothing crosses the grass at x 0 to 1: places right of it stay rejected.
        (
            "unpaved",
            &unpaved,
            [true, true, true, false, false, false, false, false],
        ),
        // The place at x 2.5 turns 2.25 m short of the kiosk's near side and its square reaches
        // past it: from there on rejected.
        (
            "kiosk",
            &kiosk,
            [true, true, true, true, false, false, false, false],
        ),
    ] {
        let pinned = lot.prove(0.0, false);
        let routed = lot.prove(0.0, true);
        let front: Vec<_> = routed[..8].iter().map(Result::is_ok).collect();
        assert_eq!(front, reaches, "{name}: {routed:?}");
        for (before, after) in pinned.iter().zip(&routed) {
            if let Err(reason) = after {
                assert_eq!(reason, &no_way(before.as_ref().unwrap_err()), "{name}");
            }
        }
    }
}

// Two gates the same distance from a place behind a blocked diagonal: the lower gate index wins,
// whichever Driveway it is; moved nearer one gate, that gate wins.
#[test]
fn a_tie_goes_to_the_lower_gate_and_otherwise_the_shorter_route_wins() {
    let lot = |gates: Vec<Vector2>, x: f32| Lot {
        half: v(15.0, 15.0),
        paving: vec![rect(v(-15.0, -15.0), v(15.0, 15.0))],
        walls: vec![
            rect(v(-3.5, 7.0), v(-2.5, 8.0)),
            rect(v(2.5, 7.0), v(3.5, 8.0)),
        ],
        gates,
        places: vec![(v(x, 0.0), DOWN)],
    };
    let (left, right) = (v(-6.0, 15.0), v(6.0, 15.0));
    for (gates, x, gate, turn) in [
        (vec![left, right], 0.0, 0, -6.0),
        (vec![right, left], 0.0, 0, 6.0),
        (vec![left, right], 1.0, 1, 6.0),
    ] {
        let lot = lot(gates, x);
        assert_eq!(lot.valid(false), [false], "the straight paths are blocked");
        for _ in 0..3 {
            let (path, chosen) = lot.prove(0.0, true)[0].clone().unwrap();
            assert_eq!(chosen, Some(gate));
            assert_points(path, [v(turn, 2.5), v(x, 2.5), v(x, -2.5)]);
        }
    }
}
