// SPDX-License-Identifier: GPL-2.0-only

//! Read-only road geometry measurements, independent of placement and surface compilation.
//!
//! Uses the renderer's physical centreline (control geometry only when physical geometry is
//! empty). Stations are unclipped three-dimensional arc lengths from the stored start.
//! Node directions use an unclipped one-metre horizontal prefix: clips reserve ownership,
//! rather than changing the alignment at the node. Road and foot edges are checked; other
//! transit modes have different design constraints and are excluded.

use super::graph::{Edge, RegionGraph};
use super::roadbed::visual_roadbed_half_width_m;
use super::types::{EdgeClass, TransitType};
use godot::prelude::Vector3;
use rayon::prelude::*;

// Ordinary-street design target supplied by the road validator specification.
const STANDARD_DESIGN_GRADE: f32 = 0.10;
// Bridge/tunnel design target supplied by the road validator specification.
const STRUCTURAL_DESIGN_GRADE: f32 = 0.06;
// Matches ROAD_PROFILE_MAX_GRADE, without the solver's terrain-slope relaxation.
const ILLEGAL_GRADE: f32 = 0.16;
// Matches ROAD_PROFILE_MAX_GRADE_CHANGE_PER_M in surface/edge/input.rs.
const DESIGN_GRADE_CHANGE_PER_M: f32 = 0.018;
// Hard vertical-curvature bound: the specification's 0.05 with the ~20% leeway placement allows
// over real-world maxima.
const ILLEGAL_GRADE_CHANGE_PER_M: f32 = 0.06;
// Safety margin above the offset-curve folding radius (the roadbed half-width).
const ILLEGAL_RADIUS_HALF_WIDTH_FACTOR: f32 = 1.25;
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
                illegal: ILLEGAL_GRADE,
                design: STANDARD_DESIGN_GRADE,
            },
            bridge_grade: RoadGeometryThreshold {
                illegal: ILLEGAL_GRADE,
                design: STRUCTURAL_DESIGN_GRADE,
            },
            tunnel_grade: RoadGeometryThreshold {
                illegal: ILLEGAL_GRADE,
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
    let mut worst_grade = (0.0, 0.0);
    let mut worst_change = (0.0, 0.0);
    let points = centreline(edge);
    let half_width_m = visual_roadbed_half_width_m(edge);
    let smallest_radius = smallest_turning_radius(points, RADIUS_WINDOW_HALF_WIDTHS * half_width_m);
    visit_grades(points, |station, length, grade, change| {
        if grade.abs() > worst_grade.0 {
            worst_grade = (grade.abs(), station + 0.5 * length);
        }
        if change > worst_change.0 {
            worst_change = (change, station);
        }
    });
    let grade_bounds = match edge.class {
        EdgeClass::Standard => &limits.standard_grade,
        EdgeClass::Bridge => &limits.bridge_grade,
        EdgeClass::Tunnel => &limits.tunnel_grade,
    };
    if let Some((limit, tier)) = upper_violation(worst_grade.0, grade_bounds) {
        output.push(RoadGeometryViolation::Grade {
            edge_id,
            station_m: worst_grade.1,
            measured: worst_grade.0,
            limit,
            tier,
        });
    }
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
    if let Some((limit, tier)) = lower_violation(smallest_radius.0, &radius_bounds) {
        output.push(RoadGeometryViolation::HorizontalRadius {
            edge_id,
            station_m: smallest_radius.1,
            measured: smallest_radius.0,
            limit,
            tier,
        });
    }
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

// Smallest radius of the centreline's turning, as `(radius, station)`, where the radius over a
// stretch is its horizontal length over its net signed turn. A polyline corner has no radius of
// its own, and a densely sampled one would read as a tiny radius vertex by vertex, so each vertex
// measures the turning across a window of `window_m` of road starting half a segment before it:
// a hairpin turns back on itself inside the window, a corner of a straight-tool road does not.
// Segments shorter than `MIN_SEGMENT_RUN_M` end a window. O(points * window segments), no
// allocation.
fn smallest_turning_radius(points: &[Vector3], window_m: f32) -> (f32, f32) {
    let mut smallest = (f32::INFINITY, 0.0);
    visit_turning_radii(
        points,
        |_| window_m,
        |station, radius, _, _| {
            if radius < smallest.0 {
                smallest = (radius, station);
            }
        },
    );
    smallest
}

/// Visits vertex station, window radius and signed vertex turn (positive toward +Z from +X).
/// Windows use horizontal length/net turn, ending at degenerate segments. The final callback
/// argument gives the measured three-dimensional station interval for a complete window, or
/// `None` when the chain/degenerate segment ends before the requested horizontal run.
/// O(points * window segments), no allocation; the caller supplies each station's window size.
pub(crate) fn visit_turning_radii(
    points: &[Vector3],
    mut window: impl FnMut(f32) -> f32,
    mut visit: impl FnMut(f32, f32, f32, Option<(f32, f32)>),
) {
    let segment = |i: usize| {
        let delta = points[i + 1] - points[i];
        let run_m = delta.x.hypot(delta.z);
        (run_m >= MIN_SEGMENT_RUN_M).then(|| (delta.z.atan2(delta.x), run_m))
    };
    // Station of the vertex after segment `first`, in three-dimensional metres.
    let mut vertex_station_m = 0.0;
    for first in 0..points.len().saturating_sub(2) {
        let first_length_m = points[first].distance_to(points[first + 1]);
        vertex_station_m += first_length_m;
        let Some((mut heading, first_run_m)) = segment(first) else {
            continue;
        };
        let window_m = window(vertex_station_m);
        let mut vertex_turn = 0.0;
        let (mut arc_m, mut turn) = (0.5 * first_run_m, 0.0f32);
        let window_start_m = vertex_station_m - 0.5 * first_length_m;
        let mut window_end_m = vertex_station_m;
        let mut next = first + 1;
        while next + 1 < points.len() {
            let Some((next_heading, run_m)) = segment(next) else {
                break;
            };
            let step = (next_heading - heading + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            if next == first + 1 {
                vertex_turn = step;
            }
            turn += step;
            heading = next_heading;
            if arc_m + 0.5 * run_m >= window_m {
                arc_m += 0.5 * run_m;
                window_end_m += 0.5 * points[next].distance_to(points[next + 1]);
                break;
            }
            arc_m += run_m;
            window_end_m += points[next].distance_to(points[next + 1]);
            next += 1;
        }
        let radius = if turn.abs() > 0.0 {
            arc_m / turn.abs()
        } else {
            f32::INFINITY
        };
        visit(
            vertex_station_m,
            radius,
            vertex_turn,
            (arc_m >= window_m).then_some((window_start_m, window_end_m)),
        );
    }
}

fn outgoing_angle(edge: &Edge, at_start: bool) -> Option<f32> {
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
    (delta.x.hypot(delta.z) >= MIN_SEGMENT_RUN_M)
        .then(|| delta.z.atan2(delta.x).rem_euclid(std::f32::consts::TAU))
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
    let mut angles = Vec::with_capacity(incident.len());
    for &edge_id in incident {
        let edge = graph.edge(edge_id);
        if !is_road(edge) {
            continue;
        }
        degree += 1;
        if let Some(angle) = outgoing_angle(edge, graph.get_valid_node(edge.start_node) == node_id)
        {
            angles.push(angle);
        }
    }
    if degree == 2 && angles.len() == 2 {
        let gap = (angles[0] - angles[1]).abs();
        let separation = gap.min(std::f32::consts::TAU - gap);
        let kink = (std::f32::consts::PI - separation).to_degrees();
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
        if angles.len() >= 2 {
            angles.sort_unstable_by(f32::total_cmp);
            let mut smallest = std::f32::consts::TAU - angles[angles.len() - 1] + angles[0];
            for pair in angles.windows(2) {
                smallest = smallest.min(pair[1] - pair[0]);
            }
            let measured = smallest.to_degrees();
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
