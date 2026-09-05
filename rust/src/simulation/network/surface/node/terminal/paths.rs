// SPDX-License-Identifier: GPL-2.0-only

//! Terminal-cap path construction and cleanup.

use super::height_anchors::endpoint_boundary_world;
use super::*;

pub(super) fn terminal_offset_boundary_path(
    mouth: &NodeInputMouth,
    start_boundary_index: usize,
    end_boundary_index: usize,
    outward: RoadVec2,
    offset_m: f64,
    endpoint_heights_m: Option<(f64, f64)>,
) -> Option<Vec<RoadVec3>> {
    terminal_offset_boundary_path_with_linear_height(
        mouth,
        start_boundary_index,
        end_boundary_index,
        outward,
        offset_m,
        endpoint_heights_m,
    )
}

pub(super) fn terminal_offset_boundary_path_with_linear_height(
    mouth: &NodeInputMouth,
    start_boundary_index: usize,
    end_boundary_index: usize,
    outward: RoadVec2,
    offset_m: f64,
    endpoint_heights_m: Option<(f64, f64)>,
) -> Option<Vec<RoadVec3>> {
    if start_boundary_index >= end_boundary_index
        || end_boundary_index >= mouth.boundary_rails.len()
    {
        return None;
    }

    let start_base = xz(endpoint_boundary_world(mouth, start_boundary_index)?);
    let end_base = xz(endpoint_boundary_world(mouth, end_boundary_index)?);
    let axis = end_base - start_base;
    let axis_len2 = axis.length_squared();
    let mut points = Vec::with_capacity(end_boundary_index - start_boundary_index + 1);

    for boundary_index in start_boundary_index..=end_boundary_index {
        let base = endpoint_boundary_world(mouth, boundary_index)?;
        let base_xz = xz(base);
        let t = if axis_len2 > f64::EPSILON {
            ((base_xz - start_base).dot(axis) / axis_len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let height_m = endpoint_heights_m.map_or(base.y, |(start_height_m, end_height_m)| {
            start_height_m + (end_height_m - start_height_m) * t
        });
        points.push(RoadVec3::new(
            base.x + outward.x * offset_m,
            height_m,
            base.z + outward.y * offset_m,
        ));
    }

    Some(points)
}

pub(super) fn terminal_cap_contour_world(
    inner_path_world: &[RoadVec3],
    outer_path_world: &[RoadVec3],
) -> Result<Option<Vec<RoadVec3>>, PathHeightResolutionError> {
    if inner_path_world.len() < 2 || outer_path_world.len() < 2 {
        return Ok(None);
    }
    let mut contour_world = inner_path_world.to_vec();
    contour_world.extend(outer_path_world.iter().rev().copied());
    clean_terminal_cap_contour_world(contour_world)
}

pub(super) fn clean_terminal_cap_path_world(
    path_world: Vec<RoadVec3>,
) -> Result<Option<Vec<RoadVec3>>, PathHeightResolutionError> {
    if path_world.len() < 2 {
        return Ok(None);
    }
    let Some(polyline) = cleaned_open_world_path_polyline(
        &path_world,
        TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M,
        false,
    ) else {
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

fn clean_terminal_cap_contour_world(
    contour_world: Vec<RoadVec3>,
) -> Result<Option<Vec<RoadVec3>>, PathHeightResolutionError> {
    let raw = road_points_to_polyline(contour_world.iter().copied().map(xz), true);
    let cleaned =
        RoadPolyline::create_from_remove_repeat(&raw, TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M);
    if cleaned.vertex_count() < 3
        || cleaned.area().abs() <= f64::from(NODE_OVERLAY_MIN_AREA_M2)
        || cleaned.scan_for_self_intersect()
    {
        return Ok(None);
    }
    let Some(cleaned_world) =
        reheight_road_points_from_world_path(polyline_to_road_points(&cleaned), &contour_world)?
    else {
        return Ok(None);
    };
    Ok((cleaned_world.len() >= 3).then_some(cleaned_world))
}

pub(super) fn normalized_terminal_cap_direction(direction: RoadVec2) -> Option<RoadVec2> {
    let length = direction.length();
    (length > TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M).then_some(direction / length)
}
