// SPDX-License-Identifier: GPL-2.0-only

//! Height-source diagnostic extraction helpers.

use super::*;
use crate::simulation::network::surface::keys::SurfaceXzKey;

pub(in crate::simulation::network::surface::tests) fn arrangement_key_from_overlay_keys(
    x_key: i64,
    z_key: i64,
) -> NodeArrangementKey {
    NodeArrangementKey::from_point(super::backend::RoadVec2::new(
        x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
    ))
}

pub(in crate::simulation::network::surface::tests) fn source_rail_debug_for_height_conflict(
    input: &super::input::NodeArrangementInput,
    constraint: Option<&super::rails::NodeRailConstraint>,
) -> String {
    let Some(constraint) = constraint else {
        return "rail_constraint=<missing>".to_string();
    };
    let mut parts = vec![format!("rail_constraint={constraint:?}")];
    let Some(boundary_index) = constraint.source_boundary_index else {
        return parts.join(" ");
    };
    let Some(mouth) = input
        .mouths
        .iter()
        .find(|mouth| mouth.order_index == constraint.source_mouth_order_index)
    else {
        parts.push("mouth=<missing>".to_string());
        return parts.join(" ");
    };
    if let Some(boundary_rail) = mouth.boundary_rails.get(boundary_index) {
        parts.push(format!(
            "boundary_path={}",
            world_path_debug(&boundary_rail.path_world)
        ));
    }
    if let Some(left_band) = boundary_index
        .checked_sub(1)
        .and_then(|index| mouth.band_intervals.get(index))
    {
        parts.push(format!(
            "left_band={:?} start_path={} end_path={}",
            left_band.band_kind,
            world_path_debug(&left_band.start_path_world),
            world_path_debug(&left_band.end_path_world)
        ));
    }
    if let Some(right_band) = mouth.band_intervals.get(boundary_index) {
        parts.push(format!(
            "right_band={:?} start_path={} end_path={}",
            right_band.band_kind,
            world_path_debug(&right_band.start_path_world),
            world_path_debug(&right_band.end_path_world)
        ));
    }
    parts.join(" ")
}

pub(in crate::simulation::network::surface::tests) fn ownership_debug_for_height_conflict(
    error: &super::height::NodeHeightFieldError,
    rails: &super::rails::NodeRailContourSet,
    ownership: &super::ownership::NodeBooleanOwnership,
) -> String {
    let super::height::NodeHeightFieldError::SharedSourceHeightConflict {
        point_x_mm,
        point_z_mm,
        owner,
        incoming_owner,
        ..
    } = error
    else {
        return String::new();
    };
    let point_xz =
        super::backend::RoadVec2::new(*point_x_mm as f64 / 1000.0, *point_z_mm as f64 / 1000.0);
    let point_key = SurfaceXzKey::from_road_xz(point_xz);
    let rail_constraints_at_key = rails
        .constraints
        .iter()
        .filter(|constraint| {
            constraint.points_xz.iter().any(|point| {
                let key = SurfaceXzKey::from_road_xz(*point);
                key.x_mm() == *point_x_mm && key.z_mm() == *point_z_mm
            }) || constraint.points_xz.windows(2).any(|segment| {
                let start = SurfaceXzKey::from_road_xz(segment[0]);
                let end = SurfaceXzKey::from_road_xz(segment[1]);
                super::segments::key_lies_on_segment(point_key, start, end)
            })
        })
        .take(24)
        .map(|constraint| {
            format!(
                "#{} {:?} owner={:?} opposite={:?} source=({},{:?}) point_count={} first={:?} last={:?}",
                constraint.constraint_index,
                constraint.kind,
                constraint.owner,
                constraint.opposite_owner,
                constraint.source_mouth_order_index,
                constraint.source_band_index,
                constraint.points_xz.len(),
                constraint
                    .points_xz
                    .first()
                    .map(|point| SurfaceXzKey::from_road_xz(*point).raw_tuple()),
                constraint
                    .points_xz
                    .last()
                    .map(|point| SurfaceXzKey::from_road_xz(*point).raw_tuple()),
            )
        })
        .collect::<Vec<_>>();
    let owned_regions_at_key = ownership
        .owned_regions
        .iter()
        .enumerate()
        .filter(|(_, region)| region.owner == *owner || region.owner == *incoming_owner)
        .filter_map(|(region_index, region)| {
            let has_vertex = region.shape.iter().flatten().any(|point| {
                let key = SurfaceXzKey::from_overlay_point(*point);
                key == point_key || (key.x_mm() == *point_x_mm && key.z_mm() == *point_z_mm)
            });
            if !has_vertex {
                return None;
            }
            let seams = region
                .seam_constraints
                .iter()
                .filter(|constraint| {
                    let start = SurfaceXzKey::from_road_xz(constraint.start_xz);
                    let end = SurfaceXzKey::from_road_xz(constraint.end_xz);
                    start == point_key
                        || end == point_key
                        || (start.x_mm() == *point_x_mm && start.z_mm() == *point_z_mm)
                        || (end.x_mm() == *point_x_mm && end.z_mm() == *point_z_mm)
                })
                .map(|constraint| {
                    format!(
                        "#{} {:?} owner={:?} opposite={:?} shared={} material={} start={:?} end={:?}",
                        constraint.constraint_index,
                        constraint.seam_source,
                        constraint.owner,
                        constraint.opposite_owner,
                        constraint.constrains_shared_height,
                        constraint.is_material_transition,
                        SurfaceXzKey::from_road_xz(constraint.start_xz).raw_tuple(),
                        SurfaceXzKey::from_road_xz(constraint.end_xz).raw_tuple(),
                    )
                })
                .collect::<Vec<_>>();
            Some(format!(
                "region={region_index} owner={:?} kind={:?} seams={seams:?}",
                region.owner, region.kind
            ))
        })
        .collect::<Vec<_>>();
    format!(
        "height_conflict_point={:?} rail_constraints_at_key={rail_constraints_at_key:?} owned_regions_at_key={owned_regions_at_key:?}",
        point_key.raw_tuple()
    )
}

