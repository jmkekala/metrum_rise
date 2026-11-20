// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared asset-local parking proofs; frontage connections are supplied only by the city.

use super::geometry::{covered, overlaps, rectangle};
use godot::prelude::Vector2;

mod editor;
pub(crate) use editor::editor_lot_payload;
#[cfg(test)]
pub(crate) use editor::editor_payload;

#[derive(Clone, Copy)]
pub(super) struct AnchorBay {
    pub pos: Vector2,
    pub forward: Vector2,
    pub width: f32,
    pub length: f32,
}

const SUPPORT_TOLERANCE_M: f32 = 0.05;

pub(super) fn driveway_points_inward(forward: Vector2, outward: Vector2) -> bool {
    forward.dot(-outward) >= 0.9
}

// How far a bay's entry edge may sit from the lot's street edge and still count as reached
// straight from the street.
const STREET_EDGE_TOLERANCE_M: f32 = 0.6;
// Where a car stops on the street before turning straight into a street-edge bay.
const STREET_APPROACH_M: f32 = 2.5;

/// The vehicle classes a lot proves a route for (`PARK-19`): cars to parking bays, freight to
/// loading bays. Indexes the class table and a site's structure sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProofClass {
    /// A resident's car: every `driveway`, every `parking` anchor.
    Car = 0,
    /// A freight carrier: `freight` driveways of at least the row's gate width (gates) and
    /// `loading_bay` anchors only.
    Freight = 1,
}

/// One row of the class table: the body a route is proven for and the room it needs.
pub(crate) struct VehicleProof {
    /// The body's width: every corridor's width.
    pub body_width: f32,
    /// The body's length: a corridor runs half of it past each end of its leg.
    pub body_length: f32,
    /// Side of the square the body needs to change heading at a corner: its diagonal, rounded up.
    pub turn_envelope: f32,
    /// How far inside a gate the approach lies, and how far before a bay's entry edge the aisle.
    pub lead: f32,
    /// A corner whose legs' unit headings have a dot product below this needs an envelope;
    /// through a slighter bend the two corridors, each extended half a body past the corner,
    /// already cover the body's sweep.
    pub straight_through_cos: f32,
    /// Whether the turn off the street always needs an envelope at the approach. A car's does;
    /// a truck's is made on the road through a gate of at least `min_gate` and is not proven in
    /// the lot (owner decision 3).
    pub turn_at_gate: bool,
    /// The smallest bay (width, length).
    pub min_bay: [f32; 2],
    /// The narrowest driveway the class may use.
    pub min_gate: f32,
    /// The clear height a passage needs to open its part for this class.
    pub clear_height: f32,
    // Words for the rejections: the vehicle, the envelope's room and its distance from a lot
    // edge, the missing driveway, a bay in a structure, and the other bays a bay may not meet.
    noun: &'static str,
    turn_room: &'static str,
    edge_room: &'static str,
    no_driveway: &'static str,
    in_structure: &'static str,
    overlaps_bay: &'static str,
    crosses_bay: &'static str,
}

impl ProofClass {
    /// Both classes, in table order.
    pub(crate) const ALL: [Self; 2] = [Self::Car, Self::Freight];

    /// This class's row of the table. O(1).
    pub(crate) const fn proof(self) -> &'static VehicleProof {
        &CLASS_TABLE[self as usize]
    }
}

// The car row is the numbers the car proof used before the table, bit for bit. The freight
// body is a rigid truck (owner decision 1): its envelope is its diagonal, √(2.6² + 8.0²) = 8.41,
// rounded up as 4.85 is for the car; turning 15° about its centre an 8 m body leaves the two
// corridors by under 4 cm, against 14 cm at 30°, hence the tighter bend.
const CLASS_TABLE: [VehicleProof; 2] = [
    VehicleProof {
        body_width: 1.8,
        body_length: 4.5,
        turn_envelope: 4.85,
        lead: 2.5,
        straight_through_cos: 0.866,
        turn_at_gate: true,
        min_bay: [1.8, 4.5],
        min_gate: 1.8,
        clear_height: 2.0,
        noun: "car",
        turn_room: "5 m by 5 m",
        edge_room: "2.5 m",
        no_driveway: "needs a Driveway point, or put the spot's front edge on the street edge pointing into the lot",
        in_structure: "overlaps the building or another structure",
        overlaps_bay: "overlaps another parking spot",
        crosses_bay: "crosses another parking spot",
    },
    VehicleProof {
        body_width: 2.6,
        body_length: 8.0,
        turn_envelope: 8.45,
        lead: 4.25,
        // cos 15°.
        straight_through_cos: 0.965_926,
        turn_at_gate: false,
        min_bay: [3.0, 8.0],
        min_gate: 4.0,
        clear_height: 4.0,
        noun: "truck",
        turn_room: "8.5 m by 8.5 m",
        edge_room: "4.25 m",
        // A dock always needs a gate; freight never uses a car Driveway (owner decision 11).
        no_driveway: "needs a freight Driveway at least 4 m wide: trucks use no car Driveway",
        in_structure: "inside the building: mark the lane as a Passage on its mesh part",
        overlaps_bay: "overlaps a parking spot or another loading bay",
        crosses_bay: "crosses a parking spot or another loading bay",
    },
];

