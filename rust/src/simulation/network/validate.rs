// SPDX-License-Identifier: GPL-2.0-only

//! Read-only road geometry measurements, independent of placement and surface compilation.
//!
//! Uses the renderer's physical centreline (control geometry only when physical geometry is
//! empty). Stations are unclipped three-dimensional arc lengths from the stored start.
//! Node directions use an unclipped one-metre horizontal prefix: clips reserve ownership,
//! rather than changing the alignment at the node. Road and foot edges are checked; other
//! transit modes have different design constraints and are excluded.

use super::grade::{ROAD_GRADE_MAX, ROAD_GRADE_WARNING, steepest_grade};
use super::graph::{Edge, RegionGraph};
use super::roadbed::visual_roadbed_half_width_m;
use super::types::{EdgeClass, TransitType};
use godot::prelude::Vector3;
use rayon::prelude::*;

// Bridge/tunnel design target supplied by the road validator specification.
const STRUCTURAL_DESIGN_GRADE: f32 = 0.06;
// Matches ROAD_PROFILE_MAX_GRADE_CHANGE_PER_M in surface/edge/input.rs.
const DESIGN_GRADE_CHANGE_PER_M: f32 = 0.018;
// Hard vertical-curvature bound: the specification's 0.05 with the ~20% leeway placement allows
// over real-world maxima.
const ILLEGAL_GRADE_CHANGE_PER_M: f32 = 0.06;
/// Safety margin above the offset-curve folding radius (the roadbed half-width): the illegal
/// horizontal radius in roadbed half-widths.
pub(crate) const ILLEGAL_RADIUS_HALF_WIDTH_FACTOR: f32 = 1.25;
// Length of road, in roadbed half-widths, over which turning is measured for the radius. With
// the illegal radius at 1.25 half-widths, a stretch this long is refused once it turns more than
// about 137 degrees: a hairpin, not a street corner.
/// Radius measurement window length in complete roadbed half-widths.
pub(crate) const RADIUS_WINDOW_HALF_WIDTHS: f32 = 3.0;
// Design lateral friction supplied for the speed-radius relationship.
const DESIGN_LATERAL_FRICTION: f32 = 0.20;
// Converts km/h squared to metres in R = v² / (127 f), without superelevation.
const SPEED_RADIUS_CONVERSION: f32 = 127.0;
// Unit conversion from graph speed_limit (m/s) to the radius formula (km/h).
const MS_TO_KMH: f32 = 3.6;
// Degree-two design continuity tolerance supplied by the validator specification.
const DESIGN_KINK_DEGREES: f32 = 3.0;
// Degree-two hard continuity tolerance supplied by the validator specification.
const ILLEGAL_KINK_DEGREES: f32 = 10.0;
// Design separation of adjacent junction arms supplied by the specification.
const DESIGN_ARM_ANGLE_DEGREES: f32 = 60.0;
// Hard separation of adjacent junction arms: the specification's 30 degrees with the ~20%
// leeway placement allows over real-world minima.
const ILLEGAL_ARM_ANGLE_DEGREES: f32 = 25.0;
// Conventional four-arm junction design target supplied by the specification.
const DESIGN_JUNCTION_DEGREE: usize = 4;
// Hard junction-degree bound supplied by the specification.
const ILLEGAL_JUNCTION_DEGREE: usize = 6;
// Ignores horizontally degenerate segments as required by the specification.
const MIN_SEGMENT_RUN_M: f32 = 0.05;
// Endpoint direction over the first metre: long enough to skip sub-5 cm end segments, short
// enough that a smooth R = 50 m curve split at the node reads about 1 degree, not a kink.
const ARM_DIRECTION_RUN_M: f32 = 1.0;

/// Severity of a geometry measurement exceeding its applicable threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoadGeometryTier {
    /// Exceeds a hard geometric bound; a future placement caller may reject it.
    Illegal,
    /// Exceeds a design target while remaining within the hard bound.
    Design,
}

