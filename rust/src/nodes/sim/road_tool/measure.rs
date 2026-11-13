// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool HUD numbers (`ROAD-112` round B, plan section 5): length, bearing, the angle to the
//! connected road at each end, radius, design speed from radius, build cost and upkeep from the
//! existing road type pricing (`road_types::cost`, as the preview prices a stroke). The height
//! values (end heights, slope, structure split) are round I's; their fields stay `None`.
//! Rust sends numbers only; texts are built in GDScript.

use crate::simulation::network::graph::{RoadMedian, RoadParking};
use crate::simulation::network::guide::{DVec2, GuidePath};
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::road_types::cost::{build_per_m, stroke_weight, upkeep_per_m_day};
use crate::simulation::network::types::EdgeClass;
use godot::prelude::Vector3;

// `validate.rs` `design_radius`: R = v^2 / (127 f) with v in km/h and lateral friction f = 0.20.
const SPEED_RADIUS_CONVERSION: f64 = 127.0;
const DESIGN_LATERAL_FRICTION: f64 = 0.20;
const MS_TO_KMH: f64 = 3.6;
// Design speeds are shown rounded down to this step.
const DESIGN_SPEED_STEP_KMH: f64 = 5.0;

/// The drawn road type's cross-section and attributes, as the preview prices a stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadTypeSpec {
    /// Forward lanes.
    pub fwd_lanes: u8,
    /// Backward lanes.
    pub bkw_lanes: u8,
    /// Median, if any (`ROAD-80`).
    pub median: Option<RoadMedian>,
    /// Kerbside parking.
    pub parking: RoadParking,
    /// Speed, sidewalks, zoning, lights (`ROAD-101`).
    pub attributes: RoadAttributes,
}

impl Default for RoadTypeSpec {
    /// The default two-lane road (`builtin:road_2l`).
    fn default() -> Self {
        Self {
            fwd_lanes: 1,
            bkw_lanes: 1,
            median: None,
            parking: RoadParking::default(),
            attributes: RoadAttributes::DEFAULT,
        }
    }
}

/// The HUD panel's numbers for one draft.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HudValues {
    /// Length of the stroke request's polyline in metres (3D, as the preview's build length).
    pub length_m: f64,
    /// Compass bearing of the draft's end tangent in degrees, clockwise from north (`-Z`), in
    /// `[0, 360)`.
    pub bearing_deg: f64,
    /// Angle between the draft and the road it starts on, in degrees: the deflection from
    /// straight on (`0..=180`) out of a dead end, the acute angle (`0..=90`) to a road through
    /// the start. `None` on open ground or at a junction of three or more arms.
    pub start_angle_deg: Option<f64>,
    /// The same at a joined end.
    pub end_angle_deg: Option<f64>,
    /// Smallest arc radius in metres; `None` for a straight draft.
    pub radius_m: Option<f64>,
    /// Design speed of that radius, `sqrt(127 * 0.20 * R)` km/h rounded down to 5 km/h.
    pub design_speed_kmh: Option<f64>,
    /// Whether the design speed is below the road type's speed limit (advice only: cars do not
    /// slow for a curve today).
    pub design_speed_warning: bool,
    /// Build cost of the stroke as a standard (ground) road; the preview's class prices
    /// bridges and tunnels.
    pub build_cost: f64,
    /// Upkeep per day of the same.
    pub upkeep_per_day: f64,
    /// Start height above the ground (round I).
    pub start_height_m: Option<f64>,
    /// End height above the ground (round I).
    pub end_height_m: Option<f64>,
    /// Slope in percent (round I).
    pub slope_percent: Option<f64>,
}

/// How a road meets one end of the draft (from `interaction::road_start_direction`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EndRoad {
    /// Continuing a dead end: the road's own direction out of it (unit XZ).
    Continue(DVec2),
    /// A road through the point, either sense (unit XZ).
    Along(DVec2),
}

/// The HUD numbers for `path`, whose stroke request is `points`, drawn as `spec`. `start_road`
/// and `end_road` are the roads met at the ends. `O(points + segments)`, no allocation.
pub fn measure(
    path: &GuidePath,
    points: &[Vector3],
    spec: &RoadTypeSpec,
    start_road: Option<EndRoad>,
    end_road: Option<EndRoad>,
) -> HudValues {
    let length_m = polyline_length(points);
    let weight = stroke_weight(
        (spec.fwd_lanes, spec.bkw_lanes),
        spec.median,
        spec.parking,
        spec.attributes,
        EdgeClass::Standard,
    );
    let radius_m = Some(path.min_radius()).filter(|radius| radius.is_finite());
    let design_speed_kmh = radius_m.map(design_speed_kmh);
    let limit_kmh = f64::from(spec.attributes.speed_ms) * MS_TO_KMH;
    let (start_tangent, end_tangent) = (path.start_tangent(), path.end_tangent());
    HudValues {
        length_m,
        bearing_deg: end_tangent.map_or(0.0, bearing_deg),
        // A road leaving a dead end straight on continues its direction.
        start_angle_deg: start_tangent
            .zip(start_road)
            .map(|(t, road)| end_angle(t, road)),
        // Arriving at a dead end straight on runs against its outward direction.
        end_angle_deg: end_tangent
            .zip(end_road)
            .map(|(t, road)| end_angle(-t, road)),
        radius_m,
        design_speed_kmh,
        design_speed_warning: design_speed_kmh.is_some_and(|speed| speed < limit_kmh),
        build_cost: length_m * build_per_m(weight),
        upkeep_per_day: length_m * upkeep_per_m_day(weight),
        ..HudValues::default()
    }
}

/// `v = sqrt(127 * 0.20 * R)` km/h, the inverse of `validate::design_radius`, rounded down to
/// 5 km/h.
pub fn design_speed_kmh(radius_m: f64) -> f64 {
    let speed = (SPEED_RADIUS_CONVERSION * DESIGN_LATERAL_FRICTION * radius_m).sqrt();
    (speed / DESIGN_SPEED_STEP_KMH).floor() * DESIGN_SPEED_STEP_KMH
}

/// Compass bearing of an XZ direction: clockwise from north (`-Z`), degrees in `[0, 360)`.
pub fn bearing_deg(direction: DVec2) -> f64 {
    // East (+X) is 90 degrees, south (+Z) 180.
    direction
        .x
        .atan2(-direction.y)
        .to_degrees()
        .rem_euclid(360.0)
}

// The angle between `leaving` (the draft's direction away from the joint) and the road there.
fn end_angle(leaving: DVec2, road: EndRoad) -> f64 {
    let angle = |a: DVec2, b: DVec2| a.perp_dot(b).abs().atan2(a.dot(b)).to_degrees();
    match road {
        EndRoad::Continue(direction) => angle(leaving, direction),
        EndRoad::Along(direction) => {
            let a = angle(leaving, direction);
            a.min(180.0 - a)
        }
    }
}

// 3D polyline length, summed in `f64` as the preview's build length.
fn polyline_length(points: &[Vector3]) -> f64 {
    points
        .windows(2)
        .map(|pair| {
            let dx = f64::from(pair[1].x - pair[0].x);
            let dy = f64::from(pair[1].y - pair[0].y);
            let dz = f64::from(pair[1].z - pair[0].z);
            (dx * dx + dy * dy + dz * dz).sqrt()
        })
        .sum()
}