/// Why a bay is rejected, with the local shape that failed when one did, so the editor can
/// draw it.
pub(super) struct Rejection {
    pub reason: String,
    pub shape: Option<[Vector2; 4]>,
}

impl From<&str> for Rejection {
    fn from(reason: &str) -> Self {
        reason.to_owned().into()
    }
}

impl From<String> for Rejection {
    fn from(reason: String) -> Self {
        Self {
            reason,
            shape: None,
        }
    }
}

// Whether the path bends at `corner` enough to need a turning envelope. A zero-length side
// has no heading, so it counts as a turn. O(1).
fn turns_at(before: Vector2, corner: Vector2, after: Vector2, straight_through_cos: f32) -> bool {
    let a = corner - before;
    let b = after - corner;
    a.length_squared() < 1e-8
        || b.length_squared() < 1e-8
        || a.normalized().dot(b.normalized()) < straight_through_cos
}

/// Whether a bay, in asset-local coordinates, opens straight onto the street: its entry edge
/// lies on the lot's frontage edge and it points into the lot. Such a bay needs no Driveway.
/// `half_extents` are the lot's half width (x) and half depth (z). O(1).
pub(super) fn opens_onto_street(
    entry: Vector2,
    forward: Vector2,
    outward: Vector2,
    half_extents: Vector2,
) -> bool {
    let edge = outward.x.abs() * half_extents.x + outward.y.abs() * half_extents.y;
    driveway_points_inward(forward, outward)
        && (entry.dot(outward) - edge).abs() <= STREET_EDGE_TOLERANCE_M
}