/// Two thresholds for a quantity, tested with strict inequalities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadGeometryThreshold<T> {
    /// Hard bound, evaluated before the design target.
    pub illegal: T,
    /// Warning target, used only when the hard bound is satisfied.
    pub design: T,
}

/// Configurable geometry bounds; grade is rise/run and angles are in degrees.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadGeometryLimits {
    /// Maximum absolute grade for surface roads.
    pub standard_grade: RoadGeometryThreshold<f32>,
    /// Maximum absolute grade for elevated roads.
    pub bridge_grade: RoadGeometryThreshold<f32>,
    /// Maximum absolute grade for underground roads.
    pub tunnel_grade: RoadGeometryThreshold<f32>,
    /// Maximum change of signed grade per horizontal metre.
    pub grade_change_per_m: RoadGeometryThreshold<f32>,
    /// Minimum illegal radius is this factor times the complete visual roadbed half-width.
    pub horizontal_radius_illegal_half_width_factor: f32,
    /// Minimum design radius is speed in km/h squared divided by 127 times this friction.
    pub horizontal_radius_design_lateral_friction: f32,
    /// Maximum departure from straight continuation at a degree-two node.
    pub direction_kink_degrees: RoadGeometryThreshold<f32>,
    /// Minimum angular gap between adjacent arms, including the wraparound gap.
    pub junction_arm_angle_degrees: RoadGeometryThreshold<f32>,
    /// Maximum number of live incident road/foot edges at a junction.
    pub junction_degree: RoadGeometryThreshold<usize>,
}

impl Default for RoadGeometryLimits {
    fn default() -> Self {
        Self {
            standard_grade: RoadGeometryThreshold {
                illegal: ROAD_GRADE_MAX,
                design: ROAD_GRADE_WARNING,
            },
            bridge_grade: RoadGeometryThreshold {
                illegal: ROAD_GRADE_MAX,
                design: STRUCTURAL_DESIGN_GRADE,
            },
            tunnel_grade: RoadGeometryThreshold {
                illegal: ROAD_GRADE_MAX,
                design: STRUCTURAL_DESIGN_GRADE,
            },
            grade_change_per_m: RoadGeometryThreshold {
                illegal: ILLEGAL_GRADE_CHANGE_PER_M,
                design: DESIGN_GRADE_CHANGE_PER_M,
            },
            horizontal_radius_illegal_half_width_factor: ILLEGAL_RADIUS_HALF_WIDTH_FACTOR,
            horizontal_radius_design_lateral_friction: DESIGN_LATERAL_FRICTION,
            direction_kink_degrees: RoadGeometryThreshold {
                illegal: ILLEGAL_KINK_DEGREES,
                design: DESIGN_KINK_DEGREES,
            },
            junction_arm_angle_degrees: RoadGeometryThreshold {
                illegal: ILLEGAL_ARM_ANGLE_DEGREES,
                design: DESIGN_ARM_ANGLE_DEGREES,
            },
            junction_degree: RoadGeometryThreshold {
                illegal: ILLEGAL_JUNCTION_DEGREE,
                design: DESIGN_JUNCTION_DEGREE,
            },
        }
    }
}

