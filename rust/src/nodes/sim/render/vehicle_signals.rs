// SPDX-License-Identifier: GPL-3.0-or-later

//! Brake lamps, indicators, the reverse lamp and the steered wheels' target (`RENDER-66`,
//! docs/vehicles.md "As built: signals"): the lamp bits and the steer angle of one posed car,
//! read from the simulation's state under the snapshot's lock.
//!
//! Presentation only: every value is a pure function of the state the snapshot reads (the same
//! state draws the same bits and angle in every run), nothing is written back and nothing is
//! saved. Per posed car O(1) apart from lane samples and bucket searches, O(log lane points) and
//! O(log cars in a lane) each, and, near a lane end, the planned connector the speed pass also
//! finds. Nothing allocates. A tier computes only what it shows: LOD3 has no lamp faces and no
//! wheels, LOD2 no wheels, and indicators are worked out only within [`SIGNAL_RANGE_M`].
//!
//! Rules:
//!
//! - **Brake**: a car on a live lane brakes when it stands (speed at most [`BRAKE_STAND_MS`]),
//!   when one of the speed pass's braking caps (a junction approach, a blocked exit lane, a
//!   blocked lane change) is below its speed, or when its IDM acceleration is below
//!   -[`BRAKE_DECEL_MS2`]: what the next tick's speed update would read. A car off every lane
//!   brakes while it waits to join its lane (the attach hold, or an egress car at its lane
//!   point). The margin on the IDM keeps a car following at a steady gap, whose acceleration
//!   hovers about zero, from flickering.
//! - **Indicators** (left, right): during a lane change, toward the target lane; on a road lane
//!   within the lead ([`SIGNAL_LEAD_M`], or [`SIGNAL_LEAD_S`] of its speed if longer) of its
//!   lane end, toward the bend of its planned connector, and on that connector to its end; within
//!   the lead of its detach point, toward its destination's door; pulling out, toward the way it
//!   joins its lane; pulling in, toward the side it left the lane until it has turned
//!   [`PULL_IN_TURNED_RAD`] from it. A connector bends when its first and last tangent differ by
//!   [`BEND_RAD`]. Roundabouts: no signal entering or circulating, the exit's bend side leaving.
//! - **Reverse**: a truck drawn tail first in a yard manoeuvre (backing out of a dock, or into a
//!   tail-in one). It shows no indicator then.
//! - **Steer**: on a lane, the signed angle between the path's tangents at the front and rear
//!   axle (an arc of radius `R` gives `L / R` for a wheelbase `L`), looking into the planned
//!   connector, or a connector's exit lane, when the front axle is past the lane's end; off a
//!   lane, `atan(L κ)` of the drawn arc's curvature `κ`, negated for a vehicle backing. Clamped
//!   to [`STEER_LOCK_RAD`] and quantised to [`STEER_QUANTUM_RAD`].
//! - A truck standing in its dock is parked: no lamp lit, wheels straight.

