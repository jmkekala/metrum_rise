// SPDX-License-Identifier: GPL-2.0-only

//! Which water a view ray first comes down onto, for the planar water reflection's choice of
//! mirror planes (`water_reflection.gd`).
//!
//! The reflection mirrors the scene through the plane of the water the camera sees, so it needs
//! that water's drawn surface height. The mist map's levels do not serve: each of its cells takes
//! the highest level among its neighbours, which on a river stepping down its valley reads up to
//! a metre above the surface drawn there, and its rays could not see land, so a lake behind a
//! hill polled like the lake in view.

use crate::simulation::terrain::graded::{GradedSurface, QueryScratch};
use crate::simulation::water::WaterSystem;
use godot::prelude::Vector3;

// Water shallower than this is a wet margin, not water anything reflects in; the mist's
// open-water threshold.
const MIN_WATER_DEPTH_M: f32 = 0.05;
// March step across the ground, in metres: under half a 10 m terrain cell, so no bank a ray
// crosses is stepped over.
const STEP_M: f32 = 4.0;

/// Height of the visible water surface (graded terrain plus visible depth, as the water renderer
/// draws it) where the ray from `origin` along unit `direction` first comes down onto water, or
/// NaN where it meets land first, rises, or meets nothing within `max_distance_m` along it.
///
/// Steps `STEP_M` across the ground, at most four times that along a steep ray, so a crossing is
/// found within one step: O(`max_distance_m` / `STEP_M`) terrain and depth samples.
pub(crate) fn first_water_surface(
    water: &WaterSystem,
    surface: &GradedSurface<'_>,
    scratch: &mut QueryScratch,
    origin: Vector3,
    direction: Vector3,
    max_distance_m: f32,
) -> f32 {
    // A level or rising ray never comes down onto water below the camera.
    if direction.y >= -0.001 {
        return f32::NAN;
    }
    let horizontal = (direction.x * direction.x + direction.z * direction.z).sqrt();
    let step = STEP_M / horizontal.max(0.25);
    let mut t = 0.0;
    while t < max_distance_m {
        t += step;
        let p = origin + direction * t;
        let ground = surface.ground_height_at(f64::from(p.x), f64::from(p.z), scratch) as f32;
        let depth = water.visible_depth_world(p.x, p.z);
        let wet = depth > MIN_WATER_DEPTH_M;
        let water_or_ground = if wet { ground + depth } else { ground };
        if p.y <= water_or_ground {
            if !wet {
                return f32::NAN;
            }
            // Raw ground + depth rises toward a dry bank, while the drawn dry vertex takes
            // its wet neighbours' level. Refine only a candidate crossing, not every march step.
            let drawn = display_water_surface(water, surface, scratch, p.x, p.z);
            if p.y <= ground.max(drawn) {
                return if ground < drawn { drawn } else { f32::NAN };
            }
        }
    }
    f32::NAN
}

/// Water height reconstructed from the shader's wet-neighbour smoothing at terrain-grid
/// vertices, then interpolated across the water mesh's diagonal. Returns NaN on dry land.
/// O(1) indexed samples; shared by the reflection vote and camera clearance.
pub(crate) fn display_water_surface(
    water: &WaterSystem,
    surface: &GradedSurface<'_>,
    scratch: &mut QueryScratch,
    world_x: f32,
    world_z: f32,
) -> f32 {
    if water.visible_depth_world(world_x, world_z) <= 0.001 {
        return f32::NAN;
    }
    let (gx, gz) = surface.terrain.world_to_grid_coords(world_x, world_z);
    let x0 = gx.floor() as usize;
    let z0 = gz.floor() as usize;
    let tx = gx.fract();
    let tz = gz.fract();
    let mut vertex = |x, z| {
        let (wx, wz) = surface.terrain.grid_to_world_coords(x, z);
        smoothed_water_vertex(water, surface, scratch, wx, wz)
    };
    let a = vertex(x0, z0);
    let d = vertex(x0 + 1, z0 + 1);
    // The top-left to bottom-right diagonal matches WaterPatchMeshBuilder's triangles.
    if tx >= tz {
        let b = vertex(x0 + 1, z0);
        a + (b - a) * tx + (d - b) * tz
    } else {
        let c = vertex(x0, z0 + 1);
        a + (d - c) * tx + (c - a) * tz
    }
}