/// Worst measurement for one check on an edge or node; illegal supersedes design.
#[derive(Clone, Debug, PartialEq)]
pub enum RoadGeometryViolation {
    /// Absolute segment rise divided by horizontal run.
    Grade {
        /// Graph edge index.
        edge_id: usize,
        /// Segment midpoint station in three-dimensional metres from the stored start.
        station_m: f32,
        /// Absolute rise/run, not a percentage.
        measured: f32,
        /// Exceeded maximum rise/run.
        limit: f32,
        /// Applicable severity.
        tier: RoadGeometryTier,
    },
    /// Change of signed grade divided by the mean adjacent horizontal runs.
    GradeChange {
        /// Graph edge index.
        edge_id: usize,
        /// Shared vertex station in three-dimensional metres from the stored start.
        station_m: f32,
        /// Absolute grade change per horizontal metre.
        measured: f32,
        /// Exceeded maximum grade change per horizontal metre.
        limit: f32,
        /// Applicable severity.
        tier: RoadGeometryTier,
    },
    /// Mean adjacent horizontal runs divided by the horizontal turn angle in radians.
    HorizontalRadius {
        /// Graph edge index.
        edge_id: usize,
        /// Shared vertex station in three-dimensional metres from the stored start.
        station_m: f32,
        /// Estimated horizontal radius in metres.
        measured: f32,
        /// Violated minimum radius in metres.
        limit: f32,
        /// Applicable severity.
        tier: RoadGeometryTier,
    },
    /// Departure from straight continuation between two outgoing arm directions.
    DirectionKink {
        /// Canonical graph node ID.
        node_id: u32,
        /// Departure from 180-degree arm separation, in degrees.
        measured: f32,
        /// Exceeded maximum kink in degrees.
        limit: f32,
        /// Applicable severity.
        tier: RoadGeometryTier,
    },
    /// Smallest angular separation between adjacent junction arms.
    JunctionArmAngle {
        /// Canonical graph node ID.
        node_id: u32,
        /// Smallest angular gap in degrees.
        measured: f32,
        /// Violated minimum gap in degrees.
        limit: f32,
        /// Applicable severity.
        tier: RoadGeometryTier,
    },
    /// Number of live incident road/foot edges, including directionless edges.
    JunctionDegree {
        /// Canonical graph node ID.
        node_id: u32,
        /// Live incident edge count.
        measured: usize,
        /// Exceeded maximum incident edge count.
        limit: usize,
        /// Applicable severity.
        tier: RoadGeometryTier,
    },
}

impl RoadGeometryViolation {
    fn sort_key(&self) -> (u8, usize) {
        match *self {
            Self::Grade { edge_id, .. } => (0, edge_id),
            Self::GradeChange { edge_id, .. } => (1, edge_id),
            Self::HorizontalRadius { edge_id, .. } => (2, edge_id),
            Self::DirectionKink { node_id, .. } => (3, node_id as usize),
            Self::JunctionArmAngle { node_id, .. } => (4, node_id as usize),
            Self::JunctionDegree { node_id, .. } => (5, node_id as usize),
        }
    }
}

fn upper_violation<T: Copy + PartialOrd>(
    value: T,
    bounds: &RoadGeometryThreshold<T>,
) -> Option<(T, RoadGeometryTier)> {
    if value > bounds.illegal {
        Some((bounds.illegal, RoadGeometryTier::Illegal))
    } else if value > bounds.design {
        Some((bounds.design, RoadGeometryTier::Design))
    } else {
        None
    }
}

fn lower_violation(
    value: f32,
    bounds: &RoadGeometryThreshold<f32>,
) -> Option<(f32, RoadGeometryTier)> {
    if value < bounds.illegal {
        Some((bounds.illegal, RoadGeometryTier::Illegal))
    } else if value < bounds.design {
        Some((bounds.design, RoadGeometryTier::Design))
    } else {
        None
    }
}

fn is_road(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}

/// Physical alignment, falling back to control geometry for uncompiled edges.
pub(crate) fn centreline(edge: &Edge) -> &[Vector3] {
    if edge.physical_geometry.is_empty() {
        &edge.geometry
    } else {
        &edge.physical_geometry
    }
}

