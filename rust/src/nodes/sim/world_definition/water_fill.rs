// SPDX-License-Identifier: GPL-3.0-or-later

//! The authored water fill: one flat surface flooded 4-connected from a seed sample over the
//! samples at or below it, with the scratch one water solve reuses across all its fills.
//!
//! A fill reads the terrain only at its seed, at its flooded samples and at their 4-neighbours,
//! so it costs `O(k)` for `k` flooded samples: no world-sized allocation, clear or scan per fill
//! (`A4c`). The world-sized buffer (one visit bit a sample) is allocated once and cleared behind
//! each flood, so a kept water book reuses it for every edit (`TERRAIN-38`). The terrain is read
//! through a height function of the row-major sample index: a dense copy at a full solve, the
//! sparse source grid at an edit.

use crate::nodes::sim::core::{AuthoredWaterPatchFillDebug, WorldWaterFillKind};
use crate::simulation::water::WaterSystem;
use std::collections::HashMap;

// Depths at or below this are not counted as a fill's visible water in the debug cache.
pub(super) const AUTHORED_WATER_DEBUG_VISIBLE_EPSILON: f32 = 0.001;

// Radix digit for sorting a fill's flooded sample indices into row-major order.
const RADIX_BITS: u32 = 12;
const RADIX_BUCKETS: usize = 1 << RADIX_BITS;

/// What one fill flood found: its flooded sample count and whether it reaches the world edge.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct LakeFillApplication {
    pub(super) touches_world_edge: bool,
    pub(super) filled_cells: usize,
}

/// Scratch for fill floods over one world's samples.
pub(super) struct FillFlood {
    // One bit a sample: set while the current fill has flooded it. The next flood clears the
    // bits of the samples the last one flooded, so a fill never touches samples it does not
    // visit and the array is never cleared whole.
    visited: Vec<u64>,
    samples: usize,
    // Depth-first work stack of sample indices.
    stack: Vec<usize>,
    // The current fill's flooded samples: in visit order after `flood`, row-major after
    // `sort_flooded_row_major`.
    flooded: Vec<usize>,
    // Radix sort scatter buffer.
    sorted: Vec<usize>,
}

impl FillFlood {
    /// Scratch for floods over `samples` terrain samples: `samples / 8` bytes of visit bits,
    /// zero-initialised in one allocation.
    pub(super) fn new(samples: usize) -> Self {
        Self {
            visited: vec![0; samples.div_ceil(64)],
            samples,
            stack: Vec::new(),
            flooded: Vec::new(),
            sorted: Vec::new(),
        }
    }

    /// The samples the last `flood` filled.
    pub(super) fn flooded(&self) -> &[usize] {
        &self.flooded
    }

    /// The world samples the scratch is sized for.
    pub(super) fn samples(&self) -> usize {
        self.samples
    }

    /// Clears the last flood's visit bits and frees the work lists, which grow to the largest
    /// fill (two words a sample of it): what a kept book holds between solves is the visit
    /// bits alone. `O(k)` for the last flood.
    pub(super) fn release_work_lists(&mut self) {
        for &idx in &self.flooded {
            self.visited[idx / 64] &= !(1 << (idx % 64));
        }
        for list in [&mut self.flooded, &mut self.stack, &mut self.sorted] {
            *list = Vec::new();
        }
    }

    /// The bytes the scratch holds: the visit bits and the work lists' capacity.
    #[cfg(test)]
    pub(super) fn held_bytes(&self) -> usize {
        let words = |list: &Vec<usize>| list.capacity() * std::mem::size_of::<usize>();
        self.visited.capacity() * 8
            + words(&self.flooded)
            + words(&self.stack)
            + words(&self.sorted)
    }

    /// Floods from `(seed_x, seed_z)` over the 4-connected samples at or below
    /// `surface_elevation_m`, reading heights through `terrain` (row-major sample index). A
    /// seed at or above the surface floods nothing. `O(k + k')` for the `k` flooded samples
    /// (each is pushed once and reads its 4 neighbours) and the `k'` the previous flood left
    /// marked; the result does not depend on visit order. It reads the terrain only at the
    /// seed, the flooded samples and their 4-neighbours.
    pub(super) fn flood(
        &mut self,
        mut terrain: impl FnMut(usize) -> f32,
        width: usize,
        height: usize,
        seed_x: usize,
        seed_z: usize,
        surface_elevation_m: f32,
    ) -> LakeFillApplication {
        for &idx in &self.flooded {
            self.visited[idx / 64] &= !(1 << (idx % 64));
        }
        self.flooded.clear();
        if seed_x >= width || seed_z >= height || width * height != self.samples {
            return LakeFillApplication::default();
        }
        let seed = seed_z * width + seed_x;
        if terrain(seed) >= surface_elevation_m {
            return LakeFillApplication::default();
        }

        let mut touches_world_edge = false;
        self.visited[seed / 64] |= 1 << (seed % 64);
        self.stack.clear();
        self.stack.push(seed);
        while let Some(idx) = self.stack.pop() {
            let (x, z) = (idx % width, idx / width);
            if x == 0 || z == 0 || x + 1 == width || z + 1 == height {
                touches_world_edge = true;
            }
            self.flooded.push(idx);
            let neighbours = [
                (x > 0).then(|| idx - 1),
                (x + 1 < width).then(|| idx + 1),
                (z > 0).then(|| idx - width),
                (z + 1 < height).then(|| idx + width),
            ];
            for neighbour in neighbours.into_iter().flatten() {
                let (word, bit) = (neighbour / 64, 1 << (neighbour % 64));
                if self.visited[word] & bit == 0 && terrain(neighbour) <= surface_elevation_m {
                    self.visited[word] |= bit;
                    self.stack.push(neighbour);
                }
            }
        }
        LakeFillApplication {
            touches_world_edge,
            filled_cells: self.flooded.len(),
        }
    }

