//! Building-specific rendering logic for Godot interaction.
//!
//! Handles building instance transform generation and plot/foundation visuals.

use crate::nodes::sim::core::SimCore;
use crate::simulation::grid::zoning::ZoneType;
use godot::prelude::*;

impl SimCore {
    // ── Building Renderer ──

    /// Returns the 12-float transforms for all placed buildings with the given asset ID.
    pub fn get_building_transforms_for_asset_internal(&self, asset_id: &str) -> PackedFloat32Array {
        let mut buffer = Vec::new();
        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for b in &self.allocator.buildings {
            if asset_id == "broken:error" {
                if !b.broken {
                    continue;
                }
            } else {
                if b.broken || b.asset_id != asset_id {
                    continue;
                }
            }

            let world_x = b.center_x;
            let world_z = b.center_y;

            let grid_x = b.center_x + hw;
            let grid_y = b.center_y + hh;
            let safe_gx = grid_x.round().clamp(0.0, w - 1.0) as usize;
            let safe_gy = grid_y.round().clamp(0.0, h - 1.0) as usize;

            let world_y = self.heightmap.get_height(safe_gx, safe_gy) * 20.0;

            let fd = b.facing_dir.normalized();
            let b_zx = fd.x;
            let b_zz = fd.y;
            let b_xx = fd.y;
            let b_xz = -fd.x;

            // Scale: prefer per-asset preview_scale, fall back to BUILDING_VISUAL_SCALE.
            let entry = self.allocator.registry.get(asset_id);
            let s = entry
                .and_then(|e| e.manifest.building.as_ref())
                .and_then(|b| b.preview_scale)
                .unwrap_or(crate::config::BUILDING_VISUAL_SCALE);
            let (sx, sy, sz) = (s, s, s);

            // Pivot offset: centres the mesh over the lot cell and grounds it at Y=0.
            // Stored in model units; applied here in world space after scale+rotation.
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
        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for b in &self.allocator.buildings {
            if b.zone_type == target_zone {
                let world_x = b.center_x;
                let world_z = b.center_y;

                let grid_x = b.center_x + hw;
                let grid_y = b.center_y + hh;
                let safe_gx = grid_x.round().clamp(0.0, w - 1.0) as usize;
                let safe_gy = grid_y.round().clamp(0.0, h - 1.0) as usize;

                let world_y = self.heightmap.get_height(safe_gx, safe_gy) * 20.0 + 0.02; // Slightly above terrain

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
}
