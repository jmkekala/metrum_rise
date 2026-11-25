// SPDX-License-Identifier: GPL-3.0-or-later

//! The authored water kept between solves, and its update in place after a terrain change
//! (`TERRAIN-38`).
//!
//! A full solve (`rebuild_authored_water_preview_internal`) keeps a book: every fill's flooded
//! samples, its flood report and whether its kind accepted it, the render patches its debug
//! entries sit in, and the source terrain it read. A brush edit, a stroke or an undo then goes
//! through [`SimCore::refresh_authored_water_after_terrain_change`], which compares the live
//! terrain with the book's and re-solves only the fills whose result can change.
//!
//! The rule is exact. A fill reads the terrain only at its seed, its flooded samples and their
//! 4-neighbours (`FillFlood::flood`), so a fill none of whose read samples changed floods the
//! same samples, reaches the same edge, is accepted the same way and gives every sample it holds
//! the same depth and debug statistics. A sample's depth is the deepest accepted fill holding it
//! (`max` of non-negative values, so the order of fills does not matter). A sample no re-solved
//! fill held before or holds now is held by the same unchanged fills as before, at unchanged
//! ground (a changed sample inside an unchanged fill would have made it re-solve), so its depth
//! cannot change. The update therefore recomputes the depth only over the samples the re-solved
//! fills gained, lost or hold at changed ground, from every accepted fill holding them, and
//! writes the samples whose bits differ. A fill that read a change is updated locally when that
//! is exact (`KeptFill::update_locally`) and flooded again from its seed otherwise. The result equals a full solve bit for bit (depth, stored chunks and the
//! debug cache), which the tests check after every edit of a long sequence.

use super::water_fill::{
    AUTHORED_WATER_DEBUG_VISIBLE_EPSILON, FillDebugSource, FillFlood, LakeFillApplication,
    PatchFillDebug, fill_depth, owning_patches,
};
use crate::debug_log;
use crate::nodes::sim::core::{AuthoredWaterPatchFillDebug, SimCore, WorldWaterFillKind};
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use std::collections::HashMap;

// Side of a flood set's bit tile, in samples: one `u64` a tile row.
const TILE: usize = 64;

type DebugCache = HashMap<(usize, usize), Vec<AuthoredWaterPatchFillDebug>>;

// A fill's flooded samples as 64 x 64 bit tiles, sorted by tile key (tile row, then column):
// bit `x % 64` of word `z % 64` of tile `(x / 64, z / 64)`. Membership is a binary search over
// the fill's tiles and one bit test.
#[derive(Default)]
struct FloodSet {
    tiles: Vec<(u64, [u64; TILE])>,
    // Inclusive `[min_x, min_z, max_x, max_z]` of the samples; `None` when empty.
    bounds: Option<[usize; 4]>,
}

fn tile_key(tile_x: usize, tile_z: usize) -> u64 {
    ((tile_z as u64) << 32) | tile_x as u64
}

impl FloodSet {
    // From samples in row-major order. `O(k + tiles log tiles)`.
    fn from_row_major(samples: &[usize], width: usize) -> Self {
        let mut set = Self::default();
        let (Some(&first), Some(&last)) = (samples.first(), samples.last()) else {
            return set;
        };
        // The current tile row's tile slots by tile column; reset when the tile row changes.
        let mut slots = vec![usize::MAX; width.div_ceil(TILE)];
        let mut used = Vec::new();
        let mut tile_z = usize::MAX;
        let (mut min_x, mut max_x) = (usize::MAX, 0);
        for &idx in samples {
            let (x, z) = (idx % width, idx / width);
            if z / TILE != tile_z {
                tile_z = z / TILE;
                for column in used.drain(..) {
                    slots[column] = usize::MAX;
                }
            }
            let column = x / TILE;
            if slots[column] == usize::MAX {
                slots[column] = set.tiles.len();
                used.push(column);
                set.tiles.push((tile_key(column, tile_z), [0; TILE]));
            }
            set.tiles[slots[column]].1[z % TILE] |= 1 << (x % TILE);
            min_x = min_x.min(x);
            max_x = max_x.max(x);
        }
        // Tiles of one tile row were created in the order their columns were first met.
        set.tiles.sort_unstable_by_key(|&(key, _)| key);
        set.bounds = Some([min_x, first / width, max_x, last / width]);
        set
    }

