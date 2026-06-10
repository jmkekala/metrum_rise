//! Building-specific rendering logic for Godot interaction.
//!
//! Handles building instance transform generation and plot/foundation visuals.

use crate::assets::{AnchorType, AssetEntry};
use crate::nodes::sim::core::SimCore;
use crate::simulation::buildings::allocator::Building;
use crate::simulation::zoning::ZoneType;
use godot::prelude::*;

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
                if b.broken || b.is_deserted || b.is_under_construction() || b.asset_id != asset_id
                {
                    continue;
                }
            }

            let world_x = b.center_x;
            let world_z = b.center_y;
            let world_y = self.heightmap.sample_height_world(world_x, world_z) * 20.0;
            let entry = self.allocator.registry.get(asset_id);
            push_building_transform(&mut buffer, b, entry, world_y, 1.0);
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
            push_building_transform(&mut buffer, b, entry, world_y, 1.0);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns the 12-float transforms for under-construction buildings with the given asset ID.
    ///
    /// The transform uses the final asset mesh with a vertical reveal scale, while the generic
    /// construction-site slab is returned by [`Self::get_construction_site_transforms_internal`].
    pub fn get_construction_building_transforms_for_asset_internal(
        &self,
        asset_id: &str,
    ) -> PackedFloat32Array {
        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if b.broken || !b.is_under_construction() || b.asset_id != asset_id {
                continue;
            }

            let world_x = b.center_x;
            let world_z = b.center_y;
            let world_y = self.heightmap.sample_height_world(world_x, world_z) * 20.0;
            let entry = self.allocator.registry.get(asset_id);
            push_building_transform(
                &mut buffer,
                b,
                entry,
                world_y,
                construction_reveal_y_scale(b.construction_progress()),
            );
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

    /// Returns generic construction-site slab transforms for under-construction buildings.
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

            let world_x = b.center_x;
            let world_z = b.center_y;
            let world_y = self.heightmap.sample_height_world(world_x, world_z) * 20.0 + 0.04;

            let fd = b.facing_dir.normalized();
            let b_zx = fd.x;
            let b_zz = fd.y;
            let b_xx = fd.y;
            let b_xz = -fd.x;

            let cell_size = self.config.zone_cell_m;
            let sx = b.width_cells as f32 * cell_size;
            let sz = b.depth_cells as f32 * cell_size;
            let sy = 0.65;

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

        PackedFloat32Array::from_iter(buffer)
    }
}

fn push_building_transform(
    buffer: &mut Vec<f32>,
    building: &Building,
    entry: Option<&AssetEntry>,
    world_y: f32,
    y_scale_factor: f32,
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
    let sy = s * y_scale_factor.clamp(0.0, 1.0);
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

fn construction_reveal_y_scale(progress: f32) -> f32 {
    let t = progress.clamp(0.0, 1.0);
    0.08 + 0.92 * t * t * (3.0 - 2.0 * t)
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
}
