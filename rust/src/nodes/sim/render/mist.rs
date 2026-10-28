// SPDX-License-Identifier: GPL-2.0-only

//! Mist map: where morning mist can lie over the world, and the water level it lies on.
//!
//! Mist forms over open water and spills a little way onto the shore, so the renderer needs, at
//! every point, how strongly mist may lie there and the level of the water it rises from. This
//! is a coarse derived grid over the whole world, rebuilt only when the visible water or the
//! terrain changes; it never feeds the simulation.

use crate::config::HEIGHT_SCALE;
use crate::nodes::sim::core::SimCore;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use rayon::prelude::*;

/// Edge of one mist cell, in metres. Fine enough for bays and narrow sounds, coarse enough that
/// the whole default world is under a quarter of a million cells.
pub(crate) const MIST_CELL_M: f32 = 32.0;
// How far mist spills from the water's edge onto the shore, in metres.
const SHORE_REACH_M: f32 = 128.0;
// Water shallower than this is a wet margin, not open water.
const MIN_WATER_DEPTH_M: f32 = 0.05;

/// Mist presence and water level per cell, row-major from the world's minimum corner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MistMap {
    /// Cells along X.
    pub width: usize,
    /// Cells along Z.
    pub height: usize,
    /// World X/Z of cell (0, 0)'s centre.
    pub origin: [f32; 2],
    /// Per cell: presence in `[0, 1]` (1 over open water, falling to 0 at the shore reach), then
    /// the level in metres of the water it lies on, the nearest water's for a shore cell.
    pub data: Vec<f32>,
}

// Shortest interval between rebuilds while the water or terrain keeps changing, as a brush
// stroke does every frame: a stroke then costs one rebuild per interval, not one per frame, and
// the mist catches up within it. A new world rebuilds at once.
const MIST_REBUILD_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

impl SimCore {
    /// Rebuilds the mist map when the world, its visible water or its visual terrain has
    /// changed since it was built, at most once per `MIST_REBUILD_INTERVAL` within one world.
    /// Snapshots share the result.
    pub(crate) fn refresh_mist_map(&mut self) {
        let key = [
            self.world_epoch,
            self.watermap.render_generation(),
            self.heightmap.source_generation(),
            self.heightmap.source_generation(),
        ];
        if key == self.mist_map_key {
            return;
        }
        let same_world = key[0] == self.mist_map_key[0];
        if same_world
            && self
                .mist_map_built_at
                .is_some_and(|built| built.elapsed() < MIST_REBUILD_INTERVAL)
        {
            return;
        }
        self.mist_map = std::sync::Arc::new(build_mist_map(&self.watermap, &self.heightmap));
        self.mist_map_key = key;
        self.mist_map_built_at = Some(std::time::Instant::now());
    }
}

