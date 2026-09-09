// SPDX-License-Identifier: GPL-2.0-only

//! Side-join path construction and endpoint lookup.

use super::*;

/// Applies material-height offsets without replacing the retained longitudinal grade.
pub(super) fn offset_side_join_path_world(
    mut path_world: Vec<RoadVec3>,
    start_height_m: f64,
    end_height_m: f64,
) -> Result<Option<Vec<RoadVec3>>, SideJoinGenerationError> {
    let Some(first) = path_world.first() else {
        return Ok(None);
    };
    let start_offset_m = start_height_m - first.y;
    let end_offset_m = end_height_m - path_world.last().unwrap().y;
    let total_length_m = path_world
        .windows(2)
        .map(|segment| xz_from_road_vec3(segment[0]).distance(xz_from_road_vec3(segment[1])))
        .sum::<f64>();
    let mut cumulative_length_m = 0.0;
    for index in 0..path_world.len() {
        if index > 0 {
            cumulative_length_m += xz_from_road_vec3(path_world[index - 1])
                .distance(xz_from_road_vec3(path_world[index]));
        }
        let t = if total_length_m > f64::EPSILON {
            cumulative_length_m / total_length_m
        } else {
            0.0
        };
        // Raised curb bands have an intentional step. Apply that offset to the shared
        // solved grade instead of replacing the grade with a mouth-to-mouth chord.
        path_world[index].y += start_offset_m + (end_offset_m - start_offset_m) * t;
    }
    clean_side_join_path_world(path_world).map_err(SideJoinGenerationError::from_path_height_error)
}

pub(super) fn side_join_boundary_path_world(
    from_mouth: &NodeInputMouth,
    from_world: RoadVec3,
    to_mouth: &NodeInputMouth,
    to_world: RoadVec3,
    path_mode: SideJoinPathMode,
    height_plane: Option<SideJoinHeightPlane>,
) -> Result<Option<SideJoinBoundaryPath>, SideJoinGenerationError> {
    let from_xz = xz_from_road_vec3(from_world);
    let to_xz = xz_from_road_vec3(to_world);
    let join_point_xz = side_join_backend_meet_point_xz(
        from_xz,
        from_mouth.direction_xz,
        to_xz,
        to_mouth.direction_xz,
    );
    let Some(path_xz) = rounded_side_join_path_xz(
        from_xz,
        from_mouth.direction_xz,
        to_xz,
        to_mouth.direction_xz,
        path_mode,
    ) else {
        return Ok(None);
    };
    let rounded_world = side_join_path_points_to_world(
        path_xz,
        from_mouth,
        from_world,
        to_mouth,
        to_world,
        height_plane,
    )?;
    let Some(rounded_world) = rounded_world else {
        return Ok(None);
    };
    let miter_xz = join_point_xz.map_or_else(
        || vec![from_xz, to_xz],
        |join_point_xz| vec![from_xz, join_point_xz, to_xz],
    );
    let miter_world = side_join_path_points_to_world(
        miter_xz,
        from_mouth,
        from_world,
        to_mouth,
        to_world,
        height_plane,
    )?;
    let Some(miter_world) = miter_world else {
        return Ok(None);
    };
    Ok(Some(SideJoinBoundaryPath {
        rounded_world,
        miter_world,
    }))
}

/// Builds a sampled sidewalk centerline using the same rounded join policy as node ownership.
pub(crate) fn rounded_sidewalk_corner_path_xz(
    start_xz: RoadVec2,
    start_direction_xz: RoadVec2,
    end_xz: RoadVec2,
    end_direction_xz: RoadVec2,
) -> Option<Vec<RoadVec2>> {
    rounded_side_join_path_xz(
        start_xz,
        start_direction_xz,
        end_xz,
        end_direction_xz,
        SideJoinPathMode::JunctionNonRoad,
    )
}

