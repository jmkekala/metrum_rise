// SPDX-License-Identifier: GPL-2.0-only

//! Zoning parcel variant export helpers.

use super::super::*;

pub(in crate::nodes::simulation_node) fn zoning_parcel_geometry_dict(
    core: &SimCore,
    geometry: &crate::simulation::zoning::ParcelGeometry,
    runtime_id: u16,
    occupied: bool,
    parcel_id: u64,
) -> VarDictionary {
    let mut corners = PackedVector3Array::new();
    for corner in zoning_parcel_surface_corners(core, geometry) {
        corners.push(corner);
    }

    let color = zoning_parcel_color(core, runtime_id, occupied);

    let mut dict = VarDictionary::new();
    dict.set("id", i64::try_from(parcel_id).unwrap_or(i64::MAX));
    dict.set("profile_runtime_id", i64::from(runtime_id));
    dict.set("occupied", occupied);
    dict.set("corners", corners);
    dict.set("color", color);
    dict
}

pub(in crate::nodes::simulation_node) fn zoning_parcel_color(
    core: &SimCore,
    runtime_id: u16,
    occupied: bool,
) -> Color {
    let mut color = if runtime_id == 0 {
        Color::from_rgba(0.78, 0.82, 0.78, 0.30)
    } else if let Some(profile) = core.zoning.profiles.profile_by_runtime_id(runtime_id) {
        Color::from_rgba(
            profile.ui_color_rgb[0] as f32 / 255.0,
            profile.ui_color_rgb[1] as f32 / 255.0,
            profile.ui_color_rgb[2] as f32 / 255.0,
            0.34,
        )
    } else {
        Color::from_rgba(0.78, 0.82, 0.78, 0.30)
    };
    if occupied {
        color = Color::from_rgba(color.r * 0.55, color.g * 0.55, color.b * 0.55, 0.28);
    }
    color
}

pub(in crate::nodes::simulation_node) fn zoning_parcel_geometries_array(
    core: &SimCore,
    geometries: &[crate::simulation::zoning::ParcelGeometry],
    runtime_id: u16,
) -> VarArray {
    let mut arr = VarArray::new();
    for geometry in geometries {
        let dict = zoning_parcel_geometry_dict(core, geometry, runtime_id, false, 0);
        arr.push(&dict.to_variant());
    }
    arr
}

pub(in crate::nodes::simulation_node) fn zoning_geometries_without_explicit_sites(
    core: &SimCore,
    geometries: Vec<crate::simulation::zoning::ParcelGeometry>,
) -> Vec<crate::simulation::zoning::ParcelGeometry> {
    geometries
        .into_iter()
        .filter(|geometry| {
            !core
                .allocator
                .parcel_geometry_overlaps_explicit_site(geometry)
        })
        .collect()
}

pub(in crate::nodes::simulation_node) fn zoning_parcel_geometries_packed_dict(
    core: &SimCore,
    geometries: &[crate::simulation::zoning::ParcelGeometry],
    runtime_id: u16,
) -> VarDictionary {
    let mut corners = PackedVector3Array::new();
    for geometry in geometries {
        for corner in zoning_parcel_surface_corners(core, geometry) {
            corners.push(corner);
        }
    }

    let mut dict = VarDictionary::new();
    dict.set(
        "parcel_count",
        i64::try_from(geometries.len()).unwrap_or(i64::MAX),
    );
    dict.set("corners", corners);
    dict.set("color", zoning_parcel_color(core, runtime_id, false));
    dict
}

pub(in crate::nodes::simulation_node) fn zoning_parcel_cell_dimensions(
    config: &WorldConfig,
    frontage_cells: i32,
    depth_cells: i32,
) -> Option<(f32, f32)> {
    if frontage_cells <= 0
        || depth_cells <= 0
        || !config.zone_cell_m.is_finite()
        || config.zone_cell_m <= 0.0
    {
        return None;
    }
    let frontage_m = frontage_cells as f32 * config.zone_cell_m;
    let depth_m = depth_cells as f32 * config.zone_cell_m;
    if frontage_m.is_finite() && depth_m.is_finite() {
        Some((frontage_m, depth_m))
    } else {
        None
    }
}

pub(in crate::nodes::simulation_node) fn zoning_parcel_surface_corners(
    core: &SimCore,
    geometry: &crate::simulation::zoning::ParcelGeometry,
) -> [Vector3; 4] {
    geometry.corners.map(|corner| {
        let surface_y = core.get_world_surface_height_internal(Vector2::new(corner.x, corner.y));
        Vector3::new(corner.x, surface_y, corner.y)
    })
}
