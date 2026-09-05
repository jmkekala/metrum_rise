// SPDX-License-Identifier: GPL-2.0-only

//! Rail path cleanup and point-appending helpers.

use super::*;

pub(in crate::simulation::network::surface::node::rails) fn subdivided_world_chord(
    start: RoadVec3,
    end: RoadVec3,
    point_count: usize,
) -> Vec<RoadVec3> {
    if point_count < 2 {
        return vec![start, end];
    }
    (0..point_count)
        .map(|index| {
            let t = index as f64 / (point_count - 1) as f64;
            start + (end - start) * t
        })
        .collect()
}

pub(in crate::simulation::network::surface::node::rails) fn clean_generated_constraint_path(
    points: Vec<RoadVec2>,
) -> Option<Vec<RoadVec2>> {
    let mut cleaned = Vec::with_capacity(points.len());
    for point in points {
        push_road_path_point(&mut cleaned, point);
    }
    if cleaned
        .windows(2)
        .any(|segment| road_point_key(segment[0]) != road_point_key(segment[1]))
    {
        let raw = road_points_to_polyline(cleaned, false);
        let rail = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
        (rail.vertex_count() >= 2 && rail.path_length() > RAIL_CONTOUR_POINT_EQUAL_EPS_M)
            .then(|| polyline_to_road_points(&rail))
    } else {
        None
    }
}

pub(in crate::simulation::network::surface::node::rails) fn open_world_path_xz(
    path_world: &[RoadVec3],
    mouth_world: RoadVec3,
    endpoint_world: RoadVec3,
) -> Vec<RoadVec2> {
    let mut points = Vec::new();
    push_road_path_point(&mut points, xz(mouth_world));
    append_world_path_xz(&mut points, path_world.iter());
    push_road_path_point(&mut points, xz(endpoint_world));
    points
}

pub(in crate::simulation::network::surface::node::rails) fn append_world_path_xz<'a>(
    points: &mut Vec<RoadVec2>,
    path_world: impl IntoIterator<Item = &'a RoadVec3>,
) {
    for point in path_world {
        push_road_path_point(points, xz(*point));
    }
}

pub(in crate::simulation::network::surface::node::rails) fn append_world_path_points<'a>(
    points: &mut Vec<RoadVec3>,
    path_world: impl IntoIterator<Item = &'a RoadVec3>,
) {
    for point in path_world {
        push_world_path_point(points, *point);
    }
}

pub(in crate::simulation::network::surface::node::rails) fn push_road_path_point(
    points: &mut Vec<RoadVec2>,
    point: RoadVec2,
) {
    if points
        .last()
        .is_none_or(|last| road_point_key(*last) != road_point_key(point))
    {
        points.push(point);
    }
}

pub(in crate::simulation::network::surface::node::rails) fn push_world_path_point(
    points: &mut Vec<RoadVec3>,
    point: RoadVec3,
) {
    if points.last().is_none_or(|last| {
        road_point_key(xz(*last)) != road_point_key(xz(point))
            || SurfaceHeightMmKey::from_m_f64(last.y) != SurfaceHeightMmKey::from_m_f64(point.y)
    }) {
        points.push(point);
    }
}

pub(in crate::simulation::network::surface::node::rails) fn remove_closing_road_path_duplicate(
    points: &mut Vec<RoadVec2>,
) {
    if points.len() > 1
        && road_point_key(points[0]) == road_point_key(*points.last().expect("len checked"))
    {
        points.pop();
    }
}

pub(in crate::simulation::network::surface::node::rails) fn remove_closing_world_path_duplicate(
    points: &mut Vec<RoadVec3>,
) {
    if points.len() > 1
        && road_point_key(xz(points[0])) == road_point_key(xz(*points.last().expect("len checked")))
        && SurfaceHeightMmKey::from_m_f64(points[0].y)
            == SurfaceHeightMmKey::from_m_f64(points.last().expect("len checked").y)
    {
        points.pop();
    }
}
