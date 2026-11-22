// SPDX-License-Identifier: GPL-3.0-or-later

//! Where an arriving household steps out of its parked car (`PARK-35`, docs/traffic.md
//! "Arrivals and stored cars").
//!
//! The simulation keeps its own door points per body class: a simulated car has no vehicle asset,
//! and the look the renderer's catalogue picks for it depends on the loaded packs, so it must
//! never decide where a person stands. The car class's doors are the bundled pilot estate's
//! (a test holds them equal); the asset's `[[vehicle.doors]]` are for drawing and tooling.
//!
//! Member 0 (the carrier, the driver) takes the driver's door, member 1 the front passenger door,
//! members 2 and 3 the rear passenger-side and rear driver-side doors. Every further member leaves
//! by a rear side door, round-robin from the passenger side; a vehicle with only front doors lets
//! them out by the front passenger door. A door's later user stands a fixed step toward the rear
//! of its earlier one, so no two members share a spot. Seats are not read: until a household can
//! own a second car (`PARK-33`) every member of a car arrival steps out of the one car. The cargo
//! door is never used. O(doors) per member, no allocation.

use crate::assets::{DoorRow, DoorSide, VehicleDoor};

/// The simulated car's doors in the contract frame (nose at `+Z`, `+X` the driver's left): the
/// bundled pilot estate's (`vehicle.civil.estate_compact`, models commit 0f4757e).
pub(crate) const CAR_DOORS: [VehicleDoor; 5] = [
    door(DoorSide::Left, DoorRow::Front, 0.895, 0.3675, 1.135),
    door(DoorSide::Right, DoorRow::Front, -0.895, 0.3675, 1.135),
    door(DoorSide::Left, DoorRow::Rear, 0.895, -0.58, 0.76),
    door(DoorSide::Right, DoorRow::Rear, -0.895, -0.58, 0.76),
    door(DoorSide::Rear, DoorRow::Cargo, 0.0, -2.24, 1.15),
];

/// How far outside a door's centre a member stands when it appears, metres.
pub(crate) const STANDING_OFFSET_M: f32 = 0.5;
/// How far toward the rear a door's next user stands from its previous one, metres.
pub(crate) const SHARED_DOOR_STEP_M: f32 = 0.6;

const fn door(side: DoorSide, row: DoorRow, x: f32, z: f32, width_m: f32) -> VehicleDoor {
    VehicleDoor {
        side,
        row,
        centre: [x, 0.0, z],
        width_m,
    }
}

/// Where household member `member` (0 the driver) stands when it steps out, `[x, z]` in the
/// contract frame; `None` when `doors` has no driver's door (no door data). O(doors).
pub(crate) fn exit_point(
    doors: &[VehicleDoor],
    member: usize,
    left_hand_drive: bool,
) -> Option<[f32; 2]> {
    let (driver, passenger) = if left_hand_drive {
        (DoorSide::Left, DoorSide::Right)
    } else {
        (DoorSide::Right, DoorSide::Left)
    };
    let find =
        |row: DoorRow, side: DoorSide| doors.iter().position(|d| d.row == row && d.side == side);
    let driver_door = find(DoorRow::Front, driver)?;
    // The first use of each door, in member order, then the doors further members share.
    let first = [
        Some(driver_door),
        find(DoorRow::Front, passenger),
        find(DoorRow::Rear, passenger),
        find(DoorRow::Rear, driver),
    ];
    let rear = [find(DoorRow::Rear, passenger), find(DoorRow::Rear, driver)];
    let shared = if rear.iter().any(Option::is_some) {
        rear
    } else {
        [find(DoorRow::Front, passenger).or(Some(driver_door)), None]
    };
    let firsts = first.iter().flatten().count();
    let (index, uses) = if member < firsts {
        (*first.iter().flatten().nth(member)?, 0)
    } else {
        let extra = member - firsts;
        let count = shared.iter().flatten().count();
        let index = *shared.iter().flatten().nth(extra % count)?;
        (
            index,
            usize::from(first.contains(&Some(index))) + extra / count,
        )
    };
    let d = &doors[index];
    let outward = match d.side {
        DoorSide::Left => [STANDING_OFFSET_M, 0.0],
        DoorSide::Right => [-STANDING_OFFSET_M, 0.0],
        DoorSide::Rear => [0.0, -STANDING_OFFSET_M],
    };
    let back = SHARED_DOOR_STEP_M * uses as f32;
    Some([d.centre[0] + outward[0], d.centre[2] + outward[1] - back])
}

/// `point` (`[x, z]`, contract frame) of a car parked at `position` with `heading`, as the world
/// `[x, z]` under the parked pose. The parked pose turns the model half a turn (its local `-Z`
/// points along travel), then by `heading` about `Y`. O(1).
pub(crate) fn world_point(position: [f32; 3], heading: f32, [x, z]: [f32; 2]) -> [f32; 2] {
    let (s, c) = heading.sin_cos();
    [position[0] - c * x - s * z, position[2] + s * x - c * z]
}

