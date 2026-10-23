// SPDX-License-Identifier: GPL-2.0-only

//! Rounded sidewalk corner routes between two crossing mouths, ported unchanged from the legacy
//! road-surface side-join backend until the junction layout supplies corner curves.

use crate::simulation::core::round_f64_to_i64;
use crate::simulation::network::roadbed::RoadVec2;
use cavalier_contours::core::math::{
    LineLineIntr, Vector2 as CavalierVec2, bulge_from_angle, line_line_intr,
};
use cavalier_contours::polyline::{
    PlineCreation, PlineSource, PlineVertex, Polyline, seg_midpoint, seg_split_at_point,
};
use std::f64::consts::{PI, TAU};

type RoadPolyline = Polyline<f64>;
type RoadPolylineVertex = PlineVertex<f64>;

const SIDE_JOIN_ARC_RADIUS_EPS_M: f64 = 0.001;
const SIDE_JOIN_ARC_SPLIT_DEPTH: usize = 3;
const SIDE_JOIN_FILLET_MIN_TANGENT_M: f64 = 0.25;
const SIDE_JOIN_FILLET_TANGENT_FRACTION: f64 = 0.25;
const SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;
// Micrometre XZ keys: endpoint identity after polyline cleanup.
const XZ_KEY_SCALE: f64 = 1_000_000.0;

fn xz_key(point: RoadVec2) -> (i64, i64) {
    (
        round_f64_to_i64(point.x * XZ_KEY_SCALE),
        round_f64_to_i64(point.y * XZ_KEY_SCALE),
    )
}

fn cleaned_open_road_polyline(
    points_xz: impl IntoIterator<Item = RoadVec2>,
    point_equal_eps_m: f64,
) -> Option<RoadPolyline> {
    let raw = RoadPolyline::from_iter(
        points_xz
            .into_iter()
            .map(|point| RoadPolylineVertex::new(point.x, point.y, 0.0)),
        false,
    );
    let mut cleaned = RoadPolyline::create_from_remove_repeat(&raw, point_equal_eps_m);
    if let Some(reduced) = cleaned.remove_redundant(point_equal_eps_m) {
        cleaned = reduced;
    }
    (cleaned.vertex_count() >= 2).then_some(cleaned)
}

fn polyline_to_road_points(polyline: &RoadPolyline) -> Vec<RoadVec2> {
    polyline
        .iter_vertexes()
        .map(|vertex| RoadVec2::new(vertex.x, vertex.y))
        .collect()
}

// ROAD-68-F replaces this with the junction layout's corner curves.
// Builds a sampled sidewalk centerline with the legacy rounded junction side-join policy.
pub(super) fn rounded_sidewalk_corner_path_xz(
    start_xz: RoadVec2,
    start_direction_xz: RoadVec2,
    end_xz: RoadVec2,
    end_direction_xz: RoadVec2,
) -> Option<Vec<RoadVec2>> {
    rounded_side_join_path_xz(start_xz, start_direction_xz, end_xz, end_direction_xz)
}

fn rounded_side_join_path_xz(
    start_xz: RoadVec2,
    start_direction_xz: RoadVec2,
    end_xz: RoadVec2,
    end_direction_xz: RoadVec2,
) -> Option<Vec<RoadVec2>> {
    if let Some(join_point_xz) =
        side_join_backend_meet_point_xz(start_xz, start_direction_xz, end_xz, end_direction_xz)
    {
        side_join_backend_join_path_xz(start_xz, join_point_xz, end_xz)
    } else {
        cleaned_open_road_points([start_xz, end_xz])
    }
}

fn side_join_backend_join_path_xz(
    start_xz: RoadVec2,
    join_point_xz: RoadVec2,
    end_xz: RoadVec2,
) -> Option<Vec<RoadVec2>> {
    let start_tangent = join_point_xz - start_xz;
    let end_tangent = end_xz - join_point_xz;
    if let Some(points) =
        side_join_backend_arc_path_xz(start_xz, start_tangent, end_xz, end_tangent)
    {
        return cleaned_open_road_points(points);
    }
    if let Some(points) = side_join_backend_fillet_path_xz(start_xz, join_point_xz, end_xz) {
        return cleaned_open_road_points(points);
    }
    if let Some(points) = side_join_backend_cavalier_join_path_xz(start_xz, join_point_xz, end_xz) {
        return Some(points);
    }
    cleaned_open_road_points([start_xz, join_point_xz, end_xz])
}