/// Appends at most one violation for each edge check in O(points) time and O(1) scratch space.
///
/// Deleted/non-road edges are ignored. Segments with horizontal run below 0.05 m are
/// excluded and break consecutive-segment comparisons. Ties retain the earliest station.
/// No allocations occur in the point loop; only appended output may allocate.
pub fn validate_edge(
    graph: &RegionGraph,
    edge_id: usize,
    limits: &RoadGeometryLimits,
    output: &mut Vec<RoadGeometryViolation>,
) {
    let edge = graph.edge(edge_id);
    if !is_road(edge) {
        return;
    }
    let mut worst_change = (0.0, 0.0);
    let points = centreline(edge);
    let half_width_m = visual_roadbed_half_width_m(edge);
    let smallest_radius = smallest_turning_radius(points, RADIUS_WINDOW_HALF_WIDTHS * half_width_m);
    visit_grades(points, |station, _, _, change| {
        if change > worst_change.0 {
            worst_change = (change, station);
        }
    });
    output.extend(grade_violation(graph, edge_id, limits));
    if let Some((limit, tier)) = upper_violation(worst_change.0, &limits.grade_change_per_m) {
        output.push(RoadGeometryViolation::GradeChange {
            edge_id,
            station_m: worst_change.1,
            measured: worst_change.0,
            limit,
            tier,
        });
    }
    let radius_bounds = RoadGeometryThreshold {
        illegal: half_width_m * limits.horizontal_radius_illegal_half_width_factor,
        design: design_radius(
            edge.speed_limit,
            limits.horizontal_radius_design_lateral_friction,
        ),
    };
    // Illegal only when the reading plus its rounding bound is under the limit (so a curve at
    // the limit never reads under it through `f32` rounding); the design tier reads the radius.
    let (radius, rounding, station_m) = smallest_radius;
    let violation = if radius + rounding < radius_bounds.illegal {
        Some((radius_bounds.illegal, RoadGeometryTier::Illegal))
    } else {
        (radius < radius_bounds.design).then_some((radius_bounds.design, RoadGeometryTier::Design))
    };
    if let Some((limit, tier)) = violation {
        output.push(RoadGeometryViolation::HorizontalRadius {
            edge_id,
            station_m,
            measured: radius,
            limit,
            tier,
        });
    }
}

/// The grade check of [`validate_edge`] alone: `edge_id`'s steepest segment (the one grade,
/// [`steepest_grade`]) against its class's tiers, at the segment's midpoint station. Placement's
/// checks of the profiles an edit re-derives read only this. O(points), no allocation.
pub(crate) fn grade_violation(
    graph: &RegionGraph,
    edge_id: usize,
    limits: &RoadGeometryLimits,
) -> Option<RoadGeometryViolation> {
    let edge = graph.edge(edge_id);
    if !is_road(edge) {
        return None;
    }
    let grade_bounds = match edge.class {
        EdgeClass::Standard => &limits.standard_grade,
        EdgeClass::Bridge => &limits.bridge_grade,
        EdgeClass::Tunnel => &limits.tunnel_grade,
    };
    let steepest = steepest_grade(centreline(edge))?;
    let (limit, tier) = upper_violation(steepest.grade, grade_bounds)?;
    Some(RoadGeometryViolation::Grade {
        edge_id,
        station_m: steepest.station_m + 0.5 * steepest.length_m,
        measured: steepest.grade,
        limit,
        tier,
    })
}

/// Visits segment start station, 3D length, signed rise/run and grade change per metre.
/// Degenerate horizontal segments report NaN grade and break consecutive comparisons.
/// O(points), no allocation; stations include degenerate segments.
pub(crate) fn visit_grades(points: &[Vector3], mut visit: impl FnMut(f32, f32, f32, f32)) {
    let mut station = 0.0;
    let mut previous: Option<(f32, f32)> = None;
    for pair in points.windows(2) {
        let delta = pair[1] - pair[0];
        let run = delta.x.hypot(delta.z);
        let length = run.hypot(delta.y);
        if run < MIN_SEGMENT_RUN_M {
            previous = None;
            visit(station, length, f32::NAN, 0.0);
        } else {
            let grade = delta.y / run;
            let change = previous.map_or(0.0, |(prev_run, prev_grade)| {
                (grade - prev_grade).abs() / (0.5 * (prev_run + run))
            });
            visit(station, length, grade, change);
            previous = Some((run, grade));
        }
        station += length;
    }
}