pub(in crate::simulation::network::surface::tests) fn world_path_debug(
    path: &[super::backend::RoadVec3],
) -> String {
    let points = path
        .iter()
        .map(|point| format!("({:.3},{:.3},{:.3})", point.x, point.y, point.z))
        .collect::<Vec<_>>();
    format!("[{}]", points.join(","))
}

pub(in crate::simulation::network::surface::tests) fn height_solution_vertices_at_arrangement_key(
    heights: &super::height::NodeHeightSolution,
    key: NodeArrangementKey,
) -> Vec<String> {
    let mut matches = Vec::new();
    for (region_index, region) in heights.regions.iter().enumerate() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            if NodeArrangementKey::from_point(vertex.point_xz) != key {
                continue;
            }
            let touching_seams = region
                .seam_constraints
                .iter()
                .filter(|constraint| {
                    let start = NodeArrangementKey::from_point(constraint.start_xz);
                    let end = NodeArrangementKey::from_point(constraint.end_xz);
                    start == key || end == key
                })
                .map(|constraint| {
                    format!(
                        "#{} {:?} owner={:?} opposite={:?} shared={} material={}",
                        constraint.constraint_index,
                        constraint.seam_source,
                        constraint.owner,
                        constraint.opposite_owner,
                        constraint.constrains_shared_height,
                        constraint.is_material_transition
                    )
                })
                .collect::<Vec<_>>();
            matches.push(format!(
                "region={} kind={:?} owner={:?} field={:?} height={:.3} authority={:?} seams={:?}",
                region_index,
                region.kind,
                region.owner,
                vertex.height_field_id,
                vertex.height_m,
                vertex.grade_authority,
                touching_seams
            ));
        }
    }
    matches
}

pub(in crate::simulation::network::surface::tests) fn height_solution_vertices_at_surface_key(
    heights: &super::height::NodeHeightSolution,
    key: SurfaceXzKey,
) -> Vec<String> {
    let mut matches = Vec::new();
    for (region_index, region) in heights.regions.iter().enumerate() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            if SurfaceXzKey::from_road_xz(vertex.point_xz) != key {
                continue;
            }
            let arrangement_key = NodeArrangementKey::from_point(vertex.point_xz);
            matches.push(format!(
                "region={} kind={:?} owner={:?} field={:?} height={:.3} arrangement_key={:?} authority={:?}",
                region_index,
                region.kind,
                region.owner,
                vertex.height_field_id,
                vertex.height_m,
                arrangement_key,
                vertex.grade_authority,
            ));
        }
    }
    matches
}