fn side_join_backend_fillet_path_xz(
    start_xz: RoadVec2,
    join_point_xz: RoadVec2,
    end_xz: RoadVec2,
) -> Option<Vec<RoadVec2>> {
    let incoming = join_point_xz - start_xz;
    let outgoing = end_xz - join_point_xz;
    let incoming_length_m = incoming.length();
    let outgoing_length_m = outgoing.length();
    if incoming_length_m <= SIDE_JOIN_FILLET_MIN_TANGENT_M
        || outgoing_length_m <= SIDE_JOIN_FILLET_MIN_TANGENT_M
    {
        return None;
    }

    let incoming_direction = incoming / incoming_length_m;
    let outgoing_direction = outgoing / outgoing_length_m;
    let tangent_length_m = (incoming_length_m.min(outgoing_length_m)
        * SIDE_JOIN_FILLET_TANGENT_FRACTION)
        .max(SIDE_JOIN_FILLET_MIN_TANGENT_M)
        .min(incoming_length_m * 0.5)
        .min(outgoing_length_m * 0.5);
    if tangent_length_m <= SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M {
        return None;
    }

    let fillet_start_xz = join_point_xz - incoming_direction * tangent_length_m;
    let fillet_end_xz = join_point_xz + outgoing_direction * tangent_length_m;
    let mut points = Vec::with_capacity((1 << SIDE_JOIN_ARC_SPLIT_DEPTH) + 4);
    points.push(start_xz);
    points.push(fillet_start_xz);
    if let Some(arc_points) = side_join_backend_arc_path_xz(
        fillet_start_xz,
        incoming_direction,
        fillet_end_xz,
        outgoing_direction,
    ) {
        points.extend(arc_points.into_iter().skip(1));
    } else {
        append_side_join_quadratic_fillet_samples(
            &mut points,
            fillet_start_xz,
            join_point_xz,
            fillet_end_xz,
            SIDE_JOIN_ARC_SPLIT_DEPTH,
        );
    }
    points.push(end_xz);
    Some(points)
}

fn append_side_join_quadratic_fillet_samples(
    points: &mut Vec<RoadVec2>,
    start_xz: RoadVec2,
    control_xz: RoadVec2,
    end_xz: RoadVec2,
    split_depth: usize,
) {
    let sample_count = 1 << split_depth;
    for index in 1..=sample_count {
        let t = index as f64 / sample_count as f64;
        let one_minus_t = 1.0 - t;
        let point = start_xz * (one_minus_t * one_minus_t)
            + control_xz * (2.0 * one_minus_t * t)
            + end_xz * (t * t);
        points.push(point);
    }
}

fn side_join_backend_cavalier_join_path_xz(
    start_xz: RoadVec2,
    join_point_xz: RoadVec2,
    end_xz: RoadVec2,
) -> Option<Vec<RoadVec2>> {
    let mut polyline = cleaned_open_road_polyline(
        [start_xz, join_point_xz, end_xz],
        SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M,
    )?;
    if polyline.vertex_count() < 2 || polyline.scan_for_self_intersect() {
        return None;
    }
    let mut points = polyline_to_road_points(&polyline);
    if points.len() < 2 {
        return None;
    }
    let last_index = points.len() - 1;
    if xz_key(points[0]) != xz_key(start_xz) || xz_key(points[last_index]) != xz_key(end_xz) {
        return None;
    }
    points[0] = start_xz;
    points[last_index] = end_xz;
    polyline = cleaned_open_road_polyline(points, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M)?;
    if polyline.vertex_count() < 2 || polyline.scan_for_self_intersect() {
        return None;
    }
    Some(polyline_to_road_points(&polyline))
}

fn side_join_backend_arc_path_xz(
    start_xz: RoadVec2,
    start_tangent: RoadVec2,
    end_xz: RoadVec2,
    end_tangent: RoadVec2,
) -> Option<Vec<RoadVec2>> {
    let start_tangent = normalized_side_join_direction(start_tangent)?;
    let end_tangent = normalized_side_join_direction(end_tangent)?;
    let center_xz = side_join_arc_center_xz(start_xz, start_tangent, end_xz, end_tangent)?;
    let start_radius_m = center_xz.distance(start_xz);
    let end_radius_m = center_xz.distance(end_xz);
    if start_radius_m <= SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M
        || (start_radius_m - end_radius_m).abs() > SIDE_JOIN_ARC_RADIUS_EPS_M
    {
        return None;
    }

    let sweep_angle =
        side_join_short_arc_sweep_angle(center_xz, start_xz, end_xz, start_tangent, end_tangent)?;
    if sweep_angle.abs() <= SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M || sweep_angle.abs() > PI {
        return None;
    }

    let start_vertex =
        RoadPolylineVertex::new(start_xz.x, start_xz.y, bulge_from_angle(sweep_angle));
    let end_vertex = RoadPolylineVertex::new(end_xz.x, end_xz.y, 0.0);
    let mut points = Vec::with_capacity((1 << SIDE_JOIN_ARC_SPLIT_DEPTH) + 1);
    append_side_join_backend_arc_samples(
        &mut points,
        start_vertex,
        end_vertex,
        SIDE_JOIN_ARC_SPLIT_DEPTH,
    );
    points.push(end_xz);
    Some(points)
}