    fn tile(&self, key: u64) -> Option<&[u64; TILE]> {
        self.tiles
            .binary_search_by_key(&key, |&(tile, _)| tile)
            .ok()
            .map(|slot| &self.tiles[slot].1)
    }

    fn contains(&self, x: usize, z: usize) -> bool {
        self.tile(tile_key(x / TILE, z / TILE))
            .is_some_and(|rows| rows[z % TILE] >> (x % TILE) & 1 == 1)
    }

    // Adds one sample. `O(log tiles)`, or `O(tiles)` to open a tile.
    fn insert(&mut self, x: usize, z: usize) {
        let key = tile_key(x / TILE, z / TILE);
        let slot = match self.tiles.binary_search_by_key(&key, |&(tile, _)| tile) {
            Ok(slot) => slot,
            Err(slot) => {
                self.tiles.insert(slot, (key, [0; TILE]));
                slot
            }
        };
        self.tiles[slot].1[z % TILE] |= 1 << (x % TILE);
        self.bounds = Some(match self.bounds {
            Some([x0, z0, x1, z1]) => [x0.min(x), z0.min(z), x1.max(x), z1.max(z)],
            None => [x, z, x, z],
        });
    }

    // Removes one sample, and its tile when that empties. The bounds stay as they were: a
    // superset, which every reader only uses to rule fills out. `O(log tiles)`, or `O(tiles)` to
    // close a tile.
    fn remove(&mut self, x: usize, z: usize) {
        let key = tile_key(x / TILE, z / TILE);
        if let Ok(slot) = self.tiles.binary_search_by_key(&key, |&(tile, _)| tile) {
            self.tiles[slot].1[z % TILE] &= !(1 << (x % TILE));
            if self.tiles[slot].1.iter().all(|&row| row == 0) {
                self.tiles.remove(slot);
            }
        }
    }
}

// Bit tiles holding the given samples (any order, repeats allowed), sorted by tile key.
fn tiles_of(samples: &[usize], width: usize) -> Vec<(u64, [u64; TILE])> {
    let mut set = FloodSet::default();
    for &idx in samples {
        set.insert(idx % width, idx / width);
    }
    set.tiles
}

// Whether a sample lies on the world's edge.
fn on_world_edge(idx: usize, width: usize, height: usize) -> bool {
    let (x, z) = (idx % width, idx / width);
    x == 0 || z == 0 || x + 1 == width || z + 1 == height
}

// Side margin, in samples, of the window around a raise's removed samples within which the
// local split check looks for a path joining their neighbours.
const SPLIT_CHECK_MARGIN: usize = 32;

// One fill as the last solve of it left it.
pub(super) struct KeptFill {
    source: FillDebugSource,
    seed: usize,
    set: FloodSet,
    application: LakeFillApplication,
    // Flooded samples on the world edge: the edge flag is `edge_samples > 0`.
    edge_samples: usize,
    accepted: bool,
    // The render patches holding this fill's debug entries.
    patches: Vec<(usize, usize)>,
}

impl KeptFill {
    // Records the flood `flood` holds (sorted row-major) for `source`.
    pub(super) fn new(
        source: FillDebugSource,
        seed: usize,
        flood: &FillFlood,
        width: usize,
        application: LakeFillApplication,
        patches: Vec<(usize, usize)>,
    ) -> Self {
        let height = flood.samples() / width;
        Self {
            source,
            seed,
            set: FloodSet::from_row_major(flood.flooded(), width),
            application,
            edge_samples: flood
                .flooded()
                .iter()
                .filter(|&&idx| on_world_edge(idx, width, height))
                .count(),
            accepted: fill_kind_accepts(source.kind, application),
            patches,
        }
    }