    /// Grows a kept fill (`TERRAIN-38`): floods from each of `starts` over the 4-connected
    /// samples at or below `surface_elevation_m` that `held` (the fill's samples) does not
    /// hold, leaving them in [`Self::flooded`] in visit order. Starts above the surface or held
    /// are skipped. `O(g + k')` for the `g` samples reached and the `k'` the previous flood left
    /// marked, plus `held`'s cost a neighbour read.
    pub(super) fn grow(
        &mut self,
        starts: &[usize],
        mut terrain: impl FnMut(usize) -> f32,
        mut held: impl FnMut(usize) -> bool,
        width: usize,
        height: usize,
        surface_elevation_m: f32,
    ) {
        for &idx in &self.flooded {
            self.visited[idx / 64] &= !(1 << (idx % 64));
        }
        self.flooded.clear();
        self.stack.clear();
        for &start in starts {
            let (word, bit) = (start / 64, 1 << (start % 64));
            if self.visited[word] & bit == 0
                && !held(start)
                && terrain(start) <= surface_elevation_m
            {
                self.visited[word] |= bit;
                self.stack.push(start);
            }
        }
        while let Some(idx) = self.stack.pop() {
            let (x, z) = (idx % width, idx / width);
            self.flooded.push(idx);
            let neighbours = [
                (x > 0).then(|| idx - 1),
                (x + 1 < width).then(|| idx + 1),
                (z > 0).then(|| idx - width),
                (z + 1 < height).then(|| idx + width),
            ];
            for neighbour in neighbours.into_iter().flatten() {
                let (word, bit) = (neighbour / 64, 1 << (neighbour % 64));
                if self.visited[word] & bit == 0
                    && terrain(neighbour) <= surface_elevation_m
                    && !held(neighbour)
                {
                    self.visited[word] |= bit;
                    self.stack.push(neighbour);
                }
            }
        }
    }

    /// Takes the last flood's depth at each flooded sample into `depth` (the larger depth wins,
    /// so the merge does not depend on fill order). `O(k)`.
    pub(super) fn merge_depth(
        &self,
        terrain_world: &[f32],
        surface_elevation_m: f32,
        depth: &mut [f32],
    ) {
        for &idx in &self.flooded {
            depth[idx] = depth[idx].max(fill_depth(terrain_world[idx], surface_elevation_m));
        }
    }

    // LSD radix sort of the flooded indices, `RADIX_BITS` per pass: `O(k + buckets)` per pass,
    // and as many passes as the world's largest index needs (two below 16.7 M samples). A small
    // fill sorts by comparison instead (`O(k log k)` with `k` below the bucket count).
    pub(super) fn sort_flooded_row_major(&mut self) {
        if self.flooded.len() < RADIX_BUCKETS {
            self.flooded.sort_unstable();
            return;
        }
        let max_index = self.samples.saturating_sub(1);
        self.sorted.resize(self.flooded.len(), 0);
        let mut shift = 0;
        while shift == 0 || (max_index >> shift) != 0 {
            let mut counts = [0usize; RADIX_BUCKETS];
            for &idx in &self.flooded {
                counts[(idx >> shift) & (RADIX_BUCKETS - 1)] += 1;
            }
            let mut start = 0;
            for count in &mut counts {
                (start, *count) = (start + *count, start);
            }
            for &idx in &self.flooded {
                let bucket = &mut counts[(idx >> shift) & (RADIX_BUCKETS - 1)];
                self.sorted[*bucket] = idx;
                *bucket += 1;
            }
            std::mem::swap(&mut self.flooded, &mut self.sorted);
            shift += RADIX_BITS;
        }
    }
}

// The depth a fill gives one flooded sample, as the dense solve wrote it into a zeroed buffer.
pub(super) fn fill_depth(terrain_m: f32, surface_elevation_m: f32) -> f32 {
    0.0f32.max(surface_elevation_m - terrain_m)
}