/// Speed-radius design relationship in metres, with speed supplied in m/s.
pub(crate) fn design_radius(speed_ms: f32, lateral_friction: f32) -> f32 {
    let speed_kmh = speed_ms * MS_TO_KMH;
    speed_kmh * speed_kmh / (SPEED_RADIUS_CONVERSION * lateral_friction)
}

// Smallest radius of the centreline's turning, as `(radius, rounding, station)`, where the
// radius over a stretch is its horizontal length over its net signed turn, each chord of a
// flattened arc read as its arc (`visit_turning_radii`). A polyline corner has no radius of its
// own, and a densely sampled one would read as a tiny radius vertex by vertex, so each vertex
// measures the turning across a window of `window_m` of road starting half a segment before it:
// a hairpin turns back on itself inside the window, a corner of a straight-tool road does not.
// Segments shorter than `MIN_SEGMENT_RUN_M` end a window. The window kept is the one whose
// reading plus rounding bound is smallest: only what the stored `f32` points prove too tight is
// illegal. O(points * window segments), no allocation.
fn smallest_turning_radius(points: &[Vector3], window_m: f32) -> (f32, f32, f32) {
    let mut smallest = (f32::INFINITY, 0.0, 0.0);
    visit_turning_radii(
        points,
        |_| window_m,
        |station, radius, rounding, _, _| {
            if radius + rounding < smallest.0 + smallest.1 {
                smallest = (radius, rounding, station);
            }
        },
    );
    smallest
}

// A chord is read as an arc's only while its sag on that arc stays within twice the guide
// kernel's flattening sag (`c * sin(phi / 2) <= 4 * sag` is the kernel's own step rule
// `c <= sqrt(8 R sag)` for the circle through the chord with the turn `phi`; the factor two
// keeps a kernel chord at exactly the rule's step an arc chord through `f32` rounding). A longer
// or sharper chord is a corner's leg and keeps its chord length.
const ARC_CHORD_SAG_LIMIT_M: f64 = 2.0 * crate::simulation::network::guide::FLATTEN_MAX_SAG_M;