    // Brings the fill to the live terrain without flooding it again, when that is exact and
    // local: the changed samples it read leave its seed alone, and the samples a raise lifts
    // above its level (`removed`) leave the rest of the fill connected, which a search within
    // a window around them shows (a fill that was connected stays so when the neighbours of
    // the removed samples still join up). The fill then is its old samples less `removed`,
    // plus what floods from the changed samples newly at or below its level that touch it
    // (`FillFlood::grow`). Returns the samples whose depth or debug statistics may change
    // (removed, grown, and changed samples it still holds), or `None`, leaving the fill as it
    // was, when the seed changed, the split check cannot show the rest connected, or the
    // fill's kind would accept it differently: the caller floods it again. `O(changed x log
    // tiles + window + grown samples)`, the window `(2 x 32 + removed span)^2` samples only
    // when something was removed.
    #[allow(clippy::too_many_arguments)]
    fn update_locally(
        &mut self,
        changed: &[usize],
        terrain: &mut GridReader,
        flood: &mut FillFlood,
        width: usize,
        height: usize,
    ) -> Option<Vec<usize>> {
        let level = self.source.surface_elevation_m;
        if changed.binary_search(&self.seed).is_ok() || self.set.bounds.is_none() {
            return None;
        }
        let held = |set: &FloodSet, idx: usize| set.contains(idx % width, idx / width);
        let mut removed = Vec::new();
        let mut kept_changed = Vec::new();
        let mut starts = Vec::new();
        for &c in changed {
            if held(&self.set, c) {
                if terrain.at(c) > level {
                    removed.push(c);
                } else {
                    kept_changed.push(c);
                }
            } else {
                starts.push(c);
            }
        }
        if !removed.is_empty() && !self.stays_connected_without(&removed, width, height) {
            return None;
        }
        // Growth starts only at changed samples beside a sample the fill keeps: an unchanged
        // sample beside the fill and at or below its level would already be in it.
        let set = &self.set;
        let keeps = |idx: usize| held(set, idx) && removed.binary_search(&idx).is_err();
        starts.retain(|&c| {
            let (x, z) = (c % width, c / width);
            (x > 0 && keeps(c - 1))
                || (x + 1 < width && keeps(c + 1))
                || (z > 0 && keeps(c - width))
                || (z + 1 < height && keeps(c + width))
        });
        flood.grow(
            &starts,
            |idx| terrain.at(idx),
            |idx| held(set, idx),
            width,
            height,
            level,
        );
        let grown = flood.flooded();
        let edge_samples = self.edge_samples
            - removed
                .iter()
                .filter(|&&idx| on_world_edge(idx, width, height))
                .count()
            + grown
                .iter()
                .filter(|&&idx| on_world_edge(idx, width, height))
                .count();
        let application = LakeFillApplication {
            touches_world_edge: edge_samples > 0,
            filled_cells: self.application.filled_cells - removed.len() + grown.len(),
        };
        if fill_kind_accepts(self.source.kind, application) != self.accepted {
            return None;
        }
        for &idx in &removed {
            self.set.remove(idx % width, idx / width);
        }
        for &idx in grown {
            self.set.insert(idx % width, idx / width);
        }
        self.application = application;
        self.edge_samples = edge_samples;
        let mut touched = removed;
        touched.extend_from_slice(grown);
        touched.extend_from_slice(&kept_changed);
        Some(touched)
    }