// O(bays + local polygon/overlay work) per tested sweep; only authoring/edit boundaries.
// `bays` and `footprints` are the class's own bays (parking spots for cars, loading bays for
// freight) and `others` the footprints of any other bays its routes may not meet (for freight
// the parking spots; none for cars). `street` is the road connection for a bay that opens onto
// the street, if any; `paths` are the class's usable driveways' road connections and approaches.
// Returns the route and the index in `paths` of the driveway it proves through (`None` for a
// street bay).
#[allow(clippy::too_many_arguments)]
pub(super) fn validate_bay(
    class: ProofClass,
    slot: usize,
    bays: &[AnchorBay],
    footprints: &[[Vector2; 4]],
    others: &[[Vector2; 4]],
    support: &[Vec<[f64; 2]>],
    has_driveway: bool,
    paths: &[[Vector2; 2]],
    street: Option<Vector2>,
    collision: impl Fn(&[Vector2]) -> bool,
) -> Result<([Vector2; 4], Option<usize>), Rejection> {
    let row = class.proof();
    let bay = &bays[slot];
    if !has_driveway && street.is_none() {
        return Err(row.no_driveway.into());
    }
    let [min_width, min_length] = row.min_bay;
    if bay.width < min_width || bay.length < min_length || bay.forward.length_squared() < 0.99 {
        return Err(format!(
            "too small for a {}: make it at least {min_width} m wide and {min_length} m long",
            row.noun
        )
        .into());
    }
    if collision(&footprints[slot]) {
        return Err(row.in_structure.into());
    }
    let other_bay = |foot: &[Vector2]| {
        footprints
            .iter()
            .enumerate()
            .any(|(other, b)| other != slot && overlaps(foot, b))
            || others.iter().any(|b| overlaps(foot, b))
    };
    // Authored surfaces stop a few centimetres short of the lot edge they were drawn to, and
    // bays painted side by side share an edge that rounding may push a hair into the next.
    let inset = rectangle(
        bay.pos,
        bay.forward,
        bay.width - SUPPORT_TOLERANCE_M * 2.0,
        bay.length - SUPPORT_TOLERANCE_M * 2.0,
    );
    if other_bay(&inset) {
        return Err(row.overlaps_bay.into());
    }
    if !covered(&inset, support) {
        return Err(
            "not fully on asphalt or concrete: move the whole spot onto a paved yard surface"
                .into(),
        );
    }
    if let Some(connection) = street {
        // The car turns in from the street and drives straight onto the bay.
        let entry = bay.pos - bay.forward * (bay.length * 0.5);
        return Ok((
            [
                connection,
                entry - bay.forward * STREET_APPROACH_M,
                entry,
                bay.pos,
            ],
            None,
        ));
    }
    // What blocks a car's swept shape, if anything.
    let blocked = |foot: &[Vector2]| {
        if !covered(foot, support) {
            Some("leaves the paved yard")
        } else if collision(foot) {
            Some("hits the building or another structure")
        } else if other_bay(foot) {
            Some(row.crosses_bay)
        } else {
            None
        }
    };
    let mut first = None;
    for (gate, &[connection, approach]) in paths.iter().enumerate() {
        let aisle = bay.pos - bay.forward * (bay.length * 0.5 + row.lead);
        let path = [connection, approach, aisle, bay.pos];
        let legs = [
            "the way in from the Driveway",
            "the way to the front of the spot",
            "the way into the spot",
        ];
        let corridors = path.windows(2).zip(legs).filter_map(|(pair, leg)| {
            let delta = pair[1] - pair[0];
            (delta.length_squared() >= 1e-8).then(|| {
                let shape = rectangle(
                    (pair[0] + pair[1]) * 0.5,
                    delta.normalized(),
                    row.body_width,
                    delta.length() + row.body_length,
                );
                (leg, shape)
            })
        });
        // Full turning envelopes prove the orientation change where the path bends: for a car
        // always at the Driveway, where it leaves the street, and elsewhere unless the path is
        // straight through by the class's bend limit.
        let turns = [
            (connection, approach, aisle, "the turn off the Driveway"),
            (approach, aisle, bay.pos, "the turn into the spot"),
        ]
        .into_iter()
        .filter(|&(before, corner, after, _)| {
            (row.turn_at_gate && corner == approach)
                || turns_at(before, corner, after, row.straight_through_cos)
        })
        // Squared to the spot, which lies square to its lot, so a turned lot keeps its envelope.
        .map(|(_, corner, _, part)| {
            let shape = rectangle(corner, bay.forward, row.turn_envelope, row.turn_envelope);
            (part, shape)
        });
        let failure = corridors
            .chain(turns)
            .find_map(|(part, shape)| blocked(&shape).map(|cause| (part, cause, shape)));
        match failure {
            None => return Ok((path, Some(gate))),
            Some(failure) => {
                first.get_or_insert(failure);
            }
        }
    }
    let Some((part, cause, shape)) = first else {
        return Err("the Driveway must sit at the street edge and point into the lot".into());
    };
    let (noun, width, lead) = (row.noun, row.body_width, row.lead);
    let room = match part {
        "the turn off the Driveway" => format!(
            "a {noun} needs about {} of clear paving just inside the Driveway to turn, so keep the Driveway about {} from the lot's side edges",
            row.turn_room, row.edge_room
        ),
        "the turn into the spot" => format!(
            "a {noun} needs about {} of clear paving in front of the spot to turn in",
            row.turn_room
        ),
        "the way to the front of the spot" => format!(
            "a {noun} drives straight from just inside the Driveway to about {lead} m in front of the spot, on a {width} m strip of clear paving reaching half a {noun} length past both ends"
        ),
        _ => format!("the way needs about {width} m of clear paving"),
    };
    Err(Rejection {
        reason: format!("no room to drive in: {part} {cause} ({room})"),
        shape: Some(shape),
    })
}

// A Driveway admits freight by its `vehicle_class` alone (owner decision 11). O(1).
pub(super) fn is_freight_class(class: Option<&str>) -> bool {
    class.map(str::trim) == Some("freight")
}

// A loading bay is a truck's when its class is `freight` or absent. O(1).
pub(super) fn is_dock_class(class: Option<&str>) -> bool {
    class.is_none_or(|class| class.trim() == "freight")
}

