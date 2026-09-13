// SPDX-License-Identifier: GPL-2.0-only

//! World-aligned pollution, noise and desirability image buffers for Godot overlays.

use crate::nodes::sim::core::SimCore;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::grid::data_grid::DataGrid;
use godot::prelude::*;
use rayon::prelude::*;

impl SimCore {
    fn environment_image_data(&self, grid: &DataGrid<f32>, color: [u8; 3]) -> PackedByteArray {
        let pixels = grid_image_pixels(
            grid,
            &self.config,
            self.heightmap.world_size(),
            (self.heightmap.width, self.heightmap.height),
            color,
        );
        PackedByteArray::from(pixels.as_slice())
    }

    /// Returns the pollution image data as a PackedByteArray (RGBA8).
    pub fn get_pollution_image_data_internal(&self) -> PackedByteArray {
        self.environment_image_data(&self.pollution.grid, [255, 50, 50])
    }

    /// Returns the noise image data as a PackedByteArray (RGBA8).
    pub fn get_noise_image_data_internal(&self) -> PackedByteArray {
        self.environment_image_data(&self.noise.grid, [200, 200, 200])
    }

    /// Returns the desirability image data as a PackedByteArray (RGBA8).
    pub fn get_desirability_image_data_internal(&self) -> PackedByteArray {
        self.environment_image_data(&self.desirability.grid, [50, 255, 50])
    }
}

