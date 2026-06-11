//! Building-specific rendering logic for Godot interaction.
//!
//! Handles building instance transform generation and plot/construction-site visuals.

use crate::assets::{AnchorType, AssetEntry};
use crate::nodes::sim::core::SimCore;
use crate::simulation::buildings::allocator::Building;
use crate::simulation::zoning::ZoneType;
use godot::prelude::*;

const SITE_INSET_SCALE: f32 = 0.92;
const FOUNDATION_INSET_SCALE: f32 = 0.68;
const SITE_PAD_HEIGHT_M: f32 = 0.08;
const FOUNDATION_PAD_HEIGHT_M: f32 = 0.18;
const SCAFFOLD_POST_THICKNESS_M: f32 = 0.28;
const SCAFFOLD_RAIL_THICKNESS_M: f32 = 0.22;

impl SimCore {
    // ── Building Renderer ──

    /// Returns the 12-float transforms for all placed buildings with the given asset ID.
    pub fn get_building_transforms_for_asset_internal(&self, asset_id: &str) -> PackedFloat32Array {
        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if asset_id == "broken:error" {
                if !b.broken || b.is_under_construction() {
                    continue;
                }
            } else {
                // Skip broken buildings (handled by broken:error group) and deserted buildings
                // (handled by the parallel deserted multimesh via get_deserted_building_transforms_for_asset_internal).
                if b.broken || b.is_deserted || b.asset_id != asset_id {
                    continue;
                }
            }

            let world_x = b.center_x;
            let world_z = b.center_y;
            let mut world_y = self.heightmap.sample_height_world(world_x, world_z) * 20.0;
            if b.is_under_construction() {
                let progress = construction_visual_progress(b, self.operational_hour_fraction());
                world_y -= construction_rise_offset_m(b, progress);
            }
            let entry = self.allocator.registry.get(asset_id);
            push_building_transform(&mut buffer, b, entry, world_y);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns the 12-float transforms for all deserted buildings with the given asset ID.
    ///
    /// Deserted buildings render in a parallel multimesh with a gray material override.
    pub fn get_deserted_building_transforms_for_asset_internal(
        &self,
        asset_id: &str,
    ) -> PackedFloat32Array {
        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if b.broken || b.is_under_construction() || !b.is_deserted || b.asset_id != asset_id {
                continue;
            }

            let world_x = b.center_x;
            let world_z = b.center_y;
            let world_y = self.heightmap.sample_height_world(world_x, world_z) * 20.0;
            let entry = self.allocator.registry.get(asset_id);
            push_building_transform(&mut buffer, b, entry, world_y);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns the 12-float transforms for building plot/foundation MultiMeshes (visualizing item 53).
    pub fn get_building_plot_transforms_internal(&self, zone_type_int: u8) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if b.zone_type == target_zone {
                let world_x = b.center_x;
                let world_z = b.center_y;
                let world_y = self.heightmap.sample_height_world(world_x, world_z) * 20.0 + 0.02; // Slightly above terrain

                let fd = b.facing_dir.normalized();
                let b_zx = fd.x;
                let b_zz = fd.y;
                let b_xx = fd.y;
                let b_xz = -fd.x;

                // Plot size is 10m * cell count (default 3x3 = 30x30)
                let cell_size = self.config.zone_cell_m;
                let sx = b.width_cells as f32 * cell_size;
                let sz = b.depth_cells as f32 * cell_size;
                let sy = 0.5; // Thin foundation box

                buffer.push(b_xx * sx);
                buffer.push(0.0);
                buffer.push(b_zx * sz);
                buffer.push(world_x);

                buffer.push(0.0);
                buffer.push(sy);
                buffer.push(0.0);
                buffer.push(world_y);

                buffer.push(b_xz * sx);
                buffer.push(0.0);
                buffer.push(b_zz * sz);
                buffer.push(world_z);
            }
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns neutral construction-site transforms for under-construction buildings.
    pub fn get_construction_site_transforms_internal(
        &self,
        zone_type_int: u8,
    ) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if !b.is_under_construction() || b.zone_type != target_zone {
                continue;
            }

            let ground_y = self.heightmap.sample_height_world(b.center_x, b.center_y) * 20.0;
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

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns raised foundation transforms for under-construction buildings.
    pub fn get_construction_foundation_transforms_internal(
        &self,
        zone_type_int: u8,
    ) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if !b.is_under_construction() || b.zone_type != target_zone {
                continue;
            }

            let ground_y = self.heightmap.sample_height_world(b.center_x, b.center_y) * 20.0;
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

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns procedural scaffold bar transforms for under-construction buildings.
    pub fn get_construction_scaffold_transforms_internal(
        &self,
        zone_type_int: u8,
    ) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if !b.is_under_construction() || b.zone_type != target_zone {
                continue;
            }

            let ground_y = self.heightmap.sample_height_world(b.center_x, b.center_y) * 20.0;
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

        PackedFloat32Array::from_iter(buffer)
    }

    fn operational_hour_fraction(&self) -> f32 {
        let seconds_per_minute = self.time.seconds_per_minute().max(f64::EPSILON);
        let minute_fraction = (self.time.time_elapsed / seconds_per_minute).clamp(0.0, 1.0);
        (f64::from(self.time.minute_of_day % 60) + minute_fraction) as f32 / 60.0
    }
}

fn push_building_transform(
    buffer: &mut Vec<f32>,
    building: &Building,
    entry: Option<&AssetEntry>,
    world_y: f32,
) {
    let world_x = building.center_x;
    let world_z = building.center_y;
    let (basis_x, basis_z) =
        building_local_xz_basis(building.facing_dir, main_anchor_forward(entry));
    let b_xx = basis_x.x;
    let b_xz = basis_x.y;
    let b_zx = basis_z.x;
    let b_zz = basis_z.y;
    let s = entry
        .and_then(|e| e.manifest.building.as_ref())
        .and_then(|b| b.preview_scale)
        .unwrap_or(crate::config::BUILDING_VISUAL_SCALE);
    let sx = s;
    let sy = s;
    let sz = s;

    // Pivot offset centres the mesh over the lot and grounds it after scale+rotation.
    let (po_x, po_y, po_z) = entry
        .and_then(|e| e.manifest.pivot_offset)
        .map(|[x, y, z]| (x, y, z))
        .unwrap_or((0.0, 0.0, 0.0));
    let tx = world_x + (b_xx * po_x + b_zx * po_z) * sx;
    let ty = world_y + po_y * sy;
    let tz = world_z + (b_xz * po_x + b_zz * po_z) * sz;

    buffer.push(b_xx * sx);
    buffer.push(0.0);
    buffer.push(b_zx * sz);
    buffer.push(tx);

    buffer.push(0.0);
    buffer.push(sy);
    buffer.push(0.0);
    buffer.push(ty);

    buffer.push(b_xz * sx);
    buffer.push(0.0);
    buffer.push(b_zz * sz);
    buffer.push(tz);
}

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

fn construction_rise_offset_m(building: &Building, progress: f32) -> f32 {
    let t = progress.clamp(0.0, 1.0);
    (1.0 - t) * (construction_scaffold_height_m(building) * 1.35 + 6.0)
}

fn construction_visual_progress(building: &Building, operational_hour_fraction: f32) -> f32 {
    construction_visual_progress_from_hours(
        building.construction_total_hours,
        building.construction_remaining_hours,
        operational_hour_fraction,
    )
}

fn construction_visual_progress_from_hours(
    total_hours: u16,
    remaining_hours: u16,
    operational_hour_fraction: f32,
) -> f32 {
    if total_hours == 0 {
        return 1.0;
    }
    let total = f32::from(total_hours);
    let remaining = f32::from(remaining_hours.min(total_hours));
    let completed_whole_hours = (total - remaining).max(0.0);
    ((completed_whole_hours + operational_hour_fraction.clamp(0.0, 1.0)) / total).clamp(0.0, 1.0)
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

fn main_anchor_forward(entry: Option<&AssetEntry>) -> [f32; 3] {
    entry
        .and_then(|entry| {
            entry
                .manifest
                .anchors
                .iter()
                .find(|anchor| anchor.anchor_type == AnchorType::Entrance && anchor.name == "main")
        })
        .map(|anchor| anchor.forward)
        .unwrap_or([0.0, 0.0, 1.0])
}

fn building_local_xz_basis(facing_dir: Vector2, anchor_forward: [f32; 3]) -> (Vector2, Vector2) {
    let world_front = if facing_dir.length_squared() > 1e-12 {
        facing_dir.normalized()
    } else {
        Vector2::new(0.0, 1.0)
    };
    let local_front = asset_local_front_xz(anchor_forward);
    let world_right = Vector2::new(world_front.y, -world_front.x);
    let basis_x = world_right * local_front.y + world_front * local_front.x;
    let basis_z = world_front * local_front.y - world_right * local_front.x;

    (basis_x, basis_z)
}

fn asset_local_front_xz(anchor_forward: [f32; 3]) -> Vector2 {
    let front = Vector2::new(anchor_forward[0], anchor_forward[2]);
    if front.length_squared() > 1e-12 {
        front.normalized()
    } else {
        Vector2::new(0.0, 1.0)
    }
}

/// Returns the scale factor for a building.
/// Standard assets use 1:10 scale (1 unit = 10m), so we scale by [`crate::config::BUILDING_VISUAL_SCALE`].
pub fn get_building_visual_scale() -> (f32, f32, f32) {
    let s = crate::config::BUILDING_VISUAL_SCALE;
    (s, s, s)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_building_visual_scale_is_adequate() {
        // This test ensures that buildings are not "miniature" by verifying the scale factor
        // returned from our logic is at least 10.0 (the standard for current assets).
        let (sx, sy, sz) = get_building_visual_scale();

        assert!(
            sy >= 10.0,
            "Building vertical scale must be at least 10.0 to match asset scale"
        );
        assert!(
            sx >= 10.0,
            "Building horizontal scale must be at least 10.0 to match asset scale"
        );
        assert!(
            sz >= 10.0,
            "Building depth scale must be at least 10.0 to match asset scale"
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
    fn construction_visual_progress_interpolates_between_hour_ticks() {
        assert!((construction_visual_progress_from_hours(4, 4, 0.0) - 0.0).abs() < 1e-6);
        assert!((construction_visual_progress_from_hours(4, 4, 0.5) - 0.125).abs() < 1e-6);
        assert!((construction_visual_progress_from_hours(4, 3, 0.5) - 0.375).abs() < 1e-6);
        assert!((construction_visual_progress_from_hours(4, 1, 1.0) - 1.0).abs() < 1e-6);
    }
}
