//! Zoning and environmental overlay rendering logic for Godot interaction.
//!
//! Handles the conversion of 2D data grids (zoning, pollution, noise, desirability)
//! into Godot-compatible image buffers for shader-based overlays.

use crate::nodes::sim::core::SimCore;
use godot::prelude::*;

impl SimCore {
    // ── Zoning & Environment Renderer ──

    /// Helper: converts a `DataGrid<f32>` to an upsampled `PackedByteArray` for Godot ImageTexture.
    pub fn grid_to_image_data_internal(
        grid: &crate::simulation::grid::data_grid::DataGrid<f32>,
        target_w: usize,
        target_h: usize,
        r: u8,
        g: u8,
        b: u8,
        alpha_max: u8,
        max_val: f32,
    ) -> PackedByteArray {
        let mut pixels = Vec::with_capacity(target_w * target_h * 4);
        let scale_x = grid.width as f32 / target_w as f32;
        let scale_y = grid.height as f32 / target_h as f32;

        for y in 0..target_h {
            for x in 0..target_w {
                let val = grid.sample_bilinear(x as f32 * scale_x, y as f32 * scale_y);
                if val <= 0.01 {
                    pixels.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    let alpha = ((val / max_val).clamp(0.0, 1.0) * alpha_max as f32) as u8;
                    pixels.extend_from_slice(&[r, g, b, alpha]);
                }
            }
        }
        PackedByteArray::from_iter(pixels)
    }

    /// Returns the pollution image data as a PackedByteArray (RGBA8).
    pub fn get_pollution_image_data_internal(&self) -> PackedByteArray {
        Self::grid_to_image_data_internal(
            &self.pollution.grid,
            self.heightmap.width,
            self.heightmap.height,
            255, 50, 50, 200, 100.0,
        )
    }

    /// Returns the noise image data as a PackedByteArray (RGBA8).
    pub fn get_noise_image_data_internal(&self) -> PackedByteArray {
        Self::grid_to_image_data_internal(
            &self.noise.grid,
            self.heightmap.width,
            self.heightmap.height,
            200, 200, 200, 200, 100.0,
        )
    }

    /// Returns the desirability image data as a PackedByteArray (RGBA8).
    pub fn get_desirability_image_data_internal(&self) -> PackedByteArray {
        Self::grid_to_image_data_internal(
            &self.desirability.grid,
            self.heightmap.width,
            self.heightmap.height,
            50, 255, 50, 200, 100.0,
        )
    }
}