fn rounded_side_join_path_xz(
    start_xz: RoadVec2,
    start_direction_xz: RoadVec2,
    end_xz: RoadVec2,
    end_direction_xz: RoadVec2,
    path_mode: SideJoinPathMode,
) -> Option<Vec<RoadVec2>> {
    if let Some(join_point_xz) =
        side_join_backend_meet_point_xz(start_xz, start_direction_xz, end_xz, end_direction_xz)
    {
        side_join_backend_join_path_xz(start_xz, join_point_xz, end_xz, path_mode)
    } else {
        cleaned_open_road_points([start_xz, end_xz])
    }
}

fn side_join_path_points_to_world(
    points_xz: Vec<RoadVec2>,
    from_mouth: &NodeInputMouth,
    from_world: RoadVec3,
    to_mouth: &NodeInputMouth,
    to_world: RoadVec3,
    height_plane: Option<SideJoinHeightPlane>,
) -> Result<Option<Vec<RoadVec3>>, SideJoinGenerationError> {
    let from_xz = xz_from_road_vec3(from_world);
    let to_xz = xz_from_road_vec3(to_world);
    let on_tangent = |point: RoadVec2, origin: RoadVec2, direction: RoadVec2| {
        (point - origin).perp_dot(direction).abs() <= 1.0 / super::super::keys::SURFACE_XZ_KEY_SCALE
    };
    let from_anchor = points_xz
        .iter()
        .take_while(|&&p| on_tangent(p, from_xz, from_mouth.direction_xz))
        .count()
        .saturating_sub(1);
    let to_anchor = points_xz.len().saturating_sub(
        points_xz
            .iter()
            .rev()
            .take_while(|&&p| on_tangent(p, to_xz, to_mouth.direction_xz))
            .count(),
    );
    // Opposite mouths can describe the same tangent. Their line intersection may
    // lie beyond a mouth because of coordinate roundoff; it is not a real corner.
    // Keep the bounded straight join, then recover BOTH incident profile domains.
    let chord = to_xz - from_xz;
    let chord_length = chord.length();
    let straight_join = chord_length > f64::EPSILON
        && points_xz.iter().all(|point| {
            // Bound incidence by endpoint/vertex coordinate rounding, not a height
            // tolerance. A genuine corner must keep its rounded or miter geometry.
            (*point - from_xz).perp_dot(chord).abs()
                <= 4.0 / super::super::keys::SURFACE_XZ_KEY_SCALE * chord_length
        });
    let (points_xz, from_anchor, to_anchor) = if straight_join {
        (vec![from_xz, to_xz], 1, 0)
    } else {
        (points_xz, from_anchor, to_anchor)
    };
    let (points_xz, from_anchor, to_anchor) = if height_plane.is_some() {
        retain_approach_profile_stations(
            points_xz,
            from_mouth,
            from_xz,
            to_mouth,
            to_xz,
            from_anchor,
            to_anchor,
        )
    } else {
        (points_xz, from_anchor, to_anchor)
    };
    let mut distances = vec![0.0; points_xz.len()];
    for i in 1..points_xz.len() {
        distances[i] = distances[i - 1] + points_xz[i].distance(points_xz[i - 1]);
    }
    let residual_curve = height_plane.and_then(|plane| {
        if from_anchor >= to_anchor || to_anchor >= points_xz.len() {
            return None;
        }
        let (from_height, from_grade) =
            approach_height_residual(from_mouth, from_world, points_xz[from_anchor], plane)?;
        let (to_height, to_grade) =
            approach_height_residual(to_mouth, to_world, points_xz[to_anchor], plane)?;
        Some((
            from_height,
            -from_grade,
            to_height,
            to_grade,
            distances[to_anchor] - distances[from_anchor],
        ))
    });
    let path_world = points_xz
        .into_iter()
        .enumerate()
        .map(|(index, point_xz)| {
            let height_m = height_plane.map_or_else(
                || height_on_linear_height_path(point_xz, from_xz, from_world.y, to_xz, to_world.y),
                |plane| {
                    if same_surface_xz_key(point_xz, from_xz) {
                        from_world.y
                    } else if same_surface_xz_key(point_xz, to_xz) {
                        to_world.y
                    } else {
                        let residual = if straight_join {
                            approach_height_residual(from_mouth, from_world, point_xz, plane)
                                .or_else(|| {
                                    approach_height_residual(to_mouth, to_world, point_xz, plane)
                                })
                                .map(|sample| sample.0)
                                .unwrap_or(0.0)
                        } else if index <= from_anchor {
                            approach_height_residual(from_mouth, from_world, point_xz, plane)
                                .map(|sample| sample.0)
                                .unwrap_or(0.0)
                        } else if index >= to_anchor {
                            approach_height_residual(to_mouth, to_world, point_xz, plane)
                                .map(|sample| sample.0)
                                .unwrap_or(0.0)
                        } else if let Some((a, da, b, db, length)) = residual_curve {
                            let t = (distances[index] - distances[from_anchor]) / length;
                            let t2 = t * t;
                            let t3 = t2 * t;
                            (2.0 * t3 - 3.0 * t2 + 1.0) * a
                                + (t3 - 2.0 * t2 + t) * length * da
                                + (-2.0 * t3 + 3.0 * t2) * b
                                + (t3 - t2) * length * db
                        } else {
                            0.0
                        };
                        plane.height_at_xz(point_xz) + residual
                    }
                },
            );
            RoadVec3::new(point_xz.x, height_m, point_xz.y)
        })
        .collect();
    clean_side_join_path_world(path_world).map_err(SideJoinGenerationError::from_path_height_error)
}