/// Visits vertex station, window radius, its rounding bound and signed vertex turn (positive
/// toward +Z from +X). A window's radius is its horizontal length over its net turn, ending at
/// degenerate segments, with each chord of a flattened arc counted as the arc it stands for, so
/// a polyline inscribed in a circle reads the circle's radius exactly: a chord turning `phi`
/// over its own length (from the turns at its two ends: `phi` at both inside an arc, `phi / 2`
/// where a tangent lead meets the arc and `phi` at the next vertex, the one turn it has at a
/// polyline end) counts `run * (phi / 2) / sin(phi / 2)`. A chord meeting a straight neighbour
/// (no turn at that end), a turn reversal, or a chord too long or too sharp for a flattened arc
/// (`ARC_CHORD_SAG_LIMIT_M`) counts its run, so a straight-tool corner reads as its chords
/// exactly as before. The rounding bound covers the stored `f32` points (half an ulp of the
/// window's largest coordinate per axis, through the two end headings and every chord); the
/// arithmetic is `f64`. The final callback argument gives the measured three-dimensional
/// station interval for a complete window, or `None` when the chain/degenerate segment ends
/// before the requested horizontal run. O(points * window segments), no allocation; the caller
/// supplies each station's window size.
pub(crate) fn visit_turning_radii(
    points: &[Vector3],
    mut window: impl FnMut(f32) -> f32,
    mut visit: impl FnMut(f32, f32, f32, f32, Option<(f32, f32)>),
) {
    let min_run_m = f64::from(MIN_SEGMENT_RUN_M);
    // Heading and horizontal run of segment `i`, `None` when it is degenerate or absent.
    let segment = |i: usize| {
        if i + 1 >= points.len() {
            return None;
        }
        let dx = f64::from(points[i + 1].x) - f64::from(points[i].x);
        let dz = f64::from(points[i + 1].z) - f64::from(points[i].z);
        let run_m = dx.hypot(dz);
        (run_m >= min_run_m).then(|| (dz.atan2(dx), run_m))
    };
    let turn_between = |from: f64, to: f64| {
        (to - from + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
    };
    // A segment's horizontal run read as the arc it flattens (see above) from the turns at its
    // start and end (`None` at a polyline end or a degenerate neighbour), with the factor's slope
    // `d/dphi ((phi / 2) / sin(phi / 2))` for the rounding bound.
    let arc_run = |run_m: f64, turn_in: Option<f64>, turn_out: Option<f64>| {
        let phi = match (turn_in, turn_out) {
            (Some(a), Some(b)) if a * b > 0.0 => {
                let (a, b) = (a.abs(), b.abs());
                (2.0 * a.min(b)).min(a.max(b))
            }
            (Some(a), None) | (None, Some(a)) => a.abs(),
            _ => 0.0,
        };
        let half = 0.5 * phi;
        if half > 0.0 && run_m * half.sin() <= 4.0 * ARC_CHORD_SAG_LIMIT_M {
            let (sin, cos) = half.sin_cos();
            (run_m * half / sin, 0.5 * (sin - half * cos) / (sin * sin))
        } else {
            (run_m, 0.0)
        }
    };
    let magnitude = |i: usize| f64::from(points[i].x.abs().max(points[i].z.abs()));
    // Station of the vertex after segment `first`, in three-dimensional metres.
    let mut vertex_station_m = 0.0;
    for first in 0..points.len().saturating_sub(2) {
        let first_length_m = points[first].distance_to(points[first + 1]);
        vertex_station_m += first_length_m;
        let Some((mut heading, first_run_m)) = segment(first) else {
            continue;
        };
        let window_m = f64::from(window(vertex_station_m));
        let mut vertex_turn = 0.0;
        // One segment of lookahead: each step reads one new segment.
        let mut ahead = segment(first + 1);
        let first_in = first
            .checked_sub(1)
            .and_then(segment)
            .map(|(previous, _)| turn_between(previous, heading));
        let first_out = ahead.map(|(next, _)| turn_between(heading, next));
        let (first_arc_m, first_slope) = arc_run(first_run_m, first_in, first_out);
        let (mut arc_m, mut turn) = (0.5 * first_arc_m, 0.0f64);
        // For the rounding bound: the largest coordinate, the turning and the factors' slopes.
        let mut largest = magnitude(first).max(magnitude(first + 1));
        let (mut turning, mut slopes, mut last_run_m) = (0.0, first_slope, first_run_m);
        let window_start_m = vertex_station_m - 0.5 * first_length_m;
        let mut window_end_m = vertex_station_m;
        let mut next = first + 1;
        while next + 1 < points.len() {
            let Some((next_heading, run_m)) = ahead else {
                break;
            };
            ahead = segment(next + 1);
            let step = turn_between(heading, next_heading);
            if next == first + 1 {
                vertex_turn = step as f32;
            }
            turn += step;
            heading = next_heading;
            let turn_out = ahead.map(|(after, _)| turn_between(next_heading, after));
            let (arc_run_m, slope) = arc_run(run_m, Some(step), turn_out);
            largest = largest.max(magnitude(next + 1));
            (turning, slopes, last_run_m) = (turning + step.abs(), slopes + slope, run_m);
            if arc_m + 0.5 * arc_run_m >= window_m {
                arc_m += 0.5 * arc_run_m;
                window_end_m += 0.5 * points[next].distance_to(points[next + 1]);
                break;
            }
            arc_m += arc_run_m;
            window_end_m += points[next].distance_to(points[next + 1]);
            next += 1;
        }
        let (radius, rounding) = if turn.abs() > 0.0 {
            // Each stored point lies within `r = u / sqrt(2)` of its true place (half an ulp `u`
            // of the window's largest coordinate on each axis), to first order: the net turn
            // depends on the first and last headings only, each off by at most `2 r / run`; the
            // chord sum moves by at most `r` at each end of the window and `r |turn|` at each
            // vertex between (`|t_in - t_out| = 2 sin(|turn| / 2)`), times the largest arc factor
            // (under 1.6); each chord's factor moves by its slope times its turn's error
            // (`4 r / run`), so by `4 r slope` over its run. `f64` arithmetic and the final `f32`
            // radius add a relative `f32::EPSILON`.
            let r = largest * f64::from(f32::EPSILON) * std::f64::consts::FRAC_1_SQRT_2;
            let arc_bound_m = r * (1.6 * (2.0 + turning) + 4.0 * slopes);
            let turn_bound = 2.0 * r * (1.0 / first_run_m + 1.0 / last_run_m);
            let radius = arc_m / turn.abs();
            let rounding = (arc_bound_m + radius * turn_bound) / turn.abs();
            (radius, rounding + radius * f64::from(f32::EPSILON))
        } else {
            (f64::INFINITY, 0.0)
        };
        visit(
            vertex_station_m,
            radius as f32,
            rounding as f32,
            vertex_turn,
            (arc_m >= window_m).then_some((window_start_m, window_end_m)),
        );
    }
}

// An edge's horizontal direction from the node: its centreline over the first metre
// (`ARM_DIRECTION_RUN_M`, or its whole length when shorter), as `[x, z]`; `None` under
// `MIN_SEGMENT_RUN_M`.
fn outgoing_direction(edge: &Edge, at_start: bool) -> Option<[f32; 2]> {
    let points = centreline(edge);
    let &origin = if at_start {
        points.first()?
    } else {
        points.last()?
    };
    let mut endpoint = origin;
    let mut remaining_m = ARM_DIRECTION_RUN_M;
    for offset in 1..points.len() {
        let point = points[if at_start {
            offset
        } else {
            points.len() - 1 - offset
        }];
        let delta = point - endpoint;
        let run_m = delta.x.hypot(delta.z);
        if run_m >= remaining_m {
            endpoint += delta * (remaining_m / run_m);
            break;
        }
        remaining_m -= run_m;
        endpoint = point;
    }
    let delta = endpoint - origin;
    (delta.x.hypot(delta.z) >= MIN_SEGMENT_RUN_M).then_some([delta.x, delta.z])
}

// The exact cross and dot products of two directions: each product of `f32` components is exact
// in `f64`, so each value is one rounding of the exact result, which a quarter turn or a mirror
// of both directions leaves unchanged (up to the cross product's sign under a mirror).
fn cross_dot(u: [f32; 2], v: [f32; 2]) -> (f64, f64) {
    let [ux, uz] = u.map(f64::from);
    let [vx, vz] = v.map(f64::from);
    (ux * vz - uz * vx, ux * vx + uz * vz)
}

// Counter-clockwise order in XZ from +x towards +z, the order of `atan2(z, x)` in `[0, 2π)`,
// exactly: the half-plane, then the sign of the exact cross product.
fn counter_clockwise(u: &[f32; 2], v: &[f32; 2]) -> std::cmp::Ordering {
    let lower = |[x, z]: [f32; 2]| z < 0.0 || (z == 0.0 && x < 0.0);
    lower(*u).cmp(&lower(*v)).then_with(|| {
        let (cross, _) = cross_dot(*u, *v);
        0.0.partial_cmp(&cross).unwrap_or(std::cmp::Ordering::Equal)
    })
}

// The angle from direction `u` counter-clockwise to `v`, in `[0, 2π)`, from their exact cross
// and dot products, so the same in every quarter turn and mirror of a junction, bit for bit
// (`ROAD-68-K23`). Each arm's own `atan2` and their difference round differently in each
// orientation, and so decided by orientation whether a Y at exactly the 25° limit was legal.
fn counter_clockwise_angle(u: [f32; 2], v: [f32; 2]) -> f64 {
    let (cross, dot) = cross_dot(u, v);
    let angle = cross.atan2(dot) + 0.0;
    if angle < 0.0 {
        angle + std::f64::consts::TAU
    } else {
        angle
    }
}

/// Appends degree-two kink or junction angle/degree violations for a canonical node.
///
/// Uses existing adjacency, ignores deleted/non-road edges and retired node aliases,
/// and samples each outgoing centreline over its first horizontal metre (or its
/// whole length when shorter). No clips are applied. Directionless edges still count
/// toward degree. O(PrefixPoints + degree log degree) time, O(degree) scratch space;
/// bounded endpoint sample density gives O(degree log degree). No per-point allocation.
pub fn validate_node(
    graph: &RegionGraph,
    node_id: u32,
    limits: &RoadGeometryLimits,
    output: &mut Vec<RoadGeometryViolation>,
) {
    if graph.get_valid_node(node_id) != node_id {
        return;
    }
    let Some(incident) = graph.adjacency().get(node_id as usize) else {
        return;
    };
    let mut degree = 0;
    let mut directions = Vec::with_capacity(incident.len());
    for &edge_id in incident {
        let edge = graph.edge(edge_id);
        if !is_road(edge) {
            continue;
        }
        degree += 1;
        if let Some(direction) =
            outgoing_direction(edge, graph.get_valid_node(edge.start_node) == node_id)
        {
            directions.push(direction);
        }
    }
    if degree == 2 && directions.len() == 2 {
        // The angle between the two directions, in `[0, π]`.
        let (cross, dot) = cross_dot(directions[0], directions[1]);
        let separation = cross.abs().atan2(dot);
        let kink = (std::f64::consts::PI - separation).to_degrees() as f32;
        if let Some((limit, tier)) = upper_violation(kink, &limits.direction_kink_degrees) {
            output.push(RoadGeometryViolation::DirectionKink {
                node_id,
                measured: kink,
                limit,
                tier,
            });
        }
    } else if degree >= 3 {
        if let Some((limit, tier)) = upper_violation(degree, &limits.junction_degree) {
            output.push(RoadGeometryViolation::JunctionDegree {
                node_id,
                measured: degree,
                limit,
                tier,
            });
        }
        if directions.len() >= 2 {
            // Adjacent arms counter-clockwise, the last to the first included.
            directions.sort_unstable_by(counter_clockwise);
            let last = directions[directions.len() - 1];
            let mut smallest = counter_clockwise_angle(last, directions[0]);
            for pair in directions.windows(2) {
                smallest = smallest.min(counter_clockwise_angle(pair[0], pair[1]));
            }
            let measured = smallest.to_degrees() as f32;
            if let Some((limit, tier)) =
                lower_violation(measured, &limits.junction_arm_angle_degrees)
            {
                output.push(RoadGeometryViolation::JunctionArmAngle {
                    node_id,
                    measured,
                    limit,
                    tier,
                });
            }
        }
    }
}

/// Audits graph IDs with Rayon and returns deterministic kind-then-ID ordering.
///
/// Uses worker-local output buffers, not one allocation per edge. Total work is
/// O(points + sum(degree log degree) + violations log violations), plus graph-slot
/// traversal and endpoint-prefix sampling. The graph and all derived state stay unchanged.
pub fn audit_graph(graph: &RegionGraph, limits: &RoadGeometryLimits) -> Vec<RoadGeometryViolation> {
    let merge = |mut a: Vec<_>, mut b: Vec<_>| {
        a.append(&mut b);
        a
    };
    let mut output = graph
        .edges()
        .par_iter()
        .enumerate()
        .fold(Vec::new, |mut out, (id, _)| {
            validate_edge(graph, id, limits, &mut out);
            out
        })
        .reduce(Vec::new, merge);
    let mut node_output = graph
        .nodes()
        .par_iter()
        .enumerate()
        .fold(Vec::new, |mut out, (id, _)| {
            validate_node(graph, id as u32, limits, &mut out);
            out
        })
        .reduce(Vec::new, merge);
    output.append(&mut node_output);
    output.sort_unstable_by_key(RoadGeometryViolation::sort_key);
    output
}

mod placement;
pub use placement::edit_violation;

#[cfg(test)]
mod tests;