    // Whether the fill, less the sorted `removed` samples (which leave its seed), stays
    // 4-connected: the kept neighbours of the removed samples must join up through kept samples
    // within a window `SPLIT_CHECK_MARGIN` beyond them. Every part of a split fill holds such a
    // neighbour, so when they all join the fill is whole. `false` may be a split or a path that
    // leaves the window. `O(window samples x log tiles)`.
    fn stays_connected_without(&self, removed: &[usize], width: usize, height: usize) -> bool {
        let [rx0, rz0, rx1, rz1] =
            removed
                .iter()
                .fold([usize::MAX, usize::MAX, 0, 0], |[x0, z0, x1, z1], &idx| {
                    let (x, z) = (idx % width, idx / width);
                    [x0.min(x), z0.min(z), x1.max(x), z1.max(z)]
                });
        let (wx0, wz0) = (
            rx0.saturating_sub(SPLIT_CHECK_MARGIN),
            rz0.saturating_sub(SPLIT_CHECK_MARGIN),
        );
        let (wx1, wz1) = (
            (rx1 + SPLIT_CHECK_MARGIN).min(width - 1),
            (rz1 + SPLIT_CHECK_MARGIN).min(height - 1),
        );
        let window_width = wx1 - wx0 + 1;
        let local = |idx: usize| (idx / width - wz0) * window_width + idx % width - wx0;
        let kept = |idx: usize| {
            let (x, z) = (idx % width, idx / width);
            (wx0..=wx1).contains(&x)
                && (wz0..=wz1).contains(&z)
                && self.set.contains(x, z)
                && removed.binary_search(&idx).is_err()
        };
        let neighbours = |idx: usize| {
            let (x, z) = (idx % width, idx / width);
            [
                (x > 0).then(|| idx - 1),
                (x + 1 < width).then(|| idx + 1),
                (z > 0).then(|| idx - width),
                (z + 1 < height).then(|| idx + width),
            ]
        };
        let mut targets: Vec<usize> = removed
            .iter()
            .flat_map(|&idx| neighbours(idx).into_iter().flatten())
            .filter(|&idx| kept(idx))
            .collect();
        targets.sort_unstable();
        targets.dedup();
        let Some(&first) = targets.first() else {
            // Nothing kept touches the removed samples: the fill was only them and its seed's
            // part, which the caller's seed check covers. Flood it again to be sure.
            return false;
        };
        let mut seen = vec![false; window_width * (wz1 - wz0 + 1)];
        let mut stack = vec![first];
        seen[local(first)] = true;
        let mut reached = 0;
        while let Some(idx) = stack.pop() {
            if targets.binary_search(&idx).is_ok() {
                reached += 1;
                if reached == targets.len() {
                    return true;
                }
            }
            for next in neighbours(idx).into_iter().flatten() {
                if kept(next) && !seen[local(next)] {
                    seen[local(next)] = true;
                    stack.push(next);
                }
            }
        }
        false
    }

    // This fill's debug statistics in one render patch, from a row-major scan of the patch's
    // owned samples it holds, as `PatchFillDebug::push_fill` sums them: `None` when it puts no
    // visible water there. `O(patch samples x log tiles)`.
    fn patch_entry(
        &self,
        (patch_x, patch_z): (usize, usize),
        terrain: &mut GridReader,
        water: &WaterSystem,
    ) -> Option<AuthoredWaterPatchFillDebug> {
        let (x0, x1, z0, z1) = water.render_patch_owned_sample_bounds(patch_x, patch_z)?;
        let (mut count, mut max, mut sum) = (0usize, 0.0f32, 0.0f32);
        for z in z0..=z1 {
            for x in x0..=x1 {
                if !self.set.contains(x, z) {
                    continue;
                }
                let depth = fill_depth(
                    terrain.at(z * water.width + x),
                    self.source.surface_elevation_m,
                );
                if depth > AUTHORED_WATER_DEBUG_VISIBLE_EPSILON {
                    count += 1;
                    max = max.max(depth);
                    sum += depth;
                }
            }
        }
        (count > 0).then_some(AuthoredWaterPatchFillDebug {
            kind: self.source.kind,
            fill_index: self.source.fill_index,
            preview: self.source.preview,
            world_x: self.source.world_x,
            world_z: self.source.world_z,
            surface_elevation_m: self.source.surface_elevation_m,
            filled_cells: self.application.filled_cells,
            touches_world_edge: self.application.touches_world_edge,
            patch_nonzero_samples: count,
            patch_max_depth_m: max,
            patch_sum_depth_m: sum,
        })
    }

