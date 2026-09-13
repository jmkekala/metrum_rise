// SPDX-License-Identifier: GPL-2.0-only

//! Spatial grid subsystems: environmental diffusion and the generic grid primitive.
//!
//! All grids use [`data_grid::DataGrid<T>`] as their storage primitive — a flat `Vec<T>`
//! with row-major indexing and `rayon::par_chunks_mut` parallelism for tick updates.
//!
//! Tick order within the grid module: pollution → noise → desirability.
//! Desirability reads from pollution and noise, so it must run last.

// Expand per field so Rayon closures retain constant coefficients, without runtime weight captures.
// Neighbor order and arithmetic remain identical for both environmental fields.
macro_rules! diffuse_emissions {
    ($target:expr, $previous:expr, $own_weight:expr, $neighbor_weight:expr, $retention:expr) => {{
        let target = &mut $target;
        let previous = &$previous;
        use rayon::prelude::*;
        let w = target.width;
        let h = target.height;
        target
            .data
            .par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..w {
                    let current = *previous.get(x, y).unwrap_or(&0.0);
                    let mut neighbor_sum = 0.0;
                    let mut count = 0.0;
                    if x > 0 {
                        neighbor_sum += *previous.get(x - 1, y).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if x < w - 1 {
                        neighbor_sum += *previous.get(x + 1, y).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if y > 0 {
                        neighbor_sum += *previous.get(x, y - 1).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if y < h - 1 {
                        neighbor_sum += *previous.get(x, y + 1).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    let average = if count > 0.0 {
                        neighbor_sum / count
                    } else {
                        0.0
                    };
                    let propagated =
                        (current * $own_weight + average * $neighbor_weight) * $retention;
                    row[x] = (row[x] + propagated).min(100.0).max(0.0);
                }
            });
    }};
}

/// Generic 2D data grid primitive.
pub mod data_grid;
/// Land value and desirability calculation.
pub mod desirability;
/// Traffic and industrial noise simulation.
pub mod noise;
/// Industrial pollution simulation.
pub mod pollution;

#[cfg(test)]
mod tests {
    use super::{noise::NoiseSystem, pollution::PollutionSystem};
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::network::graph::RegionGraph;

    fn grid_config(width: usize, height: usize) -> WorldConfig {
        // Only environmental buffers are constructed; no full gameplay world is needed.
        WorldConfig {
            width_m: width as f32 * 40.0,
            height_m: height as f32 * 40.0,
            env_cell_m: 40.0,
            ..WorldConfig::default()
        }
    }

    #[test]
    fn environmental_diffusion_retains_distinct_weights_and_edge_neighbors() {
        let allocator = BuildingAllocator::new();
        let graph = RegionGraph::new();
        for (width, height, source, expected_pollution, expected_noise) in [
            (
                3,
                2,
                vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0],
                vec![17.91, 23.88, 33.83, 35.82, 45.77, 51.74],
                vec![18.0, 22.5, 31.5, 31.5, 40.5, 45.0],
            ),
            (1, 1, vec![10.0], vec![5.97], vec![4.5]),
            (1, 2, vec![10.0, 20.0], vec![13.93, 15.92], vec![13.5, 13.5]),
            (2, 1, vec![10.0, 20.0], vec![13.93, 15.92], vec![13.5, 13.5]),
        ] {
            let config = grid_config(width, height);
            let mut pollution = PollutionSystem::new(&config);
            let mut noise = NoiseSystem::new(&config);
            pollution.grid.data.copy_from_slice(&source);
            noise.grid.data.copy_from_slice(&source);
            pollution.tick(&allocator, &config);
            noise.tick(&allocator, &graph, &config);
            for (actual, expected) in pollution
                .grid
                .data
                .iter()
                .zip(expected_pollution)
                .chain(noise.grid.data.iter().zip(expected_noise))
            {
                assert!(
                    (actual - expected).abs() < 0.00001,
                    "{width}x{height}: {actual} != {expected}"
                );
            }
        }
    }

    #[test]
    #[ignore = "manual matched release timing of the two environmental diffusion passes"]
    fn benchmark_environmental_diffusion() {
        use std::{hint::black_box, time::Instant};
        let allocator = BuildingAllocator::new();
        let graph = RegionGraph::new();
        for size in [32, 256, 512] {
            let config = grid_config(size, size);
            let mut pollution = PollutionSystem::new(&config);
            let mut noise = NoiseSystem::new(&config);
            let initial: Vec<f32> = (0..size * size).map(|i| (i % 101) as f32).collect();
            let mut samples = [0.0; 25];
            for sample in &mut samples {
                pollution.grid.data.copy_from_slice(&initial);
                noise.grid.data.copy_from_slice(&initial);
                let start = Instant::now();
                for _ in 0..8 {
                    pollution.tick(black_box(&allocator), black_box(&config));
                    noise.tick(black_box(&allocator), black_box(&graph), black_box(&config));
                }
                *sample = start.elapsed().as_secs_f64() * 1_000.0 / 8.0;
            }
            let checksum =
                pollution
                    .grid
                    .data
                    .iter()
                    .chain(&noise.grid.data)
                    .fold(0_u64, |hash, value| {
                        hash.wrapping_mul(31)
                            .wrapping_add(u64::from(value.to_bits()))
                    });
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "environmental_diffusion size={size} median_ms={:.6} checksum={checksum}",
                samples[12]
            );
        }
    }
}
