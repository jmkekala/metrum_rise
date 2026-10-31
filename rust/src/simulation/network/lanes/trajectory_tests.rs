// SPDX-License-Identifier: GPL-2.0-only

//! Junction connector trajectory invariants (`TRAFFIC-12`) on the compiled roadbed: every vehicle
//! connector stays on the carriageway, never steps backwards against its chord, never
//! counter-steers and has no kink, and every lane meets its arm's mouth frame. The measurement
//! ([`measure_connectors`]) also serves the save sweep (`nodes::sim::core::tests::
//! junction_connector_sweep`), which runs it on every node of a loaded city.

use super::super::graph::RegionGraph;
use super::super::roadbed::fixtures::{finalize, footpath_fixtures, point, road_edge};
use super::super::roadbed::{
    ArmKey, BandKind, EdgeEnd, MouthFrame, NodeKind, NodeWalkways, RoadVec2, RoadbedSystem,
};
use super::super::types::NodeType;
use super::vehicle_junctions::{TurnKind, VehicleTurn, vehicle_lane_end, vehicle_mouth_frame};
use super::*;
use crate::config;
use std::collections::BTreeMap;
use std::fmt::Write;

// Points closer than this to the carriageway count as on it (f32 lane geometry against f64
// triangles).
const OUTSIDE_TOLERANCE_M: f32 = 0.01;
// Connector polylines are checked at this spacing as well as at their own vertices.
const PROBE_STEP_M: f32 = 0.25;
// A turn of more than this between consecutive connector segments, or between a connector and
// its lanes, is a kink. Arcs are sampled at 0.15 rad at most.
const MAX_STEP_TURN_RAD: f32 = 0.2;
// Lane ends lie on their mouth lines to this distance (f32 lane points of f64 frames).
const MOUTH_LINE_TOLERANCE_M: f32 = 0.01;
// A lane's end piece runs along its arm's direction at the mouth to this angle.
const END_DIRECTION_TOLERANCE_RAD: f32 = 0.05;
// `dev`'s connector builder fell back to `+x` for an end piece shorter than this (its squared
// length at most 1e-5).
const DEV_FALLBACK_PIECE_M: f32 = 0.003_162;
// Directions are taken over at least this distance: f32 points a few kilometres from the
// origin are a few tenths of a millimetre apart, which turns a millimetre piece by a tenth of a
// radian.
const DIRECTION_PIECE_M: f32 = 0.01;
// A turn against the connector's turning sense by more than this is a counter-steer, or by more
// than the f32 resolution of its two steps allows: a point `x` metres from the origin is rounded
// by up to `x * f32::EPSILON / 2`, so a step of length `l` turns by up to about
// `2 x f32::EPSILON / l` (2 mrad for 1 m steps 8.5 km out, as in the demo save).
const COUNTERSTEER_RAD: f32 = 1.0e-3;

fn flat(p: Vector3) -> Vector2 {
    Vector2::new(p.x, p.z)
}

fn flat64(v: RoadVec2) -> Vector2 {
    Vector2::new(v.x as f32, v.y as f32)
}

// Unit XZ direction from the first of `points` to the first one farther than
// `DIRECTION_PIECE_M` from it.
fn first_direction<'a>(mut points: impl Iterator<Item = &'a Vector3>) -> Option<Vector2> {
    let first = flat(*points.next()?);
    points
        .map(|&p| flat(p) - first)
        .find(|d| d.length() > DIRECTION_PIECE_M)
        .map(Vector2::normalized)
}

// Unit XZ directions along `points`, each over at least `DIRECTION_PIECE_M`, with the length
// it spans.
fn directions_along(points: &[Vector3], out: &mut Vec<(Vector2, f32)>) {
    let mut anchor = flat(points[0]);
    for &p in &points[1..] {
        let d = flat(p) - anchor;
        if d.length() > DIRECTION_PIECE_M {
            out.push((d.normalized(), d.length()));
            anchor = flat(p);
        }
    }
}