    // After a local update of an accepted fill: rewrites its entries in the patches owning a
    // `touched` sample from a scan of each, and its flood report in every other entry it has.
    // Pushes the patches whose entries changed to `touched_patches`. `O(touched + patches
    // touched x patch samples x log tiles + the fill's patches)`.
    fn refresh_debug_locally(
        &mut self,
        touched: &[usize],
        terrain: &mut GridReader,
        water: &WaterSystem,
        debug: &mut DebugCache,
        touched_patches: &mut Vec<(usize, usize)>,
    ) {
        let (cols, rows) = (water.render_patch_cols(), water.render_patch_rows());
        let interval = water.render_patch_interval_cells();
        let mut patches = Vec::new();
        for &idx in touched {
            let (x, z) = (idx % water.width, idx / water.width);
            for patch_z in owning_patches(z, interval, rows) {
                for patch_x in owning_patches(x, interval, cols) {
                    patches.push((patch_x, patch_z));
                }
            }
        }
        patches.sort_unstable();
        patches.dedup();
        for &key in &patches {
            let entry = self.patch_entry(key, terrain, water);
            let contributors = debug.entry(key).or_default();
            let slot = contributors
                .iter()
                .position(|existing| is_entry_of(existing, &self.source));
            match (slot, entry) {
                (Some(slot), Some(entry)) => contributors[slot] = entry,
                (Some(slot), None) => {
                    contributors.remove(slot);
                    self.patches.retain(|&patch| patch != key);
                }
                (None, Some(entry)) => {
                    contributors.push(entry);
                    self.patches.push(key);
                }
                (None, None) => {}
            }
            touched_patches.push(key);
        }
        for key in &self.patches {
            if let Some(contributors) = debug.get_mut(key) {
                for entry in contributors
                    .iter_mut()
                    .filter(|entry| is_entry_of(entry, &self.source))
                {
                    entry.filled_cells = self.application.filled_cells;
                    entry.touches_world_edge = self.application.touches_world_edge;
                }
            }
        }
    }

    // Whether a changed sample is one this fill read: its seed, a flooded sample or a
    // 4-neighbour of one. `O(1)` when the changes lie outside its reach, else
    // `O(changed x log tiles)`.
    fn reads_any(&self, changed: &[usize], [cx0, cz0, cx1, cz1]: [usize; 4], width: usize) -> bool {
        let seed_hit = |&c: &usize| c == self.seed;
        let Some([x0, z0, x1, z1]) = self.set.bounds else {
            return changed.iter().any(seed_hit);
        };
        if cx1 + 1 < x0 || cx0 > x1 + 1 || cz1 + 1 < z0 || cz0 > z1 + 1 {
            return changed.iter().any(seed_hit);
        }
        changed.iter().any(|&c| {
            let (x, z) = (c % width, c / width);
            c == self.seed
                || self.set.contains(x, z)
                || (x > 0 && self.set.contains(x - 1, z))
                || self.set.contains(x + 1, z)
                || (z > 0 && self.set.contains(x, z - 1))
                || self.set.contains(x, z + 1)
        })
    }
}

/// Whether a fill's kind keeps what its flood reached: a lake must stay inside the world, open
/// water must reach its edge.
pub(super) fn fill_kind_accepts(
    kind: WorldWaterFillKind,
    application: LakeFillApplication,
) -> bool {
    match kind {
        WorldWaterFillKind::Lake => !application.touches_world_edge,
        WorldWaterFillKind::OpenWater => application.touches_world_edge,
    }
}