use crate::nodes::sim::core::SimCore;
use crate::nodes::sim::render::lane_pose::{sample_lane_change_pose, sample_lane_pose};
use crate::nodes::sim::render::vehicle_catalogue::{Axles, LAMP_LIGHTS_ON};
use crate::simulation::economy::agents::tick::idm_acceleration;
use crate::simulation::economy::agents::{
    ACCESS_LANE_ATTACH_PENDING, ACCESS_PLAN_VALID, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS,
    TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use crate::simulation::network::lanes::Lane;
use crate::simulation::parking::ParkingTripPhase;
use godot::prelude::{Vector2, Vector3};

/// The instance word's brake lamp bit.
pub(crate) const LAMP_BRAKE: u32 = 1 << 1;
/// The instance word's left indicator bit.
pub(crate) const LAMP_LEFT: u32 = 1 << 2;
/// The instance word's right indicator bit.
pub(crate) const LAMP_RIGHT: u32 = 1 << 3;
/// The instance word's reverse lamp bit.
pub(crate) const LAMP_REVERSE: u32 = 1 << 4;

/// Indicators are worked out within this distance of the eye, metres; beyond it the bits are
/// zero (a lens is a few pixels there).
pub(crate) const SIGNAL_RANGE_M: f32 = 100.0;
/// A car signals a turn or a stop at the kerb from this far before it, metres, ...
pub(crate) const SIGNAL_LEAD_M: f32 = 40.0;
/// ... or from this many seconds of its speed before it, when that is farther.
pub(crate) const SIGNAL_LEAD_S: f32 = 3.0;
/// A connector or a pull-out signals a turn of at least this between its ends, radians (25°).
pub(crate) const BEND_RAD: f32 = 25.0 * std::f32::consts::PI / 180.0;
/// A pull-out whose way joins its lane straight signals toward a lane point at least this far
/// to its side, metres (out of a kerb bay).
pub(crate) const PULL_OUT_LATERAL_M: f32 = 0.5;
/// A car pulling in signals until it faces this far from its lane's direction, radians (60°).
pub(crate) const PULL_IN_TURNED_RAD: f32 = 60.0 * std::f32::consts::PI / 180.0;
/// A car on a live lane at or below this speed stands, m/s.
pub(crate) const BRAKE_STAND_MS: f32 = 0.05;
/// A car whose IDM acceleration is below minus this brakes, m/s².
pub(crate) const BRAKE_DECEL_MS2: f32 = 1.0;
/// An egress car this close to its lane point waits there, metres.
pub(crate) const HANDOFF_STAND_M: f32 = 0.5;
/// The steering lock, radians (35°, the contract's wheel-arch clearance).
pub(crate) const STEER_LOCK_RAD: f32 = 35.0 * std::f32::consts::PI / 180.0;
/// The steer angle's step, radians (a quarter of a degree): a car on a gentle curve or a
/// straight reads one value tick after tick, so its row does not change.
pub(crate) const STEER_QUANTUM_RAD: f32 = 0.25 * std::f32::consts::PI / 180.0;
// Tiers below this have wheel parts (LOD0, LOD1), and below `LAMP_TIERS` lamp faces.
const WHEEL_TIERS: u8 = 2;
const LAMP_TIERS: u8 = 3;

/// How the snapshot posed a car.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CarPath {
    /// On its lane; `changing` while blended from its source lane in a lane change; `gate` the
    /// curvature of its drawn gate turn (`PARK-31`) near the lane point it leaves or joined its
    /// lane at (1/m, positive to the left of travel).
    Lane { changing: bool, gate: Option<f32> },
    /// Off every lane: its unit nose direction, and the curvature of its drawn travel (1/m,
    /// positive to the left of travel, 0 on a straight part).
    OffLane {
        facing: Option<Vector3>,
        curvature: f32,
    },
}

impl SimCore {
    /// Car `i`'s lamp bits and front-wheel steer angle (radians, positive to the left) as drawn
    /// at `tier`, `distance_sq` from the eye (`None`: no eye, everything computed), with the
    /// model's `axles` (`None`: wheels straight). Module docs for the rules and cost.
    pub(crate) fn car_signals(
        &self,
        i: usize,
        axles: Option<Axles>,
        tier: u8,
        distance_sq: Option<f32>,
        path: CarPath,
    ) -> (u32, f32) {
        let trip = &self.agents.parking_trip[i];
        if trip.phase == ParkingTripPhase::AtDock {
            return (0, 0.0);
        }
        let mut lamps = LAMP_LIGHTS_ON;
        if tier >= LAMP_TIERS {
            return (lamps, 0.0);
        }
        let off_lane = matches!(path, CarPath::OffLane { .. });
        let reversing = off_lane
            && matches!(
                trip.phase,
                ParkingTripPhase::PullIn | ParkingTripPhase::PullOut
            )
            && self.parking.yard_pose_reversed(trip);
        if self.car_brakes(i, off_lane) {
            lamps |= LAMP_BRAKE;
        }
        if reversing {
            lamps |= LAMP_REVERSE;
        } else if distance_sq.is_none_or(|d| d <= SIGNAL_RANGE_M * SIGNAL_RANGE_M) {
            lamps |= self.car_indicator(i, path);
        }
        let steer = match axles {
            Some(axles) if tier < WHEEL_TIERS => self.car_steer(i, axles, path, reversing),
            _ => 0.0,
        };
        (lamps, quantised_steer(steer))
    }