fn side_join_short_arc_sweep_angle(
    center_xz: RoadVec2,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
    start_tangent: RoadVec2,
    end_tangent: RoadVec2,
) -> Option<f64> {
    let mut best_sweep_angle = None;
    let mut best_tangent_score = f64::NEG_INFINITY;
    for is_ccw in [true, false] {
        let sweep_angle = side_join_arc_sweep_angle(center_xz, start_xz, end_xz, is_ccw);
        let abs_sweep_angle = sweep_angle.abs();
        if abs_sweep_angle <= SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M || abs_sweep_angle > PI {
            continue;
        }
        let start_arc_tangent = side_join_arc_tangent_xz(center_xz, start_xz, is_ccw)?;
        let end_arc_tangent = side_join_arc_tangent_xz(center_xz, end_xz, is_ccw)?;
        let tangent_score = start_arc_tangent.dot(start_tangent) + end_arc_tangent.dot(end_tangent);
        if tangent_score > best_tangent_score {
            best_sweep_angle = Some(sweep_angle);
            best_tangent_score = tangent_score;
        }
    }
    best_sweep_angle
}

fn side_join_arc_center_xz(
    start_xz: RoadVec2,
    start_tangent: RoadVec2,
    end_xz: RoadVec2,
    end_tangent: RoadVec2,
) -> Option<RoadVec2> {
    let start_normal = left_perp(start_tangent);
    let end_normal = left_perp(end_tangent);
    side_join_backend_meet_point_xz(start_xz, start_normal, end_xz, end_normal)
}

fn side_join_arc_tangent_xz(
    center_xz: RoadVec2,
    point_xz: RoadVec2,
    is_ccw: bool,
) -> Option<RoadVec2> {
    let radius = point_xz - center_xz;
    let tangent = if is_ccw {
        left_perp(radius)
    } else {
        -left_perp(radius)
    };
    normalized_side_join_direction(tangent)
}

fn side_join_arc_sweep_angle(
    center_xz: RoadVec2,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
    is_ccw: bool,
) -> f64 {
    let start = start_xz - center_xz;
    let end = end_xz - center_xz;
    let start_angle = start.y.atan2(start.x);
    let end_angle = end.y.atan2(end.x);
    if is_ccw {
        (end_angle - start_angle).rem_euclid(TAU)
    } else {
        -((start_angle - end_angle).rem_euclid(TAU))
    }
}

fn append_side_join_backend_arc_samples(
    points: &mut Vec<RoadVec2>,
    start_vertex: RoadPolylineVertex,
    end_vertex: RoadPolylineVertex,
    split_depth: usize,
) {
    if split_depth == 0 {
        points.push(RoadVec2::new(start_vertex.x, start_vertex.y));
        return;
    }
    let midpoint = seg_midpoint(start_vertex, end_vertex);
    let split = seg_split_at_point(
        start_vertex,
        end_vertex,
        midpoint,
        SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M,
    );
    append_side_join_backend_arc_samples(
        points,
        split.updated_start,
        split.split_vertex,
        split_depth - 1,
    );
    append_side_join_backend_arc_samples(points, split.split_vertex, end_vertex, split_depth - 1);
}

fn normalized_side_join_direction(direction: RoadVec2) -> Option<RoadVec2> {
    let length = direction.length();
    (length > SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M).then_some(direction / length)
}

fn left_perp(direction: RoadVec2) -> RoadVec2 {
    RoadVec2::new(-direction.y, direction.x)
}

fn side_join_backend_meet_point_xz(
    start_a: RoadVec2,
    direction_a: RoadVec2,
    start_b: RoadVec2,
    direction_b: RoadVec2,
) -> Option<RoadVec2> {
    let a0 = cavalier_vec2(start_a);
    let a1 = cavalier_vec2(start_a + direction_a);
    let b0 = cavalier_vec2(start_b);
    let b1 = cavalier_vec2(start_b + direction_b);
    match line_line_intr(a0, a1, b0, b1, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M) {
        LineLineIntr::TrueIntersect { seg1_t, .. }
        | LineLineIntr::FalseIntersect { seg1_t, .. } => Some(start_a + direction_a * seg1_t),
        LineLineIntr::NoIntersect | LineLineIntr::Overlapping { .. } => None,
    }
}

fn cavalier_vec2(point: RoadVec2) -> CavalierVec2<f64> {
    CavalierVec2::new(point.x, point.y)
}

fn cleaned_open_road_points(
    points_xz: impl IntoIterator<Item = RoadVec2>,
) -> Option<Vec<RoadVec2>> {
    cleaned_open_road_polyline(points_xz, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M)
        .map(|polyline| polyline_to_road_points(&polyline))
}