/// One arrival gate per building (`PARK-19`): the first dock takes the first gate, in anchor
/// order, that `prove` proves its route through; every later dock must prove through that gate,
/// and one that only another gate proves is rejected as differing. `prove(slot, gates)` is the
/// dock proof of dock `slot` over `gates`. Shared by the placed lot and the asset editor.
/// O(docks × gates) proofs, two per later dock that fails.
pub(super) fn dock_routes(
    docks: usize,
    gates: &[[Vector2; 2]],
    prove: impl Fn(usize, &[[Vector2; 2]]) -> Result<([Vector2; 4], Option<usize>), Rejection>,
) -> Vec<Result<[Vector2; 4], Rejection>> {
    let mut building_gate = None;
    (0..docks)
        .map(|slot| match building_gate {
            None => prove(slot, gates).map(|(path, gate)| {
                building_gate = gate;
                path
            }),
            Some(gate) => prove(slot, &gates[gate..=gate])
                .map(|(path, _)| path)
                .map_err(|rejection| {
                    if prove(slot, gates).is_ok() {
                        "its Driveway differs from the building's first loading bay's".into()
                    } else {
                        rejection
                    }
                }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::geometry::precise;
    use super::*;
    // A loading bay at `x`, its entry edge on z = 0 facing +z, reached from a gate at
    // (0, -20) facing +z, on a yard paved 40 m square, with one wall box. The dock proof as
    // `lot_proof` runs it, without a city: the gate's road connection a metre outside it.
    fn freight_route(x: f32, wall: [Vector2; 4]) -> Result<([Vector2; 4], Option<usize>), Rejection> {
        let forward = Vector2::new(0.0, 1.0);
        let dock = AnchorBay {
            pos: Vector2::new(x, 4.0),
            forward,
            width: 3.5,
            length: 8.0,
        };
        let footprints = [rectangle(dock.pos, forward, dock.width, dock.length)];
        let yard = precise(&rectangle(Vector2::ZERO, forward, 40.0, 50.0));
        let gate = Vector2::new(0.0, -20.0);
        let lead = ProofClass::Freight.proof().lead;
        validate_bay(
            ProofClass::Freight,
            0,
            &[dock],
            &footprints,
            &[],
            &[yard],
            true,
            &[[gate - forward, gate + forward * lead]],
            None,
            |foot| overlaps(foot, &wall),
        )
    }

    #[test]
    fn a_straight_freight_route_needs_no_envelope_and_a_bend_over_15_degrees_does() {
        // Clear of every corridor, inside the 8.45 m square at the approach (0, -15.75).
        let wall = rectangle(Vector2::new(-3.6, -15.5), Vector2::new(0.0, 1.0), 1.2, 5.0);
        let (path, gate) = freight_route(0.0, wall).unwrap_or_else(|r| panic!("{}", r.reason));
        assert_eq!(gate, Some(0));
        assert_eq!(path[1], Vector2::new(0.0, -15.75));
        assert_eq!(path[2], Vector2::new(0.0, -4.25), "the aisle is one truck lead before the bay");
        // 3 m across over 11.5 m bends by 14.6°: still straight through for a truck.
        assert!(freight_route(3.0, wall).is_ok());
        // 4 m across bends by 19.2°: the turn off the Driveway needs its square and hits the wall.
        let rejection = freight_route(4.0, wall).expect_err("a bend needs an envelope");
        assert_eq!(
            rejection.reason,
            "no room to drive in: the turn off the Driveway hits the building or another structure (a truck needs about 8.5 m by 8.5 m of clear paving just inside the Driveway to turn, so keep the Driveway about 4.25 m from the lot's side edges)"
        );
        let shape = rejection.shape.expect("the failing square");
        assert!((shape[0].distance_to(shape[1]) - 8.45).abs() < 1e-4, "{shape:?}");
        assert!((shape[1].distance_to(shape[2]) - 8.45).abs() < 1e-4, "{shape:?}");
    }

    #[test]
    fn a_dock_needs_a_gate_room_for_a_truck_and_a_lane_through_its_building() {
        let forward = Vector2::new(0.0, 1.0);
        let small = AnchorBay {
            pos: Vector2::new(0.0, 4.0),
            forward,
            width: 2.9,
            length: 8.0,
        };
        let footprints = [rectangle(small.pos, forward, small.width, small.length)];
        let prove = |bay: AnchorBay, has_gate, wall: &[Vector2]| {
            validate_bay(
                ProofClass::Freight,
                0,
                &[bay],
                &footprints,
                &[],
                &[],
                has_gate,
                &[],
                None,
                |foot| overlaps(foot, wall),
            )
            .err()
            .map(|r| r.reason)
        };
        let nowhere = rectangle(Vector2::new(100.0, 100.0), forward, 1.0, 1.0);
        assert_eq!(
            prove(AnchorBay { ..small }, false, &nowhere).as_deref(),
            Some("needs a freight Driveway at least 4 m wide: trucks use no car Driveway")
        );
        assert_eq!(
            prove(AnchorBay { ..small }, true, &nowhere).as_deref(),
            Some("too small for a truck: make it at least 3 m wide and 8 m long")
        );
        let bunker = rectangle(Vector2::new(0.0, 4.0), forward, 12.0, 16.0);
        assert_eq!(
            prove(AnchorBay { width: 3.5, ..small }, true, &bunker).as_deref(),
            Some("inside the building: mark the lane as a Passage on its mesh part")
        );
    }
}