    // The brake rule (module docs). O(1) off a lane but for one lane sample; on a lane the speed
    // pass's own reads.
    fn car_brakes(&self, i: usize, off_lane: bool) -> bool {
        let agents = &self.agents;
        if off_lane {
            if agents.access_flags[i] & ACCESS_LANE_ATTACH_PENDING != 0 {
                return true;
            }
            if agents.transit[i] != TRANSIT_ACCESS_EGRESS {
                return false;
            }
            let car = Vector2::new(agents.pos_x[i], agents.pos_y[i]);
            return self
                .lane_point(
                    agents.planned_attach_lane_id[i],
                    agents.planned_attach_lane_d[i],
                )
                .is_some_and(|(point, _)| flat(point).distance_to(car) <= HANDOFF_STAND_M);
        }
        let speed = agents.speed[i];
        self.agents
            .speed_outlook(i, &self.transit_network, &self.region_graph)
            .is_some_and(|outlook| {
                speed <= BRAKE_STAND_MS
                    || outlook.cap < speed
                    || idm_acceleration(speed, outlook.v_max, outlook.gap) < -BRAKE_DECEL_MS2
            })
    }

    // The indicator bits (module docs).
    fn car_indicator(&self, i: usize, path: CarPath) -> u32 {
        let agents = &self.agents;
        let lanes = &self.transit_network.lane_system.lanes;
        match path {
            CarPath::Lane { changing: true, .. } => {
                let d = agents.lane_distance[i];
                let (Some(from), Some(to)) = (
                    lanes.get(agents.lane_change_from_lane_id[i] as usize),
                    lanes.get(agents.current_lane_id[i]),
                ) else {
                    return 0;
                };
                match (sample_lane_pose(from, d), sample_lane_pose(to, d)) {
                    (Some((from_point, tangent)), Some((to_point, _))) => {
                        side_bit(left_of(tangent, to_point - from_point))
                    }
                    _ => 0,
                }
            }
            CarPath::Lane { changing: false, .. } => self.lane_indicator(i),
            CarPath::OffLane { facing, .. } => {
                let Some(facing) = facing else {
                    return 0;
                };
                let transit = agents.transit[i];
                if transit == TRANSIT_ACCESS_EGRESS
                    || agents.access_flags[i] & ACCESS_LANE_ATTACH_PENDING != 0
                {
                    self.pull_out_indicator(i, facing)
                } else if transit == TRANSIT_ACCESS_INGRESS {
                    self.pull_in_indicator(i, facing)
                } else {
                    0
                }
            }
        }
    }

    // A car on its lane: its connector's bend on a connector; on a road lane its stop at the
    // kerb, else its planned connector's bend, within the lead.
    fn lane_indicator(&self, i: usize) -> u32 {
        let agents = &self.agents;
        let lanes = &self.transit_network.lane_system.lanes;
        let lane_id = agents.current_lane_id[i];
        let Some(lane) = lanes.get(lane_id) else {
            return 0;
        };
        match agents.transit[i] {
            TRANSIT_INTERSECTION => self.connector_signal(lane_id),
            TRANSIT_NETWORK if lane.edge_id != usize::MAX => {
                let d = agents.lane_distance[i];
                let lead = SIGNAL_LEAD_M.max(SIGNAL_LEAD_S * agents.speed[i]);
                if agents.access_flags[i] & ACCESS_PLAN_VALID != 0
                    && agents.planned_detach_lane_id[i] as usize == lane_id
                    && agents.planned_detach_lane_d[i] >= d
                {
                    // It leaves this lane at its destination, before the junction.
                    if agents.planned_detach_lane_d[i] - d > lead {
                        return 0;
                    }
                    return self.kerb_indicator(i, lane, d);
                }
                if lane.length - d > lead {
                    return 0;
                }
                self.agents
                    .planned_connector(i, &self.transit_network, &self.region_graph)
                    .map_or(0, |connector| self.connector_signal(connector))
            }
            _ => 0,
        }
    }

    // Toward the door of the building car `i` drives to, seen from its place on `lane`.
    fn kerb_indicator(&self, i: usize, lane: &Lane, d: f32) -> u32 {
        let Some(entrance) = self.allocator.entrances.get(self.agents.target_building[i]) else {
            return 0;
        };
        let Some((point, tangent)) = sample_lane_pose(lane, d) else {
            return 0;
        };
        let door = Vector3::new(entrance.door_pos.x, point.y, entrance.door_pos.y);
        side_bit(left_of(tangent, door - point))
    }