// Ownership paths may be straight in XZ without being straight in elevation. Retain
// physical-profile stations on their straight approach portions, before assigning heights.
// Work is bounded to the two incident profiles and the local corner path.
fn retain_approach_profile_stations(
    points: Vec<RoadVec2>,
    from: &NodeInputMouth,
    from_xz: RoadVec2,
    to: &NodeInputMouth,
    to_xz: RoadVec2,
    from_anchor: usize,
    to_anchor: usize,
) -> (Vec<RoadVec2>, usize, usize) {
    let mut result = Vec::with_capacity(points.len());
    let mut stations = Vec::new();
    let mut new_from_anchor = from_anchor;
    let mut new_to_anchor = to_anchor;
    for (index, segment) in points.windows(2).enumerate() {
        if index == from_anchor {
            new_from_anchor = result.len();
        }
        if index == to_anchor {
            new_to_anchor = result.len();
        }
        result.push(segment[0]);
        let axis = segment[1] - segment[0];
        let length2 = axis.length_squared();
        if length2 <= f64::EPSILON {
            continue;
        }
        stations.clear();
        for (mouth, origin, in_approach) in [
            (from, from_xz, index < from_anchor),
            (to, to_xz, index >= to_anchor),
        ] {
            if !in_approach {
                continue;
            }
            let Some(rail) = mouth
                .boundary_rails
                .iter()
                .find(|rail| same_surface_xz_key(xz_from_road_vec3(rail.mouth_world), origin))
            else {
                continue;
            };
            if !path_has_vertical_curve(&rail.path_world) {
                continue;
            }
            for point in &rail.path_world {
                let t = (xz_from_road_vec3(*point) - segment[0]).dot(axis) / length2;
                if t > 0.0 && t < 1.0 {
                    stations.push((t, xz_from_road_vec3(*point)));
                }
            }
        }
        stations.sort_by(|a, b| a.0.total_cmp(&b.0));
        stations.dedup();
        result.extend(stations.iter().map(|(_, point)| *point));
    }
    if from_anchor == points.len().saturating_sub(1) {
        new_from_anchor = result.len();
    }
    if to_anchor == points.len().saturating_sub(1) {
        new_to_anchor = result.len();
    }
    if let Some(last) = points.last() {
        result.push(*last);
    }
    (result, new_from_anchor, new_to_anchor)
}

fn path_has_vertical_curve(path: &[RoadVec3]) -> bool {
    let (Some(start), Some(end)) = (path.first(), path.last()) else {
        return false;
    };
    path.iter().any(|point| {
        let height = height_on_linear_height_path(
            xz_from_road_vec3(*point),
            xz_from_road_vec3(*start),
            start.y,
            xz_from_road_vec3(*end),
            end.y,
        );
        (height - point.y).abs()
            > 4.0
                * f64::from(f32::EPSILON)
                * point.y.abs().max(start.y.abs()).max(end.y.abs()).max(1.0)
    })
}