/// Builds the map from the visible water and the base terrain its depth is measured on. O(cells x reach² / cell²):
/// one depth and height sample per cell, then a bounded search of the nearby cells for the
/// nearest water, in parallel rows.
pub(crate) fn build_mist_map(water: &WaterSystem, terrain: &TerrainSystem) -> MistMap {
    let (world_w, world_h) = water.world_size();
    let width = (world_w / MIST_CELL_M).ceil() as usize + 1;
    let height = (world_h / MIST_CELL_M).ceil() as usize + 1;
    let origin = [-world_w * 0.5, -world_h * 0.5];
    let position = |x: usize, z: usize| {
        (
            origin[0] + x as f32 * MIST_CELL_M,
            origin[1] + z as f32 * MIST_CELL_M,
        )
    };
    // The water level over each open-water cell, or NaN where there is none.
    let mut level = vec![f32::NAN; width * height];
    level
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(z, row)| {
            for (x, cell) in row.iter_mut().enumerate() {
                let (wx, wz) = position(x, z);
                let depth = water.visible_depth_world(wx, wz);
                if depth > MIN_WATER_DEPTH_M {
                    // Water depth is on the base (`TERRAIN-07` T13 (a)), so its surface is the
                    // base plus the depth, whatever the graded ground does there.
                    *cell = terrain.sample_height_world(wx, wz) * HEIGHT_SCALE + depth;
                }
            }
        });
    // A lake's surface is flat, but its depth is interpolated, so a cell straddling the shoreline
    // reads the surface too low. Each water cell takes the highest level among its open-water
    // neighbours, which is the surface itself.
    let surface: Vec<f32> = (0..width * height)
        .into_par_iter()
        .map(|i| {
            let here = level[i];
            if here.is_nan() {
                return here;
            }
            let (x, z) = ((i % width) as isize, (i / width) as isize);
            let mut top = here;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let (nx, nz) = (x + dx, z + dz);
                    if nx >= 0 && nz >= 0 && nx < width as isize && nz < height as isize {
                        let neighbour = level[nz as usize * width + nx as usize];
                        if neighbour > top {
                            top = neighbour;
                        }
                    }
                }
            }
            top
        })
        .collect();
    let level = surface;
    let reach = (SHORE_REACH_M / MIST_CELL_M).ceil() as isize;
    let mut data = vec![0.0; width * height * 2];
    data.par_chunks_mut(width * 2)
        .enumerate()
        .for_each(|(z, row)| {
            for x in 0..width {
                // The nearest open-water cell within reach; ties keep the first in scan order.
                let mut nearest: Option<(f32, f32)> = None;
                for dz in -reach..=reach {
                    let nz = z as isize + dz;
                    if nz < 0 || nz >= height as isize {
                        continue;
                    }
                    for dx in -reach..=reach {
                        let nx = x as isize + dx;
                        if nx < 0 || nx >= width as isize {
                            continue;
                        }
                        let water_level = level[nz as usize * width + nx as usize];
                        if water_level.is_nan() {
                            continue;
                        }
                        let distance = ((dx * dx + dz * dz) as f32).sqrt() * MIST_CELL_M;
                        if nearest.is_none_or(|(best, _)| distance < best) {
                            nearest = Some((distance, water_level));
                        }
                    }
                }
                if let Some((distance, water_level)) = nearest {
                    let t = (1.0 - distance / SHORE_REACH_M).clamp(0.0, 1.0);
                    row[x * 2] = t * t * (3.0 - 2.0 * t);
                    row[x * 2 + 1] = water_level;
                }
            }
        });
    MistMap {
        width,
        height,
        origin,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::core::config::WorldConfig;

    #[test]
    fn mist_lies_on_water_and_fades_over_the_shore() {
        let config = WorldConfig::new(2048.0, 2048.0, 16.0, 16.0);
        let terrain = TerrainSystem::from_world_config(&config);
        let mut water = WaterSystem::from_world_config(&config);
        // A lake two metres deep over the western half of the world.
        let mut depth = water.clone_baseline_depth_dense();
        let (width, height) = (water.width, water.height);
        for z in 0..height {
            for x in 0..width / 2 {
                depth[z * width + x] = 2.0;
            }
        }
        water.replace_baseline_depth_from_dense(&depth).unwrap();
        let map = build_mist_map(&water, &terrain);
        assert_eq!(map, build_mist_map(&water, &terrain));
        let cell = |wx: f32| {
            let x = ((wx - map.origin[0]) / MIST_CELL_M).round() as usize;
            let i = (map.height / 2 * map.width + x) * 2;
            (map.data[i], map.data[i + 1])
        };
        let ground = terrain.sample_height_world(-500.0, 0.0) * HEIGHT_SCALE;
        let (presence, level) = cell(-500.0);
        assert_eq!(presence, 1.0);
        assert!((level - (ground + 2.0)).abs() < 1e-3);
        let (near_shore, near_level) = cell(64.0);
        assert!(near_shore > 0.0 && near_shore < 1.0);
        assert_eq!(near_level, level);
        assert_eq!(cell(500.0).0, 0.0);
    }

    #[test]
    fn a_new_world_rebuilds_at_once_and_edits_within_one_are_throttled() {
        let mut core = SimCore::new(WorldConfig::new(1024.0, 1024.0, 16.0, 16.0));
        core.refresh_mist_map();
        let built = core.mist_map_key;
        // A water edit right after a rebuild waits out the interval, then rebuilds.
        let mut depth = core.watermap.clone_baseline_depth_dense();
        depth[0] = 2.0;
        core.watermap
            .replace_baseline_depth_from_dense(&depth)
            .unwrap();
        core.refresh_mist_map();
        assert_eq!(core.mist_map_key, built);
        core.mist_map_built_at = Some(std::time::Instant::now() - MIST_REBUILD_INTERVAL * 2);
        core.refresh_mist_map();
        assert_ne!(core.mist_map_key, built);
        // A replaced world rebuilds at once, even with the same generations.
        let edited = core.mist_map_key;
        core.world_epoch += 1;
        core.refresh_mist_map();
        assert_eq!(core.mist_map_key[0], edited[0] + 1);
    }

    #[test]
    #[ignore = "release timing probe; run explicitly with --release --ignored --nocapture"]
    fn mist_map_build_timing() {
        let config = WorldConfig::new(20480.0, 20480.0, 16.0, 16.0);
        let terrain = TerrainSystem::from_world_config(&config);
        let mut water = WaterSystem::from_world_config(&config);
        let mut depth = water.clone_baseline_depth_dense();
        let width = water.width;
        for (i, cell) in depth.iter_mut().enumerate() {
            // Scattered lakes over about a third of the world.
            if (i % width / 97 + i / width / 89).is_multiple_of(3) {
                *cell = 3.0;
            }
        }
        water.replace_baseline_depth_from_dense(&depth).unwrap();
        let start = std::time::Instant::now();
        let map = build_mist_map(&water, &terrain);
        eprintln!(
            "20 km mist map, {} x {} cells: {:?}",
            map.width,
            map.height,
            start.elapsed()
        );
    }
}