    // Pulling out: toward the way the car turns onto its lane, or, joining it straight, toward
    // a lane point beside it.
    fn pull_out_indicator(&self, i: usize, facing: Vector3) -> u32 {
        let agents = &self.agents;
        let Some((point, tangent)) = self.lane_point(
            agents.planned_attach_lane_id[i],
            agents.planned_attach_lane_d[i],
        ) else {
            return 0;
        };
        let turn = signed_angle(facing, tangent);
        if turn.abs() >= BEND_RAD {
            return side_bit(turn);
        }
        let car = Vector3::new(agents.pos_x[i], point.y, agents.pos_y[i]);
        let lateral = left_of(facing, point - car);
        if lateral.abs() >= PULL_OUT_LATERAL_M {
            side_bit(lateral)
        } else {
            0
        }
    }

    // Pulling in: toward the side of its lane the car is on, until it has turned away from the
    // lane's direction.
    fn pull_in_indicator(&self, i: usize, facing: Vector3) -> u32 {
        let agents = &self.agents;
        let Some((point, tangent)) = self.lane_point(
            agents.planned_detach_lane_id[i],
            agents.planned_detach_lane_d[i],
        ) else {
            return 0;
        };
        if signed_angle(tangent, facing).abs() >= PULL_IN_TURNED_RAD {
            return 0;
        }
        let car = Vector3::new(agents.pos_x[i], point.y, agents.pos_y[i]);
        side_bit(left_of(tangent, car - point))
    }

    // The indicator a car on or before connector `connector_id` shows: its bend side, none at a
    // roundabout but leaving it.
    fn connector_signal(&self, connector_id: usize) -> u32 {
        let lanes = &self.transit_network.lane_system.lanes;
        let Some(connector) = lanes.get(connector_id) else {
            return 0;
        };
        if connector.ring_entry.is_some() {
            return 0;
        }
        let Some(turn) = connector_turn(connector) else {
            return 0;
        };
        let graph = &self.region_graph;
        let ring_edge =
            |edge: usize| edge < graph.edge_count() && graph.edge(edge).roundabout.is_some();
        let at_ring = connector.node_id < graph.node_count()
            && graph
                .node_adjacency(connector.node_id as u32)
                .iter()
                .any(|&edge| ring_edge(edge));
        if at_ring {
            // Onto the ring is circulating; off it is the exit, signalled whatever its bend.
            let onto_ring = connector
                .next_lanes
                .first()
                .and_then(|&next| lanes.get(next))
                .is_some_and(|next| ring_edge(next.edge_id));
            return if onto_ring { 0 } else { side_bit(turn) };
        }
        if turn.abs() >= BEND_RAD {
            side_bit(turn)
        } else {
            0
        }
    }

    // The steer target (module docs), unclamped.
    fn car_steer(&self, i: usize, axles: Axles, path: CarPath, reversing: bool) -> f32 {
        let wheelbase = axles.wheelbase_m();
        match path {
            CarPath::OffLane { curvature, .. } => {
                let steer = (wheelbase * curvature).atan();
                if reversing { -steer } else { steer }
            }
            // Its drawn gate turn steers as an arc off the lane does.
            CarPath::Lane {
                gate: Some(curvature),
                ..
            } => (wheelbase * curvature).atan(),
            CarPath::Lane {
                changing,
                gate: None,
            } => {
                let agents = &self.agents;
                let lanes = &self.transit_network.lane_system.lanes;
                let Some(lane) = lanes.get(agents.current_lane_id[i]) else {
                    return 0.0;
                };
                let d = agents.lane_distance[i];
                let tangents = if changing {
                    let Some(from) = lanes.get(agents.lane_change_from_lane_id[i] as usize) else {
                        return 0.0;
                    };
                    let (start, length) = (
                        agents.lane_change_start_d[i],
                        agents.lane_change_length_m[i],
                    );
                    let at = |offset: f32| {
                        sample_lane_change_pose(from, lane, d + offset, start, length)
                            .map(|(_, tangent)| tangent)
                    };
                    at(axles.rear_m).zip(at(axles.front_m))
                } else {
                    let rear = sample_lane_pose(lane, d + axles.rear_m).map(|(_, t)| t);
                    rear.zip(self.tangent_ahead(i, lane, d + axles.front_m))
                };
                tangents.map_or(0.0, |(rear, front)| signed_angle(rear, front))
            }
        }
    }

