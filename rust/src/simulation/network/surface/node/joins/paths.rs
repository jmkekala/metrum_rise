//! Side-join path construction and endpoint lookup.

use super::*;

pub(super) fn reheight_side_join_path_world(
    mut path_world: Vec<RoadVec3>,
    start_height_m: f64,
    end_height_m: f64,
) -> Result<Option<Vec<RoadVec3>>, SideJoinGenerationError> {
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
        path_world[index].y = start_height_m + (end_height_m - start_height_m) * t;
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
) -> Result<Option<Vec<RoadVec3>>, SideJoinGenerationError> {
    let from_xz = xz_from_road_vec3(from_world);
    let to_xz = xz_from_road_vec3(to_world);
    let join_point_xz = side_join_backend_meet_point_xz(
        from_xz,
        from_mouth.direction_xz,
        to_xz,
        to_mouth.direction_xz,
    );
    let path_xz = if let Some(join_point_xz) = join_point_xz {
        let Some(path_xz) =
            side_join_backend_join_path_xz(from_xz, join_point_xz, to_xz, path_mode)
        else {
            return Ok(None);
        };
        path_xz
    } else {
        let Some(path_xz) = cleaned_open_road_points([from_xz, to_xz]) else {
            return Ok(None);
        };
        path_xz
    };
    let path_world = path_xz
        .into_iter()
        .map(|point_xz| {
            let height_m = height_plane.map_or_else(
                || height_on_linear_height_path(point_xz, from_xz, from_world.y, to_xz, to_world.y),
                |plane| {
                    if same_surface_xz_key(point_xz, from_xz) {
                        from_world.y
                    } else if same_surface_xz_key(point_xz, to_xz) {
                        to_world.y
                    } else {
                        plane.height_at_xz(point_xz)
                    }
                },
            );
            RoadVec3::new(point_xz.x, height_m, point_xz.y)
        })
        .collect();
    clean_side_join_path_world(path_world).map_err(SideJoinGenerationError::from_path_height_error)
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
            return side_join_backend_cavalier_join_path_xz(start_xz, join_point_xz, end_xz);
        }
    }
    cleaned_open_road_points([start_xz, join_point_xz, end_xz])
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
    Ok((cleaned_world.len() >= 2).then_some(cleaned_world))
}

fn endpoint_boundary_world(mouth: &NodeInputMouth, boundary_index: usize) -> Option<RoadVec3> {
    mouth
        .boundary_rails
        .get(boundary_index)
        .map(|rail| rail.endpoint_world)
}

pub(super) fn endpoint_layer_inner_world(
    mouth: &NodeInputMouth,
    layer: &SideJoinLayer,
) -> Option<RoadVec3> {
    endpoint_layer_boundary_world(mouth, layer, layer.inner_boundary_index)
}

pub(super) fn endpoint_layer_outer_world(
    mouth: &NodeInputMouth,
    layer: &SideJoinLayer,
) -> Option<RoadVec3> {
    endpoint_layer_boundary_world(mouth, layer, layer.outer_boundary_index)
}

fn endpoint_layer_boundary_world(
    mouth: &NodeInputMouth,
    layer: &SideJoinLayer,
    boundary_index: usize,
) -> Option<RoadVec3> {
    let interval = mouth.band_intervals.get(layer.band_index)?;
    if boundary_index == layer.band_index {
        Some(interval.endpoint_start_world)
    } else if boundary_index == layer.band_index + 1 {
        Some(interval.endpoint_end_world)
    } else {
        endpoint_boundary_world(mouth, boundary_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