// Evaluate the retained incident profile, not a new target grade. Straight approach portions
// use it verbatim; the rounded corner interpolates its residual from the common crossing plane
// with endpoint height/grade continuity. A planar profile therefore remains exactly planar.
fn approach_height_residual(
    mouth: &NodeInputMouth,
    mouth_world: RoadVec3,
    point: RoadVec2,
    plane: SideJoinHeightPlane,
) -> Option<(f64, f64)> {
    let rail = mouth.boundary_rails.iter().find(|rail| {
        same_surface_xz_key(
            xz_from_road_vec3(rail.mouth_world),
            xz_from_road_vec3(mouth_world),
        )
    })?;
    let offset = mouth_world.y - rail.mouth_world.y;
    let station = (point - xz_from_road_vec3(rail.endpoint_world)).dot(mouth.direction_xz);
    for segment in rail.path_world.windows(2) {
        let a = xz_from_road_vec3(segment[0]);
        let b = xz_from_road_vec3(segment[1]);
        let sa = (a - xz_from_road_vec3(rail.endpoint_world)).dot(mouth.direction_xz);
        let sb = (b - xz_from_road_vec3(rail.endpoint_world)).dot(mouth.direction_xz);
        if station < sa.min(sb) - 0.000001
            || station > sa.max(sb) + 0.000001
            || (sb - sa).abs() <= f64::EPSILON
        {
            continue;
        }
        let ra = segment[0].y + offset - plane.height_at_xz(a);
        let rb = segment[1].y + offset - plane.height_at_xz(b);
        let t = ((station - sa) / (sb - sa)).clamp(0.0, 1.0);
        return Some((ra + (rb - ra) * t, (rb - ra) / (sb - sa)));
    }
    None
}

fn same_surface_xz_key(a: RoadVec2, b: RoadVec2) -> bool {
    SurfaceXzKey::from_road_xz(a) == SurfaceXzKey::from_road_xz(b)
}

fn side_join_backend_join_path_xz(
    start_xz: RoadVec2,
    join_point_xz: RoadVec2,
    end_xz: RoadVec2,
    path_mode: SideJoinPathMode,
) -> Option<Vec<RoadVec2>> {
    let start_tangent = join_point_xz - start_xz;
    let end_tangent = end_xz - join_point_xz;
    match path_mode {
        SideJoinPathMode::BendArc => {
            if let Some(points) =
                side_join_backend_arc_path_xz(start_xz, start_tangent, end_xz, end_tangent)
            {
                return cleaned_open_road_points(points);
            }
        }
        SideJoinPathMode::JunctionNonRoad => {
            if let Some(points) =
                side_join_backend_arc_path_xz(start_xz, start_tangent, end_xz, end_tangent)
            {
                return cleaned_open_road_points(points);
            }
        }
    }
    if let Some(points) = side_join_backend_fillet_path_xz(start_xz, join_point_xz, end_xz) {
        return cleaned_open_road_points(points);
    }
    if path_mode == SideJoinPathMode::JunctionNonRoad
        && let Some(points) =
            side_join_backend_cavalier_join_path_xz(start_xz, join_point_xz, end_xz)
    {
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
        true,
    )?;
    if polyline.vertex_count() < 2 || polyline.scan_for_self_intersect() {
        return None;
    }
    let mut points = polyline_to_road_points(&polyline);
    if points.len() < 2 {
        return None;
    }
    let last_index = points.len() - 1;
    if SurfaceXzKey::from_road_xz(points[0]) != SurfaceXzKey::from_road_xz(start_xz)
        || SurfaceXzKey::from_road_xz(points[last_index]) != SurfaceXzKey::from_road_xz(end_xz)
    {
        return None;
    }
    points[0] = start_xz;
    points[last_index] = end_xz;
    polyline = cleaned_open_road_polyline(points, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M, true)?;
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
    cleaned_open_road_polyline(points_xz, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M, true)
        .map(|polyline| polyline_to_road_points(&polyline))
}

