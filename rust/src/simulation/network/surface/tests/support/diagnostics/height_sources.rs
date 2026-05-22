//! Height-source diagnostic extraction helpers.

use super::*;

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
                "region={} kind={:?} owner={:?} field={:?} height={:.3} seams={:?}",
                region_index,
                region.kind,
                region.owner,
                vertex.height_field_id,
                vertex.height_m,
                touching_seams
            ));
        }
    }
    matches
}