fn cross(a: Vector2, b: Vector2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn angle_between(a: Vector2, b: Vector2) -> f32 {
    cross(a, b).atan2(a.dot(b)).abs()
}

// Distance from `p` to the nearest carriageway triangle, to 1 mm, capped at 8 m.
fn outside_carriageway_m(roadbed: &RoadbedSystem, p: Vector2) -> f32 {
    if roadbed.carriageway_height_at(p.x, p.y).is_some() {
        return 0.0;
    }
    let center = RoadVec2::new(f64::from(p.x), f64::from(p.y));
    let near = |radius: f64| {
        roadbed.any_band_near_disc(center, radius, |kind| kind == BandKind::Carriageway)
    };
    let (mut low, mut high) = (0.0, 8.0);
    if !near(high) {
        return high as f32;
    }
    while high - low > 0.001 {
        let mid = 0.5 * (low + high);
        if near(mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
    high as f32
}

// Farthest probe of `points` outside the carriageway.
fn worst_outside_m(roadbed: &RoadbedSystem, points: &[Vector3]) -> f32 {
    let mut worst = 0.0_f32;
    for pair in points.windows(2) {
        let (a, b) = (flat(pair[0]), flat(pair[1]));
        let probes = ((a.distance_to(b) / PROBE_STEP_M).ceil() as usize).max(1);
        for k in 0..=probes {
            let p = a.lerp(b, k as f32 / probes as f32);
            worst = worst.max(outside_carriageway_m(roadbed, p));
        }
    }
    worst
}

/// Shape faults of one connector polyline between its lanes' directions.
#[derive(Clone, Copy, Default)]
pub(crate) struct Shape {
    pub(crate) outside_m: f32,
    pub(crate) backward: bool,
    pub(crate) countersteer: bool,
    pub(crate) kink: bool,
    pub(crate) max_step_rad: f32,
}

impl Shape {
    fn outside(&self) -> bool {
        self.outside_m > OUTSIDE_TOLERANCE_M
    }

    fn failed(&self) -> bool {
        self.outside() || self.backward || self.countersteer || self.kink
    }
}

// `points` entered along `incoming` and left along `outgoing` (the lanes' own end pieces). A turn
// whose arm directions (`frame_in`, `frame_out`) meet ahead of both ends is a single turn, which
// must not counter-steer; elsewhere an S is the only smooth path. U-turns are checked for leaving
// the carriageway and kinks only.
#[allow(clippy::too_many_arguments)]
fn shape(
    roadbed: &RoadbedSystem,
    points: &[Vector3],
    incoming: Vector2,
    outgoing: Vector2,
    frame_in: Vector2,
    frame_out: Vector2,
    u_turn: bool,
) -> Shape {
    let mut shape = Shape {
        outside_m: worst_outside_m(roadbed, points),
        ..Shape::default()
    };
    let start = flat(points[0]);
    let end = flat(*points.last().unwrap());
    let chord_m = start.distance_to(end);
    let mut directions = Vec::with_capacity(points.len() + 1);
    // The lanes' directions span their whole end pieces: take them as exact.
    directions.push((incoming, f32::INFINITY));
    directions_along(points, &mut directions);
    directions.push((outgoing, f32::INFINITY));
    let reach = points
        .iter()
        .map(|p| p.x.abs().max(p.z.abs()))
        .fold(0.0, f32::max);
    let turn = cross(frame_in, frame_out);
    let single_turn = !u_turn && turn.abs() > 0.05 && meets_ahead(start, frame_in, end, frame_out);
    if !u_turn && chord_m > 1.0e-3 {
        // A step is backward when it heads against the chord, or, in a turn sharper than a right
        // angle, whose own exit heads against the chord, against both lane directions.
        let chord = (end - start) / chord_m;
        let sharp = frame_in.dot(frame_out) < 0.0;
        shape.backward = directions[1..directions.len() - 1].iter().any(|(d, _)| {
            if sharp {
                d.dot(frame_in) < 0.0 && d.dot(frame_out) < 0.0
            } else {
                d.dot(chord) <= 0.0
            }
        });
    }
    for pair in directions.windows(2) {
        let ((a, a_m), (b, b_m)) = (pair[0], pair[1]);
        let signed = cross(a, b);
        let resolution = 2.0 * reach * f32::EPSILON * (1.0 / a_m + 1.0 / b_m);
        if single_turn && signed * turn.signum() < -COUNTERSTEER_RAD.max(resolution) {
            shape.countersteer = true;
        }
        let step = signed.atan2(a.dot(b)).abs();
        shape.max_step_rad = shape.max_step_rad.max(step);
        if step > MAX_STEP_TURN_RAD {
            shape.kink = true;
        }
    }
    shape
}

// Whether the line from `start` along `incoming` and the line into `end` along `outgoing` meet
// ahead of `start` and behind `end`.
fn meets_ahead(start: Vector2, incoming: Vector2, end: Vector2, outgoing: Vector2) -> bool {
    let sine = cross(incoming, outgoing);
    if sine.abs() <= 1.0e-6 {
        return false;
    }
    let delta = end - start;
    cross(delta, outgoing) / sine > 0.0 && cross(incoming, delta) / sine > 0.0
}

/// One lane end at a node against its arm's mouth frame.
#[derive(Clone, Copy)]
struct EndCheck {
    // Length of the lane's piece at the node.
    piece_m: f32,
    // Angle between that piece and the arm's direction at the mouth.
    angle_rad: f32,
    // Distance from the lane's end point to the mouth line, and to the lane's frame point.
    line_m: f32,
    point_m: f32,
}

// The lane end of `lane` (on `edge`'s `end`) at the node, against the mouth frame.
fn end_check(lane: &Lane, end_at_front: bool, frame_point: Vector3, outward: Vector2) -> EndCheck {
    let g = &lane.geometry;
    let (tip, next) = if end_at_front {
        (g[0], g[1])
    } else {
        (g[g.len() - 1], g[g.len() - 2])
    };
    let piece = flat(next) - flat(tip);
    let angle_rad = if piece.length() > 0.0 {
        angle_between(piece.normalized(), outward)
    } else {
        std::f32::consts::PI
    };
    EndCheck {
        piece_m: piece.length(),
        angle_rad,
        line_m: (flat(tip) - flat(frame_point)).dot(outward).abs(),
        point_m: flat(tip).distance_to(flat(frame_point)),
    }
}

/// Counts for one set of connectors.
#[derive(Default)]
pub(crate) struct Counts {
    pub(crate) connectors: usize,
    pub(crate) u_turns: usize,
    pub(crate) outside: usize,
    pub(crate) max_outside_m: f32,
    pub(crate) backward: usize,
    pub(crate) countersteer: usize,
    pub(crate) kinks: usize,
    pub(crate) max_step_turn_rad: f32,
    pub(crate) failed: usize,
    // Failing connectors by cause.
    pub(crate) causes: BTreeMap<&'static str, usize>,
    pub(crate) worst: String,
    // Lane ends at the measured nodes.
    pub(crate) lane_ends: usize,
    pub(crate) end_line_misses: usize,
    pub(crate) max_end_line_m: f32,
    pub(crate) max_end_point_m: f32,
    pub(crate) end_angle_misses: usize,
    pub(crate) max_end_angle_rad: f32,
    pub(crate) short_end_pieces: usize,
    pub(crate) min_end_piece_m: f32,
    // The frame-built curve evaluated on the same ends (Phase A's prediction of the rebuild).
    pub(crate) curve_failed: usize,
    pub(crate) curve_outside: usize,
    pub(crate) curve_max_outside_m: f32,
    pub(crate) curve_backward: usize,
    pub(crate) curve_countersteer: usize,
    pub(crate) curve_kinks: usize,
    pub(crate) curve_biarcs: usize,
    pub(crate) curve_none: usize,
    pub(crate) curve_kerb_violations: usize,
    pub(crate) curve_min_radius_m: f32,
    pub(crate) curve_min_radius_at: String,
    pub(crate) curve_worst: String,
    pub(crate) lines: Vec<String>,
}

impl Counts {
    fn clean(&self) -> bool {
        self.outside + self.backward + self.countersteer + self.kinks == 0
            && self.end_line_misses + self.end_angle_misses == 0
    }

    pub(crate) fn report(&self, name: &str) -> String {
        let mut out = String::new();
        writeln!(
            out,
            "{name}: {} connectors ({} U-turns), {} failing; {} outside (max {:.2} m, {}), \
             {} backward, {} countersteer, {} kinks (max step {:.3} rad)",
            self.connectors,
            self.u_turns,
            self.failed,
            self.outside,
            self.max_outside_m,
            self.worst,
            self.backward,
            self.countersteer,
            self.kinks,
            self.max_step_turn_rad,
        )
        .unwrap();
        writeln!(out, "  causes of failing connectors: {:?}", self.causes).unwrap();
        writeln!(
            out,
            "  lane ends: {}; off the mouth line {} (max {:.4} m, max from frame point {:.4} m); \
             direction off the arm {} (max {:.4} rad); end pieces under {DEV_FALLBACK_PIECE_M} m \
             {} (min {:.6} m)",
            self.lane_ends,
            self.end_line_misses,
            self.max_end_line_m,
            self.max_end_point_m,
            self.end_angle_misses,
            self.max_end_angle_rad,
            self.short_end_pieces,
            self.min_end_piece_m,
        )
        .unwrap();
        writeln!(
            out,
            "  frame-built curve: {} failing; {} outside (max {:.2} m, {}), {} backward, {} \
             countersteer, {} kinks; {} biarcs, {} without a construction, {} kerb violations, \
             min radius {:.2} m ({})",
            self.curve_failed,
            self.curve_outside,
            self.curve_max_outside_m,
            self.curve_worst,
            self.curve_backward,
            self.curve_countersteer,
            self.curve_kinks,
            self.curve_biarcs,
            self.curve_none,
            self.curve_kerb_violations,
            self.curve_min_radius_m,
            self.curve_min_radius_at,
        )
        .unwrap();
        out
    }
}

// The edge end at `lane`'s front (`at_front`) or back: a forward lane's front lies at its edge's
// start, a backward lane's at its end.
fn lane_end_at(lane: &Lane, at_front: bool) -> EdgeEnd {
    if lane.is_fwd == at_front {
        EdgeEnd::Start
    } else {
        EdgeEnd::End
    }
}

/// Measures every vehicle connector at each of `nodes` and every vehicle lane end there against
/// its mouth frame, and evaluates the frame-built curve on the same ends. `lines` collects one
/// line per failing connector when `verbose`.
pub(crate) fn measure_connectors(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    lanes: &LaneSystem,
    nodes: impl IntoIterator<Item = usize>,
    verbose: bool,
) -> Counts {
    let mut counts = Counts {
        min_end_piece_m: f32::INFINITY,
        curve_min_radius_m: f32::INFINITY,
        ..Counts::default()
    };
    // Each connector's source lane.
    let mut source_of = HashMap::new();
    for (id, lane) in lanes.lanes.iter().enumerate() {
        if lane.edge_id != usize::MAX && lane.lane_type == LaneType::Vehicle {
            for &next in &lane.next_lanes {
                source_of.insert(next, id);
            }
        }
    }
    for node in nodes {
        let Some(connectors) = lanes.node_lanes.get(&node) else {
            continue;
        };
        let (_, kerbs) = NodeWalkways::collect_with_kerbs(graph, node as u32);
        let kerbs = kerbs.unwrap_or_default();
        // The compiled roadbed's own mouth of arm `key` here (zero at a terminal or without a
        // node piece): the reference the lane ends are measured against.
        let piece = roadbed.node_piece(node as u32);
        let mouth = |key: ArmKey| {
            let station_m = piece
                .filter(|piece| piece.layout().kind != NodeKind::Terminal)
                .and_then(|piece| piece.layout().arms.iter().find(|arm| arm.key == key))
                .map_or(0.0, |arm| arm.mouth_station_m);
            MouthFrame::at(graph, node as u32, key, station_m)
        };
        // Lane ends at this node, once each.
        let mut checked = Vec::new();
        for &connector_id in connectors {
            let connector = &lanes.lanes[connector_id];
            if connector.lane_type != LaneType::Vehicle || connector.geometry.len() < 2 {
                continue;
            }
            let Some(&source_id) = source_of.get(&connector_id) else {
                continue;
            };
            let source = &lanes.lanes[source_id];
            let target_id = connector.next_lanes[0];
            let target = &lanes.lanes[target_id];
            let source_end = lane_end_at(source, false);
            let target_end = lane_end_at(target, true);
            let key = |lane: &Lane, end| ArmKey {
                edge: lane.edge_id,
                end,
            };
            let (Some(frame_in), Some(frame_out), Some(mouth_in), Some(mouth_out)) = (
                vehicle_mouth_frame(graph, source.edge_id, source_end),
                vehicle_mouth_frame(graph, target.edge_id, target_end),
                mouth(key(source, source_end)),
                mouth(key(target, target_end)),
            ) else {
                continue;
            };
            let end_in = vehicle_lane_end(&frame_in, source_end, source.lane_idx);
            let end_out = vehicle_lane_end(&frame_out, target_end, target.lane_idx);
            let (out_in, out_out) = (flat64(end_in.outward), flat64(end_out.outward));
            let reference_in = vehicle_lane_end(&mouth_in, source_end, source.lane_idx);
            let reference_out = vehicle_lane_end(&mouth_out, target_end, target.lane_idx);
            let check_in = end_check(
                source,
                false,
                reference_in.point,
                flat64(reference_in.outward),
            );
            let check_out = end_check(
                target,
                true,
                reference_out.point,
                flat64(reference_out.outward),
            );
            for (id, check) in [(source_id, check_in), (target_id, check_out)] {
                if checked.contains(&id) {
                    continue;
                }
                checked.push(id);
                counts.lane_ends += 1;
                counts.end_line_misses += usize::from(check.line_m > MOUTH_LINE_TOLERANCE_M);
                counts.max_end_line_m = counts.max_end_line_m.max(check.line_m);
                counts.max_end_point_m = counts.max_end_point_m.max(check.point_m);
                counts.end_angle_misses +=
                    usize::from(check.angle_rad > END_DIRECTION_TOLERANCE_RAD);
                counts.max_end_angle_rad = counts.max_end_angle_rad.max(check.angle_rad);
                counts.short_end_pieces += usize::from(check.piece_m < DEV_FALLBACK_PIECE_M);
                counts.min_end_piece_m = counts.min_end_piece_m.min(check.piece_m);
            }

            let u_turn = target.edge_id == source.edge_id;
            counts.connectors += 1;
            counts.u_turns += usize::from(u_turn);
            let label = format!(
                "node {node} lanes {source_id} -> {target_id} (edges {} -> {})",
                source.edge_id, target.edge_id
            );
            // The lanes' own end pieces, as cars drive them.
            let incoming = first_direction(source.geometry.iter().rev()).map(|d| -d);
            let outgoing = first_direction(target.geometry.iter());
            let (Some(incoming), Some(outgoing)) = (incoming, outgoing) else {
                continue;
            };
            let built = shape(
                roadbed,
                &connector.geometry,
                incoming,
                outgoing,
                -out_in,
                out_out,
                u_turn,
            );
            counts.outside += usize::from(built.outside());
            if built.outside_m > counts.max_outside_m {
                counts.max_outside_m = built.outside_m;
                counts.worst = label.clone();
            }
            counts.backward += usize::from(built.backward);
            counts.countersteer += usize::from(built.countersteer);
            counts.kinks += usize::from(built.kink);
            counts.max_step_turn_rad = counts.max_step_turn_rad.max(built.max_step_rad);
            if built.failed() {
                counts.failed += 1;
                let short = check_in.piece_m < DEV_FALLBACK_PIECE_M
                    || check_out.piece_m < DEV_FALLBACK_PIECE_M;
                let off_angle = check_in.angle_rad > END_DIRECTION_TOLERANCE_RAD
                    || check_out.angle_rad > END_DIRECTION_TOLERANCE_RAD;
                let off_line = check_in.line_m > MOUTH_LINE_TOLERANCE_M
                    || check_out.line_m > MOUTH_LINE_TOLERANCE_M;
                let cause = if short {
                    "lane end piece under 3.2 mm (+x fallback)"
                } else if off_angle {
                    "lane end piece off the arm direction"
                } else if off_line {
                    "lane end off the mouth line"
                } else if piece.is_none() {
                    "curve at a node without a layout"
                } else {
                    "curve (junction-span cubic) with good lane ends"
                };
                *counts.causes.entry(cause).or_default() += 1;
                if verbose {
                    counts.lines.push(format!(
                        "BUILT {label}: {cause}; outside {:.2} m, backward {}, countersteer {}, \
                         kink {} (step {:.3}); ends in piece {:.4} m angle {:.3} line {:.4}, out \
                         piece {:.4} m angle {:.3} line {:.4}",
                        built.outside_m,
                        built.backward,
                        built.countersteer,
                        built.kink,
                        built.max_step_rad,
                        check_in.piece_m,
                        check_in.angle_rad,
                        check_in.line_m,
                        check_out.piece_m,
                        check_out.angle_rad,
                        check_out.line_m,
                    ));
                }
            }

            // The frame-built curve on the same ends.
            let as64 = |p: Vector3| RoadVec2::new(f64::from(p.x), f64::from(p.z));
            let Some(turn) = VehicleTurn::new(
                as64(end_in.point),
                -end_in.outward,
                as64(end_out.point),
                end_out.outward,
                &kerbs,
            ) else {
                counts.curve_none += 1;
                counts.curve_failed += 1;
                if verbose {
                    counts.lines.push(format!("CURVE {label}: no construction"));
                }
                continue;
            };
            counts.curve_biarcs += usize::from(turn.kind == TurnKind::Biarc);
            counts.curve_kerb_violations += usize::from(turn.kerb_margin_m(&kerbs) < 0.0);
            if !u_turn && (turn.min_radius_m() as f32) < counts.curve_min_radius_m {
                counts.curve_min_radius_m = turn.min_radius_m() as f32;
                counts.curve_min_radius_at = format!("{label}, {:?}", turn.kind);
            }
            let points = turn.polyline(end_in.point, end_out.point);
            if std::env::var("METRUM_SWEEP_DUMP").is_ok_and(|dump| dump == label) {
                eprintln!(
                    "DUMP {label}: frame in {end_in:?} out {end_out:?}\n built {:?}\n curve {points:?}\n source tail {:?}\n target head {:?}",
                    connector.geometry,
                    &source.geometry[source.geometry.len().saturating_sub(4)..],
                    &target.geometry[..target.geometry.len().min(4)]
                );
            }
            let curve = shape(roadbed, &points, -out_in, out_out, -out_in, out_out, u_turn);
            counts.curve_outside += usize::from(curve.outside());
            if curve.outside_m > counts.curve_max_outside_m {
                counts.curve_max_outside_m = curve.outside_m;
                counts.curve_worst = label.clone();
            }
            counts.curve_backward += usize::from(curve.backward);
            counts.curve_countersteer += usize::from(curve.countersteer);
            counts.curve_kinks += usize::from(curve.kink);
            if curve.failed() {
                counts.curve_failed += 1;
                if verbose {
                    counts.lines.push(format!(
                        "CURVE {label}: {:?}, outside {:.2} m, backward {}, countersteer {}, \
                         kink {}, radius {:.2} m, kerb margin {:.2} m",
                        turn.kind,
                        curve.outside_m,
                        curve.backward,
                        curve.countersteer,
                        curve.kink,
                        turn.min_radius_m(),
                        turn.kerb_margin_m(&kerbs),
                    ));
                }
            }
        }
    }
    counts
}

// A node at the origin with one arm per `(angle in degrees, lanes per direction)`, each 80 m to a
// road end, the reference screenshot fixtures' arm length. Even arms leave the hub from their
// edge's start, odd ones from its end.
fn star(arms: &[(f32, u8)]) -> RegionGraph {
    let mut graph = RegionGraph::new();
    let hub = graph.add_node(point(0.0, 0.0, false), NodeType::Junction);
    for (index, &(angle_deg, lanes)) in arms.iter().enumerate() {
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let far = graph.add_node(point(cos * 80.0, sin * 80.0, false), NodeType::Junction);
        let (a, b) = if index % 2 == 0 {
            (hub, far)
        } else {
            (far, hub)
        };
        let (pa, pb) = (graph.node(a).pos, graph.node(b).pos);
        let mut edge = road_edge(
            a,
            b,
            vec![pa, pb],
            f32::from(lanes) * 2.0 * config::LANE_WIDTH,
        );
        edge.fwd_lanes = lanes;
        edge.bkw_lanes = lanes;
        graph.add_edge(edge);
    }
    finalize(&mut graph);
    graph
}

fn fixtures() -> Vec<(String, RegionGraph)> {
    let mut fixtures = vec![
        (
            "four_way_1x1".to_string(),
            star(&[(0.0, 1), (90.0, 1), (180.0, 1), (270.0, 1)]),
        ),
        (
            "t_1x1".to_string(),
            star(&[(0.0, 1), (90.0, 1), (180.0, 1)]),
        ),
        (
            "four_way_2x2".to_string(),
            star(&[(0.0, 2), (90.0, 2), (180.0, 2), (270.0, 2)]),
        ),
        (
            "skew_60".to_string(),
            star(&[(0.0, 1), (60.0, 1), (180.0, 1), (240.0, 1)]),
        ),
        ("bend_45".to_string(), star(&[(180.0, 1), (45.0, 1)])),
    ];
    fixtures.extend(footpath_fixtures(false).into_iter().filter(|(name, _)| {
        name.starts_with("t_beside_arm/") || name.starts_with("through_road/")
    }));
    fixtures
}

#[test]
fn junction_connectors_stay_on_the_carriageway_without_backward_steps_or_kinks() {
    let mut report = String::new();
    let mut clean = true;
    for (name, mut graph) in fixtures() {
        let mut roadbed = RoadbedSystem::new(64.0);
        roadbed.compile_dirty(&graph);
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        let counts = measure_connectors(&graph, &roadbed, &lanes, [0], true);
        assert!(counts.connectors > 0, "{name}: no connectors measured");
        clean &= counts.clean();
        report.push_str(&counts.report(&name));
        for line in &counts.lines {
            writeln!(report, "  {line}").unwrap();
        }
    }
    assert!(clean, "junction connector violations:\n{report}");
}
