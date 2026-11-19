// SPDX-License-Identifier: GPL-2.0-only

//! Building-specific rendering logic for Godot interaction.
//!
//! Handles building instance transform generation and construction-site visuals.

use crate::assets::{AssetEntry, MeshPart};
use crate::nodes::sim::core::SimCore;
use crate::simulation::buildings::allocator::{Building, building_local_xz_basis};
use crate::simulation::core::scheduler::{SLOTS_PER_DAY, SLOTS_PER_MINUTE};
use crate::simulation::zoning::ZoneType;
use godot::prelude::*;

const SITE_INSET_SCALE: f32 = 0.92;
const FOUNDATION_INSET_SCALE: f32 = 0.68;
const SITE_PAD_HEIGHT_M: f32 = 0.08;
const FOUNDATION_PAD_HEIGHT_M: f32 = 0.18;
const SCAFFOLD_POST_THICKNESS_M: f32 = 0.28;
const SCAFFOLD_RAIL_THICKNESS_M: f32 = 0.22;
const CONSTRUCTION_RISE_INITIAL_HEIGHT_FACTOR: f32 = 0.65;
const CONSTRUCTION_RISE_INITIAL_EXTRA_M: f32 = 2.0;
const CONSTRUCTION_RISE_INITIAL_MIN_OFFSET_M: f32 = 6.0;
const CONSTRUCTION_RISE_INITIAL_MAX_OFFSET_M: f32 = 12.0;

impl SimCore {
    // ── Building Renderer ──

    pub(crate) fn get_service_building_preview_part_transforms_internal(
        &self,
        asset_id: &str,
        center_2d: Vector2,
        support_height_m: f32,
        facing_dir: Vector2,
    ) -> PackedFloat32Array {
        let Some(entry) = self.allocator.registry.get(asset_id) else {
            return PackedFloat32Array::new();
        };

        let mut buffer = Vec::with_capacity(entry.manifest.mesh_parts.len() * 12);
        for part in &entry.manifest.mesh_parts {
            push_building_part_transform_for_pose(
                &mut buffer,
                center_2d,
                support_height_m,
                facing_dir,
                Some(entry),
                part,
            );
        }

        PackedFloat32Array::from_iter(buffer)
    }

    // Construction-site pad transforms of zone `zone_type_int`'s buildings under construction:
    // the retired scan, kept verbatim as the published site rows' oracle (`RENDER-30`).
    #[cfg(test)]
    pub(crate) fn get_construction_site_transforms_internal(
        &self,
        zone_type_int: u8,
    ) -> Vec<f32> {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return Vec::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if !b.is_under_construction() || b.zone_type != target_zone {
                continue;
            }

            let ground_y = b.support_height_m;
            let (right, front) = building_lot_basis(b);
            let (lot_width, lot_depth) = building_lot_size_m(self.config.zone_cell_m, b);
            let width = lot_width * SITE_INSET_SCALE;
            let depth = lot_depth * SITE_INSET_SCALE;
            let center_y = ground_y + SITE_PAD_HEIGHT_M * 0.5 + 0.015;

            push_oriented_box_transform(
                &mut buffer,
                b.center_x,
                b.center_y,
                center_y,
                right,
                front,
                width,
                SITE_PAD_HEIGHT_M,
                depth,
            );
        }

        buffer
    }

    // Raised foundation transforms, as above.
    #[cfg(test)]
    pub(crate) fn get_construction_foundation_transforms_internal(
        &self,
        zone_type_int: u8,
    ) -> Vec<f32> {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return Vec::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if !b.is_under_construction() || b.zone_type != target_zone {
                continue;
            }

            let ground_y = b.support_height_m;
            let (right, front) = building_lot_basis(b);
            let (lot_width, lot_depth) = building_lot_size_m(self.config.zone_cell_m, b);
            let width = lot_width * FOUNDATION_INSET_SCALE;
            let depth = lot_depth * FOUNDATION_INSET_SCALE;
            let center_y = ground_y + SITE_PAD_HEIGHT_M + FOUNDATION_PAD_HEIGHT_M * 0.5 + 0.025;

            push_oriented_box_transform(
                &mut buffer,
                b.center_x,
                b.center_y,
                center_y,
                right,
                front,
                width,
                FOUNDATION_PAD_HEIGHT_M,
                depth,
            );
        }

        buffer
    }

    // Procedural scaffold bar transforms, as above.
    #[cfg(test)]
    pub(crate) fn get_construction_scaffold_transforms_internal(
        &self,
        zone_type_int: u8,
    ) -> Vec<f32> {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return Vec::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if !b.is_under_construction() || b.zone_type != target_zone {
                continue;
            }

            let ground_y = b.support_height_m;
            let (right, front) = building_lot_basis(b);
            let (lot_width, lot_depth) = building_lot_size_m(self.config.zone_cell_m, b);
            let width = lot_width * 0.82;
            let depth = lot_depth * 0.78;
            let height = construction_scaffold_height_m(b);

            push_scaffold_transforms(
                &mut buffer,
                b.center_x,
                b.center_y,
                ground_y,
                right,
                front,
                width,
                depth,
                height,
            );
        }

        buffer
    }

    /// Continuous absolute slot clock, for presentation interpolation between slots.
    pub(crate) fn construction_clock_slots(&self) -> f64 {
        let seconds_per_minute = self.time.seconds_per_minute().max(f64::EPSILON);
        let minute_fraction = (self.time.time_elapsed / seconds_per_minute).clamp(0.0, 1.0);
        let minute_start = u64::from(self.time.day_index.saturating_sub(1)) * SLOTS_PER_DAY
            + u64::from(self.time.minute_of_day) * SLOTS_PER_MINUTE;
        minute_start as f64 + minute_fraction * SLOTS_PER_MINUTE as f64
    }
}