fn smoothed_water_vertex(
    water: &WaterSystem,
    surface: &GradedSurface<'_>,
    scratch: &mut QueryScratch,
    x: f32,
    z: f32,
) -> f32 {
    let cell = surface.terrain.cell_size_m();
    let ground = surface.ground_height_at(f64::from(x), f64::from(z), scratch) as f32;
    let depth = water.visible_depth_world(x, z);
    let mut sum = 0.0;
    let mut weight = 0.0;
    for (dx, dz, w) in [
        (0.0, 0.0, 2.0),
        (-cell, 0.0, 1.0),
        (cell, 0.0, 1.0),
        (0.0, -cell, 1.0),
        (0.0, cell, 1.0),
    ] {
        let d = water.visible_depth_world(x + dx, z + dz);
        if d > 0.001 {
            sum += (surface.ground_height_at(
                f64::from(x + dx),
                f64::from(z + dz),
                scratch,
            ) as f32
                + d)
                * w;
            weight += w;
        }
    }
    if weight == 0.0 {
        for (dx, dz) in [(-cell, -cell), (cell, -cell), (-cell, cell), (cell, cell)] {
            let d = water.visible_depth_world(x + dx, z + dz);
            if d > 0.001 {
                sum += surface.ground_height_at(
                    f64::from(x + dx),
                    f64::from(z + dz),
                    scratch,
                ) as f32
                    + d;
                weight += 1.0;
            }
        }
    }
    if weight == 0.0 {
        ground
    } else if depth > 0.001 {
        let center = ground + depth;
        center + (sum / weight - center) * 0.94
    } else {
        sum / weight
    }
}

// Keep the budget identical to the renderer's 14 x 12 screen vote. Selection uses stack storage.
const MAX_VOTE_RAYS: usize = 168;
const PLANE_TOLERANCE_M: f32 = 0.5;
const KEEP_SHARE: f32 = 0.75;