impl super::ParkingSystem {
    /// Where member `member` (0 the driver) of a household whose car stands at `place` steps
    /// out, world `[x, z]`, under the place's saved pose; `None` for a place that is gone. O(1).
    pub(crate) fn car_exit_point(
        &self,
        place: super::Handle<super::Place>,
        member: usize,
        left_hand_drive: bool,
    ) -> Option<[f32; 2]> {
        let p = self.place_handles.resolve(place)?;
        let local = exit_point(&CAR_DOORS, member, left_hand_drive)?;
        Some(world_point(
            self.places.position[p],
            self.places.heading[p],
            local,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled_doors(asset: &str) -> Vec<VehicleDoor> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../godot/bootstrap/mods/vehicles/assets")
            .join(asset)
            .join("asset.toml");
        let text = std::fs::read_to_string(path).unwrap();
        crate::assets::AssetManifest::from_str(&text)
            .unwrap()
            .vehicle
            .unwrap()
            .doors
    }

    #[test]
    fn the_simulated_car_has_the_bundled_estates_doors() {
        assert_eq!(bundled_doors("vehicle.civil.estate_compact"), CAR_DOORS);
    }

    // Each member's door (`side`, `row`) and how many stood at it before, matched against the
    // point the rule gives for each door and use.
    fn seat(
        doors: &[VehicleDoor],
        member: usize,
        left_hand_drive: bool,
    ) -> (DoorSide, DoorRow, usize) {
        let [x, z] = exit_point(doors, member, left_hand_drive).unwrap();
        for d in doors.iter().filter(|d| d.row != DoorRow::Cargo) {
            let out = if d.side == DoorSide::Left {
                STANDING_OFFSET_M
            } else {
                -STANDING_OFFSET_M
            };
            for uses in 0..8 {
                let p = [
                    d.centre[0] + out,
                    d.centre[2] - SHARED_DOOR_STEP_M * uses as f32,
                ];
                if (p[0] - x).abs() < 1e-5 && (p[1] - z).abs() < 1e-5 {
                    return (d.side, d.row, uses);
                }
            }
        }
        panic!("member {member} at [{x}, {z}] stands at no door");
    }

    #[test]
    fn members_take_the_doors_in_order_for_both_drive_sides() {
        use DoorRow::{Front, Rear};
        use DoorSide::{Left, Right};
        // Six members: the largest household admission draws (`MAX_STARTER_IMMIGRANT_HOUSEHOLD_SIZE`).
        let estate = bundled_doors("vehicle.civil.estate_compact");
        let left: Vec<_> = (0..6).map(|m| seat(&estate, m, true)).collect();
        assert_eq!(
            left,
            [
                (Left, Front, 0),
                (Right, Front, 0),
                (Right, Rear, 0),
                (Left, Rear, 0),
                (Right, Rear, 1),
                (Left, Rear, 1)
            ]
        );
        let right: Vec<_> = (0..6).map(|m| seat(&estate, m, false)).collect();
        assert_eq!(
            right,
            [
                (Right, Front, 0),
                (Left, Front, 0),
                (Left, Rear, 0),
                (Right, Rear, 0),
                (Left, Rear, 1),
                (Right, Rear, 1)
            ]
        );
        // The truck has only front doors: every passenger leaves by the passenger door, one step
        // further back each.
        let truck = bundled_doors("vehicle.utility.rigid_box_18t");
        let left: Vec<_> = (0..6).map(|m| seat(&truck, m, true)).collect();
        assert_eq!(
            left,
            [
                (Left, Front, 0),
                (Right, Front, 0),
                (Right, Front, 1),
                (Right, Front, 2),
                (Right, Front, 3),
                (Right, Front, 4)
            ]
        );
        // No two members of one car stand on one spot; no door data, no point.
        for doors in [&estate, &truck] {
            for lhd in [true, false] {
                let points: Vec<_> = (0..6).map(|m| exit_point(doors, m, lhd).unwrap()).collect();
                for (a, p) in points.iter().enumerate() {
                    for q in &points[a + 1..] {
                        assert!(
                            (p[0] - q[0]).hypot(p[1] - q[1]) >= 0.5,
                            "{p:?} and {q:?} overlap"
                        );
                    }
                }
            }
        }
        assert_eq!(exit_point(&[], 0, true), None);
    }

    #[test]
    fn a_point_lands_beside_the_parked_body() {
        // A car whose nose points along world +X: heading = atan2(-1, -0).
        let heading = (-1.0f32).atan2(-0.0);
        let [x, z] = world_point([100.0, 0.0, 50.0], heading, [0.895, 0.3675]);
        // Facing +X, the driver's left is world -Z; the front door is slightly ahead.
        assert!(
            (x - 100.3675).abs() < 1e-4 && (z - 49.105).abs() < 1e-4,
            "{x} {z}"
        );
    }
}