// Reads a sparse grid by row-major sample index, keeping the chunk it last looked up: a flood
// or a tile walk stays in one chunk for long runs, so most reads are two divisions and an index.
struct GridReader<'a> {
    grid: &'a SparseChunkGrid<f32>,
    width: usize,
    chunk_size: usize,
    chunk: Option<(usize, usize)>,
    slice: Option<&'a [f32]>,
}

impl<'a> GridReader<'a> {
    fn new(grid: &'a SparseChunkGrid<f32>, width: usize) -> Self {
        Self {
            grid,
            width,
            chunk_size: grid.chunk_size(),
            chunk: None,
            slice: None,
        }
    }

    fn at(&mut self, idx: usize) -> f32 {
        let (x, z) = (idx % self.width, idx / self.width);
        let chunk = (x / self.chunk_size, z / self.chunk_size);
        if self.chunk != Some(chunk) {
            self.chunk = Some(chunk);
            self.slice = self.grid.chunk_slice(chunk.0, chunk.1);
        }
        match self.slice {
            Some(cells) => cells[(z % self.chunk_size) * self.chunk_size + x % self.chunk_size],
            None => self.grid.default_value(),
        }
    }
}

/// The authored water a full solve left: what an in-place update needs to re-solve only the
/// fills a terrain change reaches. Kept only while no fill preview is active; rebuilt by every
/// full solve and dropped whenever the water or the fills change another way (a load, an undo
/// record's water). Derived state, never saved.
pub(crate) struct AuthoredWaterBook {
    // The source terrain the water was solved on: shared chunk pointers, so it costs
    // `O(chunks)` pointers and keeps no chunk the live terrain does not also hold after an
    // update.
    terrain: SparseChunkGrid<f32>,
    flood: FillFlood,
    patch_debug: PatchFillDebug,
    // Lakes in their order, then open water in theirs: the solve's order.
    fills: Vec<KeptFill>,
    // Fills brought up to date in place and fills flooded again, over the book's life.
    local_updates: u64,
    refloods: u64,
}

impl AuthoredWaterBook {
    pub(super) fn new(
        terrain: SparseChunkGrid<f32>,
        mut flood: FillFlood,
        patch_debug: PatchFillDebug,
        fills: Vec<KeptFill>,
    ) -> Self {
        flood.release_work_lists();
        Self {
            terrain,
            flood,
            patch_debug,
            fills,
            local_updates: 0,
            refloods: 0,
        }
    }