fn grid_image_pixels(
    grid: &DataGrid<f32>,
    config: &WorldConfig,
    (world_width_m, world_height_m): (f32, f32),
    (target_w, target_h): (usize, usize),
    [r, g, b]: [u8; 3],
) -> Vec<u8> {
    let mut pixels = vec![0; target_w * target_h * 4];
    if pixels.is_empty() || grid.data.is_empty() {
        return pixels;
    }
    let pixel_width_m = world_width_m / target_w as f32;
    let pixel_height_m = world_height_m / target_h as f32;
    // Reuse each column's world-to-grid conversion across all image rows.
    let grid_columns: Vec<f32> = (0..target_w)
        .map(|x| {
            let world_x = (x as f32 + 0.5) * pixel_width_m - world_width_m * 0.5;
            config.world_to_env_grid(world_x, 0.0).0
        })
        .collect();
    // Texture UVs address pixel centres; environmental cells use their authored metre spacing,
    // including worlds whose rounded grid dimensions do not cover the exact physical extent.
    pixels
        .par_chunks_mut(target_w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let world_y = (y as f32 + 0.5) * pixel_height_m - world_height_m * 0.5;
            let (_, grid_y) = config.world_to_env_grid(0.0, world_y);
            for (pixel, &grid_x) in row.chunks_exact_mut(4).zip(&grid_columns) {
                let val = grid.sample_bilinear(grid_x, grid_y);
                if val <= 0.01 {
                    continue;
                }
                let alpha = ((val / 100.0).clamp(0.0, 1.0) * 200.0) as u8;
                pixel.copy_from_slice(&[r, g, b, alpha]);
            }
        });
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_pixels_follow_world_cell_centres() {
        let config = WorldConfig::new(40.0, 40.0, 20.0, 10.0);
        let mut grid = DataGrid::new(2, 2, 0.0);
        grid.data.copy_from_slice(&[0.0, 100.0, 50.0, 100.0]);
        let pixels = grid_image_pixels(
            &grid,
            &config,
            (config.width_m, config.height_m),
            (4, 4),
            [255, 50, 50],
        );
        // Pixel centres are at -15, -5, 5, 15 metres; source centres are at -10 and 10.
        let expected_alpha = [
            0, 50, 150, 200, 25, 68, 156, 200, 75, 106, 168, 200, 100, 125, 175, 200,
        ];
        for (pixel, alpha) in pixels.chunks_exact(4).zip(expected_alpha) {
            let expected = if alpha == 0 {
                [0, 0, 0, 0]
            } else {
                [255, 50, 50, alpha]
            };
            assert_eq!(pixel, expected);
        }

        // Rounded source dimensions must not stretch the 20-metre cells to fit this world.
        let config = WorldConfig::new(45.0, 35.0, 20.0, 10.0);
        grid.data.copy_from_slice(&[10.0, 20.0, 30.0, 40.0]);
        let pixels = grid_image_pixels(
            &grid,
            &config,
            (config.width_m, config.height_m),
            (9, 7),
            [50, 255, 50],
        );
        for (x, y, alpha) in [(0, 0, 20), (4, 3, 47), (8, 0, 40), (8, 6, 80)] {
            let start = (y * 9 + x) * 4;
            assert_eq!(&pixels[start..start + 4], &[50, 255, 50, alpha]);
        }

        // Terrain rounds 45 × 35 authored metres to a 50 × 40 rendered extent.
        // The shader's UVs span that rendered extent, while environmental cells retain
        // their authored origin. Pixel (1, 1) is at world (-12.5, -8), inside cell (0, 0).
        let pixels = grid_image_pixels(
            &grid,
            &config,
            (50.0, 40.0),
            config.get_terrain_grid_size(),
            [50, 255, 50],
        );
        assert_eq!(&pixels[(6 + 1) * 4..(6 + 2) * 4], &[50, 255, 50, 20]);
    }

    #[test]
    fn environment_pixels_preserve_empty_threshold_and_saturation_rules() {
        let config = WorldConfig::new(40.0, 40.0, 20.0, 10.0);
        for (value, expected) in [
            (-10.0, [0, 0, 0, 0]),
            (0.01, [0, 0, 0, 0]),
            (0.5, [200, 200, 200, 1]),
            (150.0, [200, 200, 200, 200]),
        ] {
            let grid = DataGrid::new(1, 1, value);
            let pixels = grid_image_pixels(
                &grid,
                &config,
                (config.width_m, config.height_m),
                (3, 2),
                [200; 3],
            );
            assert_eq!(pixels, expected.repeat(6));
            assert!(
                grid_image_pixels(
                    &grid,
                    &config,
                    (config.width_m, config.height_m),
                    (0, 2),
                    [200; 3]
                )
                .is_empty()
            );
            assert!(
                grid_image_pixels(
                    &grid,
                    &config,
                    (config.width_m, config.height_m),
                    (2, 0),
                    [200; 3]
                )
                .is_empty()
            );
        }
        let empty = DataGrid::new(0, 0, 0.0);
        assert_eq!(
            grid_image_pixels(
                &empty,
                &config,
                (config.width_m, config.height_m),
                (2, 2),
                [200; 3]
            ),
            vec![0; 16]
        );
    }

    #[test]
    #[ignore = "unprofiled environmental raster scaling; run alone with --release --ignored --nocapture"]
    fn benchmark_environmental_overlay_raster() {
        use std::hint::black_box;
        use std::time::Instant;
        for world_m in [1_280.0, 10_240.0, 20_000.0] {
            let config = WorldConfig::new(world_m, world_m, 40.0, 10.0);
            let mut grid = DataGrid::new(config.env_grid_width(), config.env_grid_height(), 0.0);
            for (index, value) in grid.data.iter_mut().enumerate() {
                *value = ((index % grid.width * 17 + index / grid.width * 31) % 121) as f32 - 10.0;
            }
            let dimensions = config.get_terrain_grid_size();
            let mut samples = Vec::with_capacity(21);
            let mut checksum = 0u64;
            for sample in 0..24 {
                let start = Instant::now();
                for _ in 0..4 {
                    black_box(grid_image_pixels(
                        black_box(&grid),
                        &config,
                        (config.width_m, config.height_m),
                        dimensions,
                        [255, 50, 50],
                    ));
                }
                let elapsed = start.elapsed().as_secs_f64() * 1_000.0 / 4.0;
                if sample >= 3 {
                    samples.push(elapsed);
                }
            }
            for (index, value) in grid_image_pixels(
                &grid,
                &config,
                (config.width_m, config.height_m),
                dimensions,
                [255, 50, 50],
            )
            .iter()
            .enumerate()
            {
                checksum =
                    checksum.wrapping_add((index as u64 + 1).wrapping_mul(u64::from(*value)));
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "environment_raster width={} height={} median_ms={:.6} checksum={checksum}",
                dimensions.0,
                dimensions.1,
                samples[samples.len() / 2]
            );
        }
    }
}