/// Chooses up to two distinct screen-share leaders, preserving each incumbent's slot and exact
/// height until a challenger exceeds its share by one third. O(rays²), with no heap allocation.
pub(crate) fn select_planes(levels: &[f32], weights: &[f32], previous: &[f32]) -> [f32; 2] {
    let mut bins = [(f32::NAN, 0.0_f32, 0.0_f32); MAX_VOTE_RAYS];
    let mut count = 0;
    for (&level, &weight) in levels.iter().zip(weights).take(MAX_VOTE_RAYS) {
        if !level.is_finite() || weight <= 0.0 {
            continue;
        }
        let key = (level / 0.25).round();
        let index = (0..count).find(|&i| bins[i].0 == key).unwrap_or(count);
        if index == count {
            bins[index].0 = key;
            count += 1;
        }
        bins[index].1 += weight;
        bins[index].2 += level * weight;
    }
    let score = |plane: f32| {
        bins[..count]
            .iter()
            .filter(|bin| (bin.2 / bin.1 - plane).abs() < PLANE_TOLERANCE_M)
            .map(|bin| bin.1)
            .sum::<f32>()
    };
    let mut chosen = [f32::NAN; 2];
    for (slot, &level) in previous.iter().take(2).enumerate() {
        if level.is_finite()
            && score(level) > 0.0
            && !chosen
                .iter()
                .any(|&p| (p - level).abs() < PLANE_TOLERANCE_M)
        {
            chosen[slot] = level;
        }
    }
    // Fill vacancies first, then compare a challenger against the weakest selected lake,
    // never against the first mirror's leader. Stable slots also keep textures on their lake.
    for _ in 0..2 {
        let mut best = (f32::NAN, 0.0, 0.0);
        for bin in &bins[..count] {
            let level = bin.2 / bin.1;
            if chosen
                .iter()
                .any(|&p| (p - level).abs() < PLANE_TOLERANCE_M)
            {
                continue;
            }
            let share = score(level);
            if share > best.1
                || (share == best.1 && (bin.1 > best.2 || (bin.1 == best.2 && level < best.0)))
            {
                best = (level, share, bin.1);
            }
        }
        if !best.0.is_finite() {
            break;
        }
        let slot = chosen.iter().position(|p| p.is_nan()).unwrap_or_else(|| {
            if score(chosen[0]) <= score(chosen[1]) {
                0
            } else {
                1
            }
        });
        if chosen[slot].is_nan() || score(chosen[slot]) < best.1 * KEEP_SHARE {
            chosen[slot] = best.0;
        } else {
            break;
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HEIGHT_SCALE;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::roadbed::RoadbedSystem;
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::terrain::graded::NoSites;
    use crate::simulation::core::config::WorldConfig;

    // A flat world with a lake two metres deep over its western half.
    fn lake_world() -> (TerrainSystem, WaterSystem) {
        let config = WorldConfig::new(2048.0, 2048.0, 16.0, 16.0);
        let terrain = TerrainSystem::from_world_config(&config);
        let mut water = WaterSystem::from_world_config(&config);
        let mut depth = water.clone_baseline_depth_dense();
        let (width, height) = (water.width, water.height);
        for z in 0..height {
            for x in 0..width / 2 {
                depth[z * width + x] = 2.0;
            }
        }
        water.replace_baseline_depth_from_dense(&depth).unwrap();
        (terrain, water)
    }

    #[test]
    fn planes_keep_both_small_lakes_and_their_slots_through_rank_changes() {
        let levels = [86.866, 137.095, 150.0];
        // The saved high view's lakes each take less than the old 7% cutoff.
        let mut chosen = select_planes(&levels, &[0.03, 0.013, 0.001], &[]);
        assert_eq!(chosen, [86.866, 137.095]);
        for weights in [[0.012, 0.032, 0.001], [0.033, 0.013, 0.001]] {
            chosen = select_planes(&levels, &weights, &chosen);
            assert_eq!(chosen, [86.866, 137.095]);
        }
        // A genuine challenger replaces only the weakest incumbent and stays on that slot.
        chosen = select_planes(&levels, &[0.03, 0.01, 0.02], &chosen);
        assert_eq!(chosen, [86.866, 150.0]);
        assert_eq!(
            select_planes(&levels, &[0.03, 0.015, 0.02], &chosen),
            chosen
        );
    }

    #[test]
    fn plane_height_does_not_follow_a_winning_bin_or_drift() {
        let mut chosen = [137.10, 86.866];
        for frame in 0..100 {
            let levels = if frame % 2 == 0 {
                [137.11, 137.32, 86.866]
            } else {
                [137.34, 137.09, 86.866]
            };
            chosen = select_planes(&levels, &[0.3, 0.1, 0.03], &chosen);
            assert_eq!(chosen, [137.10, 86.866]);
        }
        // Empty votes retire a lake, and exact ties are deterministic under input permutation.
        assert!(select_planes(&[], &[], &chosen).iter().all(|p| p.is_nan()));
        assert_eq!(select_planes(&[20.0, 10.0], &[0.1, 0.1], &[]), [10.0, 20.0]);
        assert_eq!(select_planes(&[10.0, 20.0], &[0.1, 0.1], &[]), [10.0, 20.0]);
    }

    #[test]
    fn drawn_water_level_stays_flat_beside_a_dry_high_bank() {
        let config = WorldConfig::new(128.0, 128.0, 16.0, 16.0).with_terrain_resolution(8.0);
        let mut terrain = TerrainSystem::from_world_config(&config);
        let mut water = WaterSystem::from_world_config(&config);
        let mut ground = terrain.clone_source_dense();
        let mut depths = water.clone_baseline_depth_dense();
        for z in 0..terrain.height {
            for x in 0..terrain.width {
                ground[z * terrain.width + x] = if x < 8 { 0.0 } else { 4.0 / HEIGHT_SCALE };
                depths[z * terrain.width + x] = if x < 8 { 2.0 } else { 0.0 };
            }
        }
        terrain.replace_source_from_dense(&ground).unwrap();
        water.replace_baseline_depth_from_dense(&depths).unwrap();
        let roadbed = RoadbedSystem::new(RegionGraph::CHUNK_SIZE);
        let surface = GradedSurface {
            terrain: &terrain,
            roadbed: &roadbed,
            sites: &NoSites,
            query_index: None,
        };
        let mut scratch = QueryScratch::default();
        assert!(
            (display_water_surface(&water, &surface, &mut scratch, -6.0, 1.0) - 2.0).abs()
                < 1e-4
        );
        // Raw terrain + interpolated depth reads 3 m here; the rendered shore is at 2 m.
        assert!(
            (display_water_surface(&water, &surface, &mut scratch, -4.0, 1.0) - 2.0).abs()
                < 1e-4
        );
        let level = first_water_surface(
            &water,
            &surface,
            &mut scratch,
            Vector3::new(-6.0, 3.0, 1.0),
            Vector3::DOWN,
            8.0,
        );
        assert!((level - 2.0).abs() < 1e-4);
        assert!(display_water_surface(&water, &surface, &mut scratch, 16.0, 1.0).is_nan());
    }

    #[test]
    fn a_ray_onto_the_lake_reads_its_surface_and_one_onto_land_reads_none() {
        let (terrain, water) = lake_world();
        let ground = terrain.sample_height_world(-500.0, 0.0) * HEIGHT_SCALE;
        let roadbed = RoadbedSystem::new(RegionGraph::CHUNK_SIZE);
        let surface = GradedSurface {
            terrain: &terrain,
            roadbed: &roadbed,
            sites: &NoSites,
            query_index: None,
        };
        let mut scratch = QueryScratch::default();
        let origin = Vector3::new(0.0, ground + 100.0, 0.0);
        let west = Vector3::new(-1.0, -0.2, 0.0).normalized();
        let level = first_water_surface(&water, &surface, &mut scratch, origin, west, 3000.0);
        assert!((level - (ground + 2.0)).abs() < 1e-3, "lake read {level}");
        let east = Vector3::new(1.0, -0.2, 0.0).normalized();
        assert!(
            first_water_surface(&water, &surface, &mut scratch, origin, east, 3000.0).is_nan()
        );
        // A ray that ends before it comes down reads nothing.
        assert!(
            first_water_surface(&water, &surface, &mut scratch, origin, west, 50.0).is_nan()
        );
    }
}