    // Brings the water and the debug cache to the live terrain. Returns whether any depth
    // changed. Cost: `O(storage chunks)` pointer comparisons and a scan of each chunk the
    // terrain change wrote; per fill `O(1)` to rule it out by bounds, or `O(changed x log
    // tiles)` to test it; per locally updated fill the cost of `KeptFill::update_locally` and
    // of rescanning the patches it touched; per fill flooded again `O(k + k_old)` for its new
    // and old flooded samples (flood, row-major sort, tile set, debug entries); and the depth
    // pass over the touched tiles, `O(tiles x fills)` tile lookups plus the samples they hold.
    // Nothing is allocated, cleared or scanned in proportion to the world except the chunk
    // pointer comparison, the book's copy of the terrain's chunk pointers and, when a reader
    // shares the water grid, its one copy of chunk pointers.
    fn update(
        &mut self,
        terrain: &TerrainSystem,
        water: &mut WaterSystem,
        debug: &mut DebugCache,
    ) -> bool {
        let live = terrain.source_grid();
        let mut changed = Vec::new();
        live.changed_cells_since(&self.terrain, &mut changed);
        self.terrain = live.clone();
        if changed.is_empty() {
            return false;
        }
        let (width, height) = (water.width, water.height);
        let changed_bounds =
            changed
                .iter()
                .fold([usize::MAX, usize::MAX, 0, 0], |[x0, z0, x1, z1], &c| {
                    let (x, z) = (c % width, c / width);
                    [x0.min(x), z0.min(z), x1.max(x), z1.max(z)]
                });
        let mut reader = GridReader::new(live, width);
        let mut region: Vec<(u64, [u64; TILE])> = Vec::new();
        let mut touched_patches = Vec::new();
        for fill in &mut self.fills {
            if !fill.reads_any(&changed, changed_bounds, width) {
                continue;
            }
            if let Some(touched) =
                fill.update_locally(&changed, &mut reader, &mut self.flood, width, height)
            {
                self.local_updates += 1;
                if fill.accepted {
                    region.extend(tiles_of(&touched, width));
                    fill.refresh_debug_locally(
                        &touched,
                        &mut reader,
                        water,
                        debug,
                        &mut touched_patches,
                    );
                }
                continue;
            }
            self.refloods += 1;
            if fill.accepted {
                region.extend_from_slice(&fill.set.tiles);
            }
            for key in fill.patches.drain(..) {
                if let Some(contributors) = debug.get_mut(&key) {
                    contributors.retain(|entry| !is_entry_of(entry, &fill.source));
                }
                touched_patches.push(key);
            }
            let (seed_x, seed_z) = (fill.seed % width, fill.seed / width);
            let application = self.flood.flood(
                |idx| reader.at(idx),
                width,
                height,
                seed_x,
                seed_z,
                fill.source.surface_elevation_m,
            );
            self.flood.sort_flooded_row_major();
            fill.set = FloodSet::from_row_major(self.flood.flooded(), width);
            fill.edge_samples = self
                .flood
                .flooded()
                .iter()
                .filter(|&&idx| on_world_edge(idx, width, height))
                .count();
            fill.application = application;
            fill.accepted = fill_kind_accepts(fill.source.kind, application);
            if fill.accepted {
                region.extend_from_slice(&fill.set.tiles);
                self.patch_debug.push_fill(
                    debug,
                    &self.flood,
                    |idx| reader.at(idx),
                    water,
                    fill.source,
                    application,
                    &mut fill.patches,
                );
                touched_patches.extend_from_slice(&fill.patches);
            }
        }
        self.flood.release_work_lists();
        let writes = self.region_depth_writes(region, &mut reader, water);
        water.write_baseline_depth_samples(&writes);
        touched_patches.sort_unstable();
        touched_patches.dedup();
        for key in touched_patches {
            match debug.get_mut(&key) {
                Some(contributors) if contributors.is_empty() => {
                    debug.remove(&key);
                }
                Some(contributors) => super::sort_authored_water_patch_fill_debug(contributors),
                None => {}
            }
        }
        debug_log!(
            "world-editor",
            "authored_water_update changed_samples={} depth_writes={} local_updates={} refloods={}",
            changed.len(),
            writes.len(),
            self.local_updates,
            self.refloods
        );
        !writes.is_empty()
    }

