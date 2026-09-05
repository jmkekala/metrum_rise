// SPDX-License-Identifier: GPL-2.0-only

//! Junction and connector speed helpers.

use crate::config::{
    CAR_JUNCTION_LATERAL_ACCEL_MS2, CAR_JUNCTION_MIN_SPEED_MS, CAR_JUNCTION_SPEED_MS,
};
use crate::simulation::network::lanes::Lane;
use godot::prelude::Vector3;

/// Caps a car speed by the global junction design speed.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn junction_car_speed(speed: f32) -> f32 {
    speed.min(CAR_JUNCTION_SPEED_MS)
}

fn flat_unit(v: Vector3) -> Option<Vector3> {
    let flat = Vector3::new(v.x, 0.0, v.z);
    if flat.length_squared() > 1.0e-8 {
        Some(flat.normalized())
    } else {
        None
    }
}

fn lane_end_tangent(lane: &Lane, at_start: bool) -> Option<Vector3> {
    if lane.geometry.len() < 2 {
        return None;
    }
    if at_start {
        for segment in lane.geometry.windows(2) {
            if let Some(tangent) = flat_unit(segment[1] - segment[0]) {
                return Some(tangent);
            }
        }
    } else {
        for idx in (1..lane.geometry.len()).rev() {
            if let Some(tangent) = flat_unit(lane.geometry[idx] - lane.geometry[idx - 1]) {
                return Some(tangent);
            }
        }
    }
    None
}

/// Returns the curvature-limited speed cap for a junction connector lane.
pub(in crate::simulation::economy::agents::tick) fn connector_turn_speed(
    connector_lane: &Lane,
) -> f32 {
    let Some(start_tangent) = lane_end_tangent(connector_lane, true) else {
        return CAR_JUNCTION_SPEED_MS;
    };
    let Some(end_tangent) = lane_end_tangent(connector_lane, false) else {
        return CAR_JUNCTION_SPEED_MS;
    };

    let dot = start_tangent.dot(end_tangent).clamp(-1.0, 1.0);
    let turn_angle_rad = dot.acos();
    if turn_angle_rad < 0.15 {
        return CAR_JUNCTION_SPEED_MS;
    }

    let radius_m = connector_lane.length.max(0.1) / turn_angle_rad;
    (CAR_JUNCTION_LATERAL_ACCEL_MS2 * radius_m)
        .sqrt()
        .clamp(CAR_JUNCTION_MIN_SPEED_MS, CAR_JUNCTION_SPEED_MS)
}

/// Caps a car entering a connector by the connector-specific turn speed.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn junction_entry_speed(
    speed: f32,
    connector_lane: &Lane,
) -> f32 {
    speed.min(connector_turn_speed(connector_lane))
}