    // The path's tangent `d` metres along car `i`'s `lane`, past its end on the lane it takes
    // next: its planned connector from a road lane, a connector's exit lane.
    fn tangent_ahead(&self, i: usize, lane: &Lane, d: f32) -> Option<Vector3> {
        if d > lane.length {
            let lanes = &self.transit_network.lane_system.lanes;
            let next = if lane.edge_id == usize::MAX {
                lane.next_lanes.first().copied()
            } else {
                self.agents
                    .planned_connector(i, &self.transit_network, &self.region_graph)
            };
            if let Some(next) = next.and_then(|next| lanes.get(next)) {
                return sample_lane_pose(next, d - lane.length).map(|(_, tangent)| tangent);
            }
        }
        sample_lane_pose(lane, d).map(|(_, tangent)| tangent)
    }

    // The point and tangent `d` metres along lane `lane_id`. O(log lane points).
    fn lane_point(&self, lane_id: u32, d: f32) -> Option<(Vector3, Vector3)> {
        sample_lane_pose(
            self.transit_network
                .lane_system
                .lanes
                .get(lane_id as usize)?,
            d,
        )
    }
}

// A connector's turn from its first to its last tangent, radians, positive to the left; a
// U-turn takes its side from the chord. `None` for a degenerate connector. O(points) at worst,
// O(1) for the connectors the builder makes.
fn connector_turn(connector: &Lane) -> Option<f32> {
    let first = connector
        .geometry
        .windows(2)
        .find_map(|pair| unit(pair[1] - pair[0]))?;
    let last = connector
        .geometry
        .windows(2)
        .rev()
        .find_map(|pair| unit(pair[1] - pair[0]))?;
    let turn = signed_angle(first, last);
    if turn.abs() < 0.9 * std::f32::consts::PI {
        return Some(turn);
    }
    let chord = *connector.geometry.last()? - connector.geometry[0];
    Some(turn.abs().copysign(left_of(first, chord)))
}

// The ground-plane unit vector of `v`, if it has length.
fn unit(v: Vector3) -> Option<Vector3> {
    let flat = Vector3::new(v.x, 0.0, v.z);
    (flat.length_squared() > 1e-8).then(|| flat.normalized())
}

// How far `offset` lies to the left of direction `forward` on the ground plane (scaled by
// `forward`'s length): `+X` is the right of a car heading `-Z`, so the left is `UP × forward`.
fn left_of(forward: Vector3, offset: Vector3) -> f32 {
    Vector3::UP
        .cross(Vector3::new(forward.x, 0.0, forward.z))
        .dot(offset)
}

// The ground-plane angle from `from` to `to`, radians, positive turning left.
fn signed_angle(from: Vector3, to: Vector3) -> f32 {
    let (from, to) = (
        Vector3::new(from.x, 0.0, from.z),
        Vector3::new(to.x, 0.0, to.z),
    );
    let sine = Vector3::UP.cross(from).dot(to);
    sine.atan2(from.dot(to))
}

fn side_bit(left: f32) -> u32 {
    if left > 0.0 {
        LAMP_LEFT
    } else if left < 0.0 {
        LAMP_RIGHT
    } else {
        0
    }
}

fn flat(v: Vector3) -> Vector2 {
    Vector2::new(v.x, v.z)
}

// Clamped to the lock and rounded to the quantum; zero is +0.0, so a straight car's bits never
// change sign.
fn quantised_steer(steer: f32) -> f32 {
    if !steer.is_finite() {
        return 0.0;
    }
    let steps = (steer.clamp(-STEER_LOCK_RAD, STEER_LOCK_RAD) / STEER_QUANTUM_RAD).round();
    if steps == 0.0 {
        0.0
    } else {
        steps * STEER_QUANTUM_RAD
    }
}

#[cfg(test)]
mod tests;