/// One fill's identity in the debug cache.
#[derive(Clone, Copy)]
pub(super) struct FillDebugSource {
    pub(super) kind: WorldWaterFillKind,
    pub(super) fill_index: i32,
    pub(super) preview: bool,
    pub(super) world_x: f32,
    pub(super) world_z: f32,
    pub(super) surface_elevation_m: f32,
}

// One fill's running statistics in one render patch.
#[derive(Clone, Copy, Default)]
struct PatchFillStats {
    nonzero_samples: usize,
    max_depth_m: f32,
    sum_depth_m: f32,
}

/// Per-solve scratch for the debug cache: one slot per render patch, reset only where a fill
/// wrote.
pub(super) struct PatchFillDebug {
    stats: Vec<PatchFillStats>,
    touched: Vec<usize>,
}

impl PatchFillDebug {
    /// Scratch for `water`'s render patches.
    pub(super) fn new(water: &WaterSystem) -> Self {
        Self {
            stats: vec![
                PatchFillStats::default();
                water.render_patch_cols() * water.render_patch_rows()
            ],
            touched: Vec::new(),
        }
    }

    /// The bytes the scratch holds.
    #[cfg(test)]
    pub(super) fn held_bytes(&self) -> usize {
        self.stats.capacity() * std::mem::size_of::<PatchFillStats>()
            + self.touched.capacity() * std::mem::size_of::<usize>()
    }

    /// Adds the last flood's contribution to every render patch holding one of its visible
    /// samples, and appends those patches to `patches`. The flood must be sorted row-major
    /// ([`FillFlood::sort_flooded_row_major`]), so each patch sums its depths in the order a
    /// row-by-row scan of the patch would. `O(k)` and the patches the fill touches.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_fill(
        &mut self,
        out: &mut HashMap<(usize, usize), Vec<AuthoredWaterPatchFillDebug>>,
        flood: &FillFlood,
        mut terrain: impl FnMut(usize) -> f32,
        water: &WaterSystem,
        source: FillDebugSource,
        application: LakeFillApplication,
        patches: &mut Vec<(usize, usize)>,
    ) {
        let (cols, rows) = (water.render_patch_cols(), water.render_patch_rows());
        let interval = water.render_patch_interval_cells();
        for &idx in flood.flooded() {
            let depth_m = fill_depth(terrain(idx), source.surface_elevation_m);
            if depth_m <= AUTHORED_WATER_DEBUG_VISIBLE_EPSILON {
                continue;
            }
            let (x, z) = (idx % water.width, idx / water.width);
            // A sample on a patch boundary belongs to the patches on both sides (the owned
            // bounds include their last row and column), so up to four patches.
            for patch_z in owning_patches(z, interval, rows) {
                for patch_x in owning_patches(x, interval, cols) {
                    debug_assert!(
                        water
                            .render_patch_owned_sample_bounds(patch_x, patch_z)
                            .is_some_and(|(start_x, end_x, start_z, end_z)| {
                                (start_x..=end_x).contains(&x) && (start_z..=end_z).contains(&z)
                            }),
                        "sample ({x}, {z}) outside patch ({patch_x}, {patch_z})"
                    );
                    let slot = patch_z * cols + patch_x;
                    let stats = &mut self.stats[slot];
                    if stats.nonzero_samples == 0 {
                        self.touched.push(slot);
                    }
                    stats.nonzero_samples += 1;
                    stats.max_depth_m = stats.max_depth_m.max(depth_m);
                    stats.sum_depth_m += depth_m;
                }
            }
        }
        for slot in self.touched.drain(..) {
            let stats = std::mem::take(&mut self.stats[slot]);
            patches.push((slot % cols, slot / cols));
            out.entry((slot % cols, slot / cols))
                .or_default()
                .push(AuthoredWaterPatchFillDebug {
                    kind: source.kind,
                    fill_index: source.fill_index,
                    preview: source.preview,
                    world_x: source.world_x,
                    world_z: source.world_z,
                    surface_elevation_m: source.surface_elevation_m,
                    filled_cells: application.filled_cells,
                    touches_world_edge: application.touches_world_edge,
                    patch_nonzero_samples: stats.nonzero_samples,
                    patch_max_depth_m: stats.max_depth_m,
                    patch_sum_depth_m: stats.sum_depth_m,
                });
        }
    }
}

// The patches along one axis whose owned span (`WaterSystem::render_patch_owned_sample_bounds`:
// `p * interval ..= min((p + 1) * interval, last sample)`) holds sample `coord`: patch
// `coord / interval` when it exists (the last sample of an axis that ends on a boundary has
// none), and the previous patch when `coord` lies on its far boundary.
pub(super) fn owning_patches(
    coord: usize,
    interval: usize,
    count: usize,
) -> impl Iterator<Item = usize> {
    let own = coord / interval;
    let previous = (coord > 0 && coord.is_multiple_of(interval)).then(|| own - 1);
    previous.into_iter().chain((own < count).then_some(own))
}