fn push_building_part_transform_for_pose(
    buffer: &mut Vec<f32>,
    center_2d: Vector2,
    world_y: f32,
    facing_dir: Vector2,
    entry: Option<&AssetEntry>,
    part: &MeshPart,
) {
    buffer.extend_from_slice(&super::building_lod::pack_transform(building_part_pose(
        center_2d, world_y, facing_dir, entry, part,
    )));
}

/// Shared placed mesh transform for gameplay batches and service-placement previews.
pub(super) fn building_part_pose(
    center_2d: Vector2,
    world_y: f32,
    facing_dir: Vector2,
    entry: Option<&AssetEntry>,
    part: &MeshPart,
) -> glam::Mat4 {
    let (basis_x, basis_z) = building_local_xz_basis(facing_dir, building_frontage_forward(entry));
    let local = part.local_transform();
    let part_x_axis = basis_x * local.matrix3.x_axis.x + basis_z * local.matrix3.x_axis.z;
    let part_z_axis = basis_x * local.matrix3.z_axis.x + basis_z * local.matrix3.z_axis.z;
    let translation = center_2d + basis_x * local.translation.x + basis_z * local.translation.z;

    glam::Mat4::from_cols(
        glam::Vec4::new(part_x_axis.x, 0.0, part_x_axis.y, 0.0),
        glam::Vec4::new(0.0, local.matrix3.y_axis.y, 0.0, 0.0),
        glam::Vec4::new(part_z_axis.x, 0.0, part_z_axis.y, 0.0),
        glam::Vec4::new(
            translation.x,
            world_y + local.translation.y,
            translation.y,
            1.0,
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn push_scaffold_transforms(
    buffer: &mut Vec<f32>,
    center_x: f32,
    center_z: f32,
    ground_y: f32,
    right: Vector2,
    front: Vector2,
    width: f32,
    depth: f32,
    height: f32,
) {
    let half_width = width * 0.5;
    let half_depth = depth * 0.5;
    let post_center_y = ground_y + height * 0.5 + 0.08;
    let post_offsets = [
        (-half_width, -half_depth),
        (0.0, -half_depth),
        (half_width, -half_depth),
        (-half_width, 0.0),
        (half_width, 0.0),
        (-half_width, half_depth),
        (0.0, half_depth),
        (half_width, half_depth),
    ];

    for (offset_x, offset_z) in post_offsets {
        let (x, z) = offset_point(center_x, center_z, right, front, offset_x, offset_z);
        push_oriented_box_transform(
            buffer,
            x,
            z,
            post_center_y,
            right,
            front,
            SCAFFOLD_POST_THICKNESS_M,
            height,
            SCAFFOLD_POST_THICKNESS_M,
        );
    }

    let rail_levels = [height * 0.42, height * 0.72, height * 0.94];
    for level in rail_levels {
        let rail_center_y = ground_y + level + 0.08;
        for offset_z in [-half_depth, half_depth] {
            let (x, z) = offset_point(center_x, center_z, right, front, 0.0, offset_z);
            push_oriented_box_transform(
                buffer,
                x,
                z,
                rail_center_y,
                right,
                front,
                width + SCAFFOLD_POST_THICKNESS_M,
                SCAFFOLD_RAIL_THICKNESS_M,
                SCAFFOLD_RAIL_THICKNESS_M,
            );
        }
        for offset_x in [-half_width, half_width] {
            let (x, z) = offset_point(center_x, center_z, right, front, offset_x, 0.0);
            push_oriented_box_transform(
                buffer,
                x,
                z,
                rail_center_y,
                right,
                front,
                SCAFFOLD_RAIL_THICKNESS_M,
                SCAFFOLD_RAIL_THICKNESS_M,
                depth + SCAFFOLD_POST_THICKNESS_M,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_oriented_box_transform(
    buffer: &mut Vec<f32>,
    center_x: f32,
    center_z: f32,
    center_y: f32,
    right: Vector2,
    front: Vector2,
    scale_x: f32,
    scale_y: f32,
    scale_z: f32,
) {
    buffer.push(right.x * scale_x);
    buffer.push(0.0);
    buffer.push(front.x * scale_z);
    buffer.push(center_x);

    buffer.push(0.0);
    buffer.push(scale_y);
    buffer.push(0.0);
    buffer.push(center_y);

    buffer.push(right.y * scale_x);
    buffer.push(0.0);
    buffer.push(front.y * scale_z);
    buffer.push(center_z);
}

fn offset_point(
    center_x: f32,
    center_z: f32,
    right: Vector2,
    front: Vector2,
    offset_x: f32,
    offset_z: f32,
) -> (f32, f32) {
    (
        center_x + right.x * offset_x + front.x * offset_z,
        center_z + right.y * offset_x + front.y * offset_z,
    )
}

fn building_lot_basis(building: &Building) -> (Vector2, Vector2) {
    let front = if building.facing_dir.length_squared() > 1e-12 {
        building.facing_dir.normalized()
    } else {
        Vector2::new(0.0, 1.0)
    };
    let right = Vector2::new(front.y, -front.x);
    (right, front)
}

fn building_lot_size_m(cell_size_m: f32, building: &Building) -> (f32, f32) {
    (
        building.width_cells as f32 * cell_size_m,
        building.depth_cells as f32 * cell_size_m,
    )
}

/// Appends a building under construction's site pad row (12 floats) to `buffer`.
pub(super) fn construction_site_pad_rows(buffer: &mut Vec<f32>, cell_size_m: f32, b: &Building) {
    let ground_y = b.support_height_m;
    let (right, front) = building_lot_basis(b);
    let (lot_width, lot_depth) = building_lot_size_m(cell_size_m, b);
    let width = lot_width * SITE_INSET_SCALE;
    let depth = lot_depth * SITE_INSET_SCALE;
    let center_y = ground_y + SITE_PAD_HEIGHT_M * 0.5 + 0.015;
    push_oriented_box_transform(
        buffer,
        b.center_x,
        b.center_y,
        center_y,
        right,
        front,
        width,
        SITE_PAD_HEIGHT_M,
        depth,
    );
}

/// Appends a building under construction's raised foundation row to `buffer`.
pub(super) fn construction_foundation_rows(buffer: &mut Vec<f32>, cell_size_m: f32, b: &Building) {
    let ground_y = b.support_height_m;
    let (right, front) = building_lot_basis(b);
    let (lot_width, lot_depth) = building_lot_size_m(cell_size_m, b);
    let width = lot_width * FOUNDATION_INSET_SCALE;
    let depth = lot_depth * FOUNDATION_INSET_SCALE;
    let center_y = ground_y + SITE_PAD_HEIGHT_M + FOUNDATION_PAD_HEIGHT_M * 0.5 + 0.025;
    push_oriented_box_transform(
        buffer,
        b.center_x,
        b.center_y,
        center_y,
        right,
        front,
        width,
        FOUNDATION_PAD_HEIGHT_M,
        depth,
    );
}

/// Appends a building under construction's 20 scaffold bar rows (eight posts, three rail
/// levels of four rails) to `buffer`.
pub(super) fn construction_scaffold_rows(buffer: &mut Vec<f32>, cell_size_m: f32, b: &Building) {
    let ground_y = b.support_height_m;
    let (right, front) = building_lot_basis(b);
    let (lot_width, lot_depth) = building_lot_size_m(cell_size_m, b);
    let width = lot_width * 0.82;
    let depth = lot_depth * 0.78;
    let height = construction_scaffold_height_m(b);
    push_scaffold_transforms(
        buffer,
        b.center_x,
        b.center_y,
        ground_y,
        right,
        front,
        width,
        depth,
        height,
    );
}

/// Existing construction animation's vertical offset below the authored support plane.
pub(super) fn construction_rise_offset_m(building: &Building, progress: f32) -> f32 {
    let t = progress.clamp(0.0, 1.0);
    (1.0 - t) * construction_initial_rise_offset_m(building)
}

fn construction_initial_rise_offset_m(building: &Building) -> f32 {
    construction_initial_rise_offset_for_height_m(construction_scaffold_height_m(building))
}

fn construction_initial_rise_offset_for_height_m(scaffold_height_m: f32) -> f32 {
    (scaffold_height_m.max(0.0) * CONSTRUCTION_RISE_INITIAL_HEIGHT_FACTOR
        + CONSTRUCTION_RISE_INITIAL_EXTRA_M)
        .clamp(
            CONSTRUCTION_RISE_INITIAL_MIN_OFFSET_M,
            CONSTRUCTION_RISE_INITIAL_MAX_OFFSET_M,
        )
}

/// Continuous presentation progress at the slot clock `clock_slots`, without changing
/// construction simulation state.
pub(super) fn construction_visual_progress(building: &Building, clock_slots: f64) -> f32 {
    building.construction_progress(clock_slots)
}

fn construction_scaffold_height_m(building: &Building) -> f32 {
    let level = f32::from(building.level.max(1));
    match building.zone_type {
        ZoneType::Residential => 7.0 + level * 2.2,
        ZoneType::Commercial => 8.0 + level * 3.0,
        ZoneType::Industrial => 8.5 + level * 2.8,
        ZoneType::Office | ZoneType::Mixed | ZoneType::None => 7.0 + level * 2.5,
    }
    .clamp(7.0, 18.0)
}

fn building_frontage_forward(entry: Option<&AssetEntry>) -> [f32; 3] {
    entry
        .map(|entry| entry.manifest.building_frontage_forward())
        .unwrap_or([0.0, 0.0, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn building_part_pose_matches_editor_yaw_scale_and_pivot() {
        for yaw in [-90.0_f32, -31.0, 0.0, 47.0, 90.0] {
            for heading in [-2.4_f32, 0.0, 1.2, std::f32::consts::PI] {
                let mut part = MeshPart::single_lod0("rotated", "unused.glb");
                part.position = [2.0, 3.0, -1.0];
                part.rotation_degrees = [0.0, yaw, 0.0];
                part.scale = 2.0;
                part.pivot_offset = Some([0.5, 0.25, -0.75]);
                let mut buffer = Vec::new();
                let origin = Vector3::new(11.0, 17.0, -5.0);
                push_building_part_transform_for_pose(
                    &mut buffer,
                    Vector2::new(origin.x, origin.z),
                    origin.y,
                    Vector2::new(heading.sin(), heading.cos()),
                    None,
                    &part,
                );
                let actual = Transform3D::new(
                    Basis::from_cols(
                        Vector3::new(buffer[0], buffer[4], buffer[8]),
                        Vector3::new(buffer[1], buffer[5], buffer[9]),
                        Vector3::new(buffer[2], buffer[6], buffer[10]),
                    ),
                    Vector3::new(buffer[3], buffer[7], buffer[11]),
                );
                // Independent oracle: the editor uses Godot's positive-Y Basis, not our helper.
                let building = Basis::from_axis_angle(Vector3::UP, heading);
                let rotation = Basis::from_axis_angle(Vector3::UP, yaw.to_radians());
                for point in [Vector3::ZERO, Vector3::RIGHT, Vector3::UP, Vector3::BACK] {
                    let expected = origin
                        + building
                            * (Vector3::new(2.0, 3.0, -1.0)
                                + rotation * ((point + Vector3::new(0.5, 0.25, -0.75)) * 2.0));
                    assert!((actual * point).distance_to(expected) < 1e-5);
                }
            }
        }
    }

    fn assert_vec2_close(actual: Vector2, expected: Vector2) {
        assert!(
            (actual.x - expected.x).abs() < 1e-5 && (actual.y - expected.y).abs() < 1e-5,
            "expected ({:.3}, {:.3}), got ({:.3}, {:.3})",
            expected.x,
            expected.y,
            actual.x,
            actual.y
        );
    }

    #[test]
    fn test_building_basis_aligns_authored_front_to_road_facing_dir() {
        let world_front = Vector2::new(0.0, -1.0);

        let (basis_x, basis_z) = building_local_xz_basis(world_front, [0.0, 0.0, 1.0]);
        assert_vec2_close(basis_x * 0.0 + basis_z * 1.0, world_front);

        let (basis_x, basis_z) = building_local_xz_basis(world_front, [0.0, 0.0, -1.0]);
        assert_vec2_close(basis_x * 0.0 + basis_z * -1.0, world_front);
    }

    #[test]
    fn construction_initial_rise_offset_starts_near_surface() {
        assert!(
            (construction_initial_rise_offset_for_height_m(7.0) - 6.55).abs() < 1e-5,
            "low-rise construction should become visible quickly"
        );
        assert_eq!(
            construction_initial_rise_offset_for_height_m(18.0),
            CONSTRUCTION_RISE_INITIAL_MAX_OFFSET_M,
            "tall buildings should not start deeply buried"
        );
        assert_eq!(
            construction_initial_rise_offset_for_height_m(0.0),
            CONSTRUCTION_RISE_INITIAL_MIN_OFFSET_M
        );
    }
}