    // The new depth of every sample in `region` (tiles of the re-solved fills' old and new
    // flooded samples, repeated tiles merged): `0.0`, raised to each accepted fill's depth where
    // it holds the sample, in fill order as the full solve merges. Returns the samples whose
    // bits differ from the stored depth. `O(tiles x fills)` tile lookups (a bounds test, then a
    // binary search) and `O(samples held)`.
    fn region_depth_writes(
        &self,
        mut region: Vec<(u64, [u64; TILE])>,
        terrain: &mut GridReader,
        water: &WaterSystem,
    ) -> Vec<(usize, f32)> {
        region.sort_unstable_by_key(|&(key, _)| key);
        region.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                for (row, bits) in kept.1.iter_mut().zip(later.1) {
                    *row |= bits;
                }
            }
            same
        });
        let width = water.width;
        let mut stored = GridReader::new(water.baseline_depth_grid(), width);
        let mut depth = [0.0f32; TILE * TILE];
        let mut writes = Vec::new();
        for (key, mask) in region {
            let (tile_x, tile_z) = ((key & 0xFFFF_FFFF) as usize, (key >> 32) as usize);
            let (tx0, tz0) = (tile_x * TILE, tile_z * TILE);
            for fill in &self.fills {
                if !fill.accepted
                    || fill.set.bounds.is_none_or(|[x0, z0, x1, z1]| {
                        x1 < tx0 || x0 >= tx0 + TILE || z1 < tz0 || z0 >= tz0 + TILE
                    })
                {
                    continue;
                }
                let Some(rows) = fill.set.tile(key) else {
                    continue;
                };
                let level = fill.source.surface_elevation_m;
                for (row, (&held, &wanted)) in rows.iter().zip(&mask).enumerate() {
                    let mut bits = held & wanted;
                    while bits != 0 {
                        let column = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        let idx = (tz0 + row) * width + tx0 + column;
                        let cell = &mut depth[row * TILE + column];
                        *cell = cell.max(fill_depth(terrain.at(idx), level));
                    }
                }
            }
            for (row, &wanted) in mask.iter().enumerate() {
                let mut bits = wanted;
                while bits != 0 {
                    let column = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let idx = (tz0 + row) * width + tx0 + column;
                    let cell = &mut depth[row * TILE + column];
                    if cell.to_bits() != stored.at(idx).to_bits() {
                        writes.push((idx, *cell));
                    }
                    *cell = 0.0;
                }
            }
        }
        writes
    }
}

impl AuthoredWaterBook {
    /// The bytes the book holds between solves: the fills' bit tiles, the visit bits, the
    /// terrain's chunk pointers (not the chunks, which the live terrain shares) and the debug
    /// scratch. For the measurements of `TERRAIN-38`.
    #[cfg(test)]
    pub(crate) fn held_bytes(&self) -> usize {
        let tile = std::mem::size_of::<(u64, [u64; TILE])>();
        let fills: usize = self
            .fills
            .iter()
            .map(|fill| {
                std::mem::size_of::<KeptFill>()
                    + fill.set.tiles.capacity() * tile
                    + fill.patches.capacity() * std::mem::size_of::<(usize, usize)>()
            })
            .sum();
        fills
            + self.flood.held_bytes()
            + self.terrain.materialized_chunk_count() * 2 * std::mem::size_of::<usize>()
            + self.patch_debug.held_bytes()
    }
}

// Whether a debug entry is the given fill's.
fn is_entry_of(entry: &AuthoredWaterPatchFillDebug, source: &FillDebugSource) -> bool {
    entry.kind == source.kind
        && entry.fill_index == source.fill_index
        && entry.preview == source.preview
}

impl SimCore {
    /// Brings the authored water up to date with the terrain after anything wrote it: a brush
    /// edit or stroke, and the undo that restores the terrain from its record (`TERRAIN-37`).
    /// The one water update every terrain change goes through. With a kept book (a full solve
    /// left one, no fill preview is active) the water is updated in place, bounded by the
    /// changed samples and the fills that read them ([`AuthoredWaterBook`]); otherwise this is
    /// a full solve, which keeps a book for the next change.
    pub(crate) fn refresh_authored_water_after_terrain_change(&mut self) {
        if !self.has_authored_water_internal() {
            return;
        }
        if self.world_lake_fill_preview.is_none()
            && let Some(mut book) = self.authored_water_book.take()
        {
            debug_assert_eq!(
                book.fills.len(),
                self.world_lake_fills.len() + self.world_open_water_fills.len(),
                "the kept book must hold the fills it was solved with"
            );
            let changed = book.update(
                &self.heightmap,
                &mut self.watermap,
                &mut self.authored_water_patch_fill_debug_cache,
            );
            self.authored_water_book = Some(book);
            if changed {
                self.water_dirty = true;
                self.bump_road_tool_query_generation();
            }
            return;
        }
        if let Err(err) = self.rebuild_authored_water_preview_internal() {
            debug_log!(
                "world-editor",
                "rebuild_authored_water_after_terrain_change failed: {}",
                err
            );
        }
    }
}

#[cfg(test)]
mod tests;