fn height_on_linear_height_path(
    point_xz: RoadVec2,
    start_xz: RoadVec2,
    start_height_m: f64,
    end_xz: RoadVec2,
    end_height_m: f64,
) -> f64 {
    let axis = end_xz - start_xz;
    let axis_len2 = axis.length_squared();
    let t = if axis_len2 > f64::EPSILON {
        ((point_xz - start_xz).dot(axis) / axis_len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    start_height_m + (end_height_m - start_height_m) * t
}

fn clean_side_join_path_world(
    path_world: Vec<RoadVec3>,
) -> Result<Option<Vec<RoadVec3>>, PathHeightResolutionError> {
    if path_world.len() < 2 {
        return Ok(None);
    }
    let Some(polyline) =
        cleaned_open_world_path_polyline(&path_world, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M, true)
    else {
        return Ok(None);
    };
    if polyline.vertex_count() < 2 {
        return Ok(None);
    }
    let points_xz = polyline_to_road_points(&polyline);
    let Some(cleaned_world) = reheight_road_points_from_world_path(points_xz, &path_world)? else {
        return Ok(None);
    };
    // XZ simplification is allowed only when it also preserves the physical grade.
    let removes_height_support = path_world.iter().any(|point| {
        !cleaned_world.windows(2).any(|segment| {
            let axis = xz_from_road_vec3(segment[1]) - xz_from_road_vec3(segment[0]);
            let t = (xz_from_road_vec3(*point) - xz_from_road_vec3(segment[0])).dot(axis)
                / axis.length_squared();
            t >= 0.0
                && t <= 1.0
                && xz_from_road_vec3(*point).distance(xz_from_road_vec3(segment[0]) + axis * t)
                    <= SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M
                && (point.y - (segment[0].y + (segment[1].y - segment[0].y) * t)).abs()
                    <= 4.0 * f64::from(f32::EPSILON) * point.y.abs().max(1.0)
        })
    });
    if removes_height_support {
        let mut retained = path_world;
        remove_repeated_road_vec3_xz_points(&mut retained)?;
        return Ok(Some(retained));
    }
    Ok((cleaned_world.len() >= 2).then_some(cleaned_world))
}

fn mouth_boundary_world(mouth: &NodeInputMouth, boundary_index: usize) -> Option<RoadVec3> {
    mouth
        .boundary_rails
        .get(boundary_index)
        .map(|rail| rail.mouth_world)
}

pub(super) fn mouth_layer_inner_world(
    mouth: &NodeInputMouth,
    layer: &SideJoinLayer,
) -> Option<RoadVec3> {
    mouth_layer_boundary_world(mouth, layer, layer.inner_boundary_index)
}

pub(super) fn mouth_layer_outer_world(
    mouth: &NodeInputMouth,
    layer: &SideJoinLayer,
) -> Option<RoadVec3> {
    mouth_layer_boundary_world(mouth, layer, layer.outer_boundary_index)
}

fn mouth_layer_boundary_world(
    mouth: &NodeInputMouth,
    layer: &SideJoinLayer,
    boundary_index: usize,
) -> Option<RoadVec3> {
    let interval = mouth.band_intervals.get(layer.band_index)?;
    if boundary_index == layer.band_index {
        Some(interval.mouth_start_world)
    } else if boundary_index == layer.band_index + 1 {
        Some(interval.mouth_end_world)
    } else {
        mouth_boundary_world(mouth, boundary_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::node::input::{
        NodeInputBoundaryRail, NodeInputBoundaryRailRole,
    };

    #[test]
    fn straight_join_preserves_both_approaches_and_removes_extrapolated_miter() {
        let mouth = |direction: RoadVec2, path: Vec<RoadVec3>| NodeInputMouth {
            order_index: 0,
            edge_idx: 0,
            side: IncidentEdgeSide::Start,
            direction_xz: direction,
            direction_angle_ccw: direction.y.atan2(direction.x),
            conflict_handoff_distance_m: 10.0,
            mouth_rails: Vec::new(),
            endpoint_rails: Vec::new(),
            band_intervals: Vec::new(),
            uses_explicit_band_domain_paths: false,
            boundary_rails: vec![NodeInputBoundaryRail {
                boundary_index: 0,
                role: NodeInputBoundaryRailRole::OuterFootprint {
                    adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                },
                endpoint_world: path[0],
                mouth_world: *path.last().unwrap(),
                path_world: path,
            }],
        };
        let left = mouth(
            -RoadVec2::X,
            vec![
                RoadVec3::new(0.0, 10.0, 0.0),
                RoadVec3::new(-5.0, 8.6, 0.0),
                RoadVec3::new(-10.0, 7.3, 0.0),
            ],
        );
        let right = mouth(
            RoadVec2::X,
            vec![
                RoadVec3::new(0.0, 10.0, 0.0),
                RoadVec3::new(5.0, 11.7, 0.0),
                RoadVec3::new(10.0, 13.6, 0.0),
            ],
        );
        let plane = SideJoinHeightPlane {
            origin: RoadVec3::new(0.0, 10.0, 0.0),
            grade_x: 0.3,
            grade_z: 0.0,
        };
        for (from, to) in [(&left, &right), (&right, &left)] {
            let start = from.boundary_rails[0].mouth_world;
            let end = to.boundary_rails[0].mouth_world;
            for with_miter in [false, true] {
                let mut points = vec![xz_from_road_vec3(start)];
                if with_miter {
                    points.push(xz_from_road_vec3(end) + to.direction_xz);
                }
                points.push(xz_from_road_vec3(end));
                let path =
                    side_join_path_points_to_world(points, from, start, to, end, Some(plane))
                        .unwrap()
                        .unwrap();
                assert!(path.iter().all(|p| p.x.abs() <= 10.0));
                for expected in left.boundary_rails[0]
                    .path_world
                    .iter()
                    .chain(&right.boundary_rails[0].path_world)
                {
                    let actual = path.iter().find(|p| p.x == expected.x).unwrap();
                    assert!((actual.y - expected.y).abs() < 1.0e-9, "{path:?}");
                }
            }
        }
    }

    #[test]
    fn straight_ownership_path_retains_vertical_curve_supports() {
        let curved = vec![
            RoadVec3::new(0.0, 10.0, 0.0),
            RoadVec3::new(1.0, 10.25, 0.0),
            RoadVec3::new(2.0, 11.0, 0.0),
        ];
        let retained = clean_side_join_path_world(curved.clone()).unwrap().unwrap();
        assert_eq!(
            retained, curved,
            "XZ-collinear stations still carry the solved vertical curve"
        );
        let planar = vec![curved[0], RoadVec3::new(1.0, 10.5, 0.0), curved[2]];
        let cleaned = clean_side_join_path_world(planar).unwrap().unwrap();
        assert_eq!(
            cleaned,
            vec![curved[0], curved[2]],
            "a truly linear grade needs no extra station"
        );
    }

    #[test]
    fn logged_bend_side_join_uses_short_arc_instead_of_miter() {
        let center = RoadVec2::new(-6.673889, -23.719093);
        let west = RoadVec2::new(-125.385117, -31.426414);
        let east = RoadVec2::new(34.735245, 44.360130);
        let first_direction = normalized_test_direction(east - center);
        let second_direction = normalized_test_direction(west - center);
        let first_outer = center + left_perp(first_direction) * 5.0;
        let second_outer = center - left_perp(second_direction) * 5.0;
        let join_point = side_join_backend_meet_point_xz(
            first_outer,
            first_direction,
            second_outer,
            second_direction,
        )
        .expect("logged bend offset rails should have a miter meet point");

        let path = side_join_backend_join_path_xz(
            first_outer,
            join_point,
            second_outer,
            SideJoinPathMode::BendArc,
        )
        .expect("logged bend side join should emit a path");

        assert!(
            path.len() > 3,
            "bend side join must keep an arc, not the miter fallback: {path:?}"
        );
        assert!(
            path.iter()
                .all(|point| (point.distance(center) - 5.0).abs() <= 0.01),
            "bend side join arc must stay on the road-owned endpoint radius: {path:?}"
        );
        assert!(
            path.iter()
                .all(|point| point.distance(join_point) > SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M),
            "bend side join arc must not route through the sharp miter point: {path:?}"
        );
    }

    fn normalized_test_direction(direction: RoadVec2) -> RoadVec2 {
        direction / direction.length()
    }

    #[test]
    fn bend_side_join_rounds_asymmetric_miter() {
        let start = RoadVec2::new(8.0, 0.0);
        let join = RoadVec2::new(8.0, 2.0);
        let end = RoadVec2::new(0.0, 8.0);

        assert!(
            side_join_backend_arc_path_xz(start, join - start, end, end - join).is_none(),
            "test setup must exercise the bounded fillet fallback"
        );
        let path = side_join_backend_join_path_xz(start, join, end, SideJoinPathMode::BendArc)
            .expect("bend fillet should emit a path");

        assert_bounded_fillet_path(start, join, end, &path);
    }

    #[test]
    fn junction_side_join_rounds_asymmetric_miter() {
        let start = RoadVec2::new(8.0, 0.0);
        let join = RoadVec2::new(8.0, 2.0);
        let end = RoadVec2::new(0.0, 8.0);

        assert!(
            side_join_backend_arc_path_xz(start, join - start, end, end - join).is_none(),
            "test setup must exercise the bounded fillet fallback"
        );
        let path =
            side_join_backend_join_path_xz(start, join, end, SideJoinPathMode::JunctionNonRoad)
                .expect("JunctionN fillet should emit a path");

        assert_bounded_fillet_path(start, join, end, &path);
    }

    fn assert_bounded_fillet_path(
        start: RoadVec2,
        join: RoadVec2,
        end: RoadVec2,
        path: &[RoadVec2],
    ) {
        assert_eq!(
            SurfaceXzKey::from_road_xz(path[0]),
            SurfaceXzKey::from_road_xz(start)
        );
        assert_eq!(
            SurfaceXzKey::from_road_xz(*path.last().expect("non-empty path")),
            SurfaceXzKey::from_road_xz(end)
        );
        assert!(
            path.len() > 4,
            "fillet must add a rounded corner path, not a miter: {path:?}"
        );
        let incoming = join - start;
        let outgoing = end - join;
        let incoming_length_m = incoming.length();
        let outgoing_length_m = outgoing.length();
        let tangent_length_m =
            expected_fillet_tangent_length_m(incoming_length_m, outgoing_length_m);
        let expected_start = join - incoming / incoming_length_m * tangent_length_m;
        let expected_end = join + outgoing / outgoing_length_m * tangent_length_m;
        assert!(
            path[1].distance(expected_start) <= 1.0e-6,
            "fillet must start inside the bounded tangent window: {path:?}"
        );
        assert!(
            path[path.len() - 2].distance(expected_end) <= 1.0e-6,
            "fillet must end inside the bounded tangent window: {path:?}"
        );
        assert!(
            path.iter()
                .all(|point| point.distance(join) > SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M),
            "rounded fillet path must not route through the sharp miter point: {path:?}"
        );
    }

    fn expected_fillet_tangent_length_m(incoming_length_m: f64, outgoing_length_m: f64) -> f64 {
        (incoming_length_m.min(outgoing_length_m) * SIDE_JOIN_FILLET_TANGENT_FRACTION)
            .max(SIDE_JOIN_FILLET_MIN_TANGENT_M)
            .min(incoming_length_m * 0.5)
            .min(outgoing_length_m * 0.5)
    }

    #[test]
    fn junction_non_road_side_join_uses_arc_when_rails_support_it() {
        let start = RoadVec2::new(5.0, 0.0);
        let join = RoadVec2::new(5.0, 5.0);
        let end = RoadVec2::new(0.0, 5.0);

        let path =
            side_join_backend_join_path_xz(start, join, end, SideJoinPathMode::JunctionNonRoad)
                .expect("junction side join should emit a path");

        assert!(
            path.len() > 3,
            "junction non-road side join must keep an arc, not the miter fallback: {path:?}"
        );
        assert!(
            path.iter()
                .all(|point| (point.length() - 5.0).abs() <= 0.01),
            "junction side join arc must stay on the supported radius: {path:?}"
        );
        assert!(
            path.iter()
                .all(|point| point.distance(join) > SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M),
            "junction side join arc must not route through the sharp miter point: {path:?}"
        );
    }
}
