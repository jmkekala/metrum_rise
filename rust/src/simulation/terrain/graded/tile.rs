// SPDX-License-Identifier: GPL-2.0-only

//! Tiles, level lattices and the tile bake (`TERRAIN-07`): the pointwise function sampled on a
//! level's lattice over one 64 m tile, with exact pruning, coverage, packing into texels and
//! rows, and the tile's inputs.
//!
//! A bake reads only its tile's window (the lattice grown by the cap or the band), so a tile's
//! bytes never depend on the batch, tile order or worker count.

use super::coverage::rasterize;
use super::grade::{CAP_M, ReachBounds, reach};
use super::sample::{BaseGrid, BaseWindow, NO_SEAM, evaluate};
use super::seams::{Seam, SeamScratch, gather_road_seams};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem};
use crate::simulation::terrain::TerrainSystem;
use rayon::prelude::*;
use std::cell::RefCell;

/// Tile side, metres; tiles are aligned to the world origin.
pub const TILE_M: f64 = 64.0;
// Seam bin side over a tile's lattice, metres.
const BIN_M: f64 = 4.0;
// Extra window growth beyond the cap or band: gathering more seams changes no value.
const WINDOW_PAD_M: f64 = 1.0;

/// A 64 m tile: `floor(x / 64 m)`, `floor(z / 64 m)`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TileKey {
    /// Tile column.
    pub x: i32,
    /// Tile row.
    pub z: i32,
}

impl TileKey {
    /// The tile holding a world XZ point.
    pub fn at(x: f64, z: f64) -> Self {
        Self {
            x: (x / TILE_M).floor() as i32,
            z: (z / TILE_M).floor() as i32,
        }
    }

    /// World XZ of the tile's lower corner.
    pub fn origin(self) -> [f64; 2] {
        [f64::from(self.x) * TILE_M, f64::from(self.z) * TILE_M]
    }
}

/// A sampling level of a tile.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Level {
    /// 0.5 m, to 256 m from the camera.
    #[default]
    Gl0,
    /// 1 m.
    Gl1,
    /// 2 m.
    Gl2,
    /// The base lattice at a patch LOD step of `step` base cells (2, 4 or 8), plus the tile's
    /// sides.
    Gl3 {
        /// Base cells per lattice step.
        step: u8,
    },
}

impl Level {
    /// Lattice spacing in metres over a base of `cell_m` cells.
    pub fn spacing_m(self, cell_m: f64) -> f64 {
        match self {
            Self::Gl0 => 0.5,
            Self::Gl1 => 1.0,
            Self::Gl2 => 2.0,
            Self::Gl3 { step } => f64::from(step) * cell_m,
        }
    }

    /// Footprint distance band `B = max(2 m, 3 x spacing)`: within it every sample names its
    /// nearest road seam.
    pub fn band_m(self, cell_m: f64) -> f64 {
        (3.0 * self.spacing_m(cell_m)).max(2.0)
    }

    /// Seam row capacity of one slot (`T7`): at least twice the measured maximum (`GL2`'s 6 m
    /// band named 116 seams in a reference tile). `GL3`'s 128 is its largest lattice (at most
    /// 11 x 11 points on a base of 8 m cells, each naming one seam) rounded up.
    pub fn seam_capacity(self) -> usize {
        match self {
            Self::Gl0 => 512,
            Self::Gl1 | Self::Gl2 => 256,
            Self::Gl3 { .. } => 128,
        }
    }
}

/// What a tile's values were baked from, compared exactly to adopt a preview's tile at commit:
/// the window's seam keys (heights included) in key order, the covering top triangles' bits
/// sorted, and the base revisions of the 512 m chunks under the window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileInputs {
    /// Seam keys, sorted.
    pub seams: Vec<[u64; 6]>,
    /// Top triangle vertex bits, sorted and unique.
    pub triangles: Vec<[u64; 9]>,
    /// `((chunk x, chunk z), revision)` per 512 m chunk under the window, sorted.
    pub base_revisions: Vec<((i32, i32), u64)>,
}

/// Measurements of one bake, for `T1`, `T2` and `T7`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BakeStats {
    /// Seams in the window.
    pub window_seams: usize,
    /// Sum and largest of the seams' reaches, metres.
    pub reach_sum_m: f64,
    /// Largest reach, metres.
    pub reach_max_m: f64,
    /// Candidate seams over every sample (the bins' list lengths).
    pub candidates: u64,
    /// Ground samples whose cones conflict (`L > U`).
    pub conflicts: u32,
    /// Conflict samples within one lattice spacing of a road seam (`T2`).
    pub seam_conflicts: u32,
}

/// One baked tile at one level: the texels with their apron and the seam row, packed for
/// upload, with the inputs they came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TilePayload {
    /// The tile.
    pub tile: TileKey,
    /// The level.
    pub level: Level,
    /// Lattice points per axis, apron included (`x`, `z`).
    pub samples: [usize; 2],
    /// Per lattice point, rows along +z, points along +x: `delta` (graded minus base, metres),
    /// `sdf` (distance to the nearest road seam clamped to the band, negative under a road),
    /// the nearest road seam's index in [`Self::seams`] within the band or `-1`, and the
    /// material word `16 (e + 1) + class` (0 until sites, `TERRAIN-07-C`).
    pub texels: Vec<[f32; 4]>,
    /// The road seams the texels name, tile-local `[ax, az, bx, bz]` (metres from the tile
    /// origin), road on the left, in key order.
    pub seams: Vec<[f32; 4]>,
    /// Material edges likewise (empty until sites, `TERRAIN-07-C`).
    pub edges: Vec<[f32; 4]>,
    /// Whether any sample is graded, covered or within the band of a seam.
    pub graded: bool,
    /// What the tile was baked from.
    pub inputs: TileInputs,
    /// Bake measurements.
    pub stats: BakeStats,
}

/// Why a tile cannot be represented (`T7`): nothing is dropped silently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BakeError {
    /// More road seams are named than the level's slot holds.
    SeamCapacity {
        /// The tile.
        tile: TileKey,
        /// The level.
        level: Level,
        /// Seams named.
        count: usize,
        /// The level's capacity.
        capacity: usize,
    },
}

/// Per-worker bake buffers, reused across tiles.
#[derive(Default)]
pub(super) struct BakeScratch {
    seams: SeamScratch,
    window: Vec<Seam>,
    base: BaseWindow,
    xs: Vec<f64>,
    zs: Vec<f64>,
    seam_bins: Vec<[u32; 4]>,
    bin_starts: Vec<u32>,
    bin_items: Vec<u32>,
    bin_cursor: Vec<u32>,
    pieces: Vec<PieceId>,
    road: Vec<f32>,
    nearest: Vec<u32>,
    row_of: Vec<u32>,
}

thread_local! {
    // One scratch per thread: pool workers keep theirs between bakes.
    static SCRATCH: RefCell<BakeScratch> = RefCell::default();
}

/// Bakes `tiles` at `level` into `out` (resized to one payload per tile, in tile order, its
/// buffers reused), in parallel on the shared pool. Each tile reads only its own window, so the
/// bytes do not depend on the batch, the order or the worker count. Returns the error of the
/// lowest failing tile.
pub fn bake_tiles(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    tiles: &[TileKey],
    level: Level,
    out: &mut Vec<TilePayload>,
) -> Result<(), BakeError> {
    bake_tiles_with(terrain, roadbed, tiles, level, true, out)
}

pub(super) fn bake_tiles_with(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    tiles: &[TileKey],
    level: Level,
    prune: bool,
    out: &mut Vec<TilePayload>,
) -> Result<(), BakeError> {
    out.resize_with(tiles.len(), TilePayload::default);
    let failure = crate::thread_pools::install(|| {
        out.par_iter_mut()
            .zip(tiles.par_iter())
            .filter_map(|(payload, &tile)| {
                SCRATCH.with_borrow_mut(|scratch| {
                    bake_tile(terrain, roadbed, tile, level, prune, scratch, payload).err()
                })
            })
            .min_by_key(|error| match error {
                BakeError::SeamCapacity { tile, .. } => *tile,
            })
    });
    failure.map_or(Ok(()), Err)
}

// The lattice coordinates of one axis of a tile at `level`, apron included: one spacing
// beyond each side at `GL0`-`GL2`; at `GL3` the sides, the base step lines between them, and
// the step line beyond each side. Every value is exact, so neighbours share border points and
// levels share their common points bitwise.
fn lattice_axis(origin_m: f64, level: Level, grid: &BaseGrid, axis: usize, out: &mut Vec<f64>) {
    out.clear();
    match level {
        Level::Gl3 { step } => {
            let step_m = f64::from(step) * grid.cell_m;
            let line = |m: i64| -grid.half[axis] + m as f64 * step_m;
            let end = origin_m + TILE_M;
            let mut m = ((origin_m + grid.half[axis]) / step_m).floor() as i64;
            while line(m) >= origin_m {
                m -= 1;
            }
            out.push(line(m));
            out.push(origin_m);
            m += 1;
            while line(m) <= origin_m {
                m += 1;
            }
            while line(m) < end {
                out.push(line(m));
                m += 1;
            }
            out.push(end);
            while line(m) <= end {
                m += 1;
            }
            out.push(line(m));
        }
        _ => {
            let spacing = level.spacing_m(grid.cell_m);
            let steps = (TILE_M / spacing) as i64;
            out.extend((-1..=steps + 1).map(|i| origin_m + i as f64 * spacing));
        }
    }
}

/// Both lattice axes of a tile at `level` over the terrain's base lattice.
pub(super) fn lattice(
    terrain: &TerrainSystem,
    tile: TileKey,
    level: Level,
    xs: &mut Vec<f64>,
    zs: &mut Vec<f64>,
) {
    let grid = BaseGrid::of(terrain);
    let origin = tile.origin();
    lattice_axis(origin[0], level, &grid, 0, xs);
    lattice_axis(origin[1], level, &grid, 1, zs);
}

pub(super) fn grown(rect: [f64; 4], by: f64) -> [f64; 4] {
    [rect[0] - by, rect[1] - by, rect[2] + by, rect[3] + by]
}

pub(super) fn meets(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// The window a tile's bake reads at `level`: its lattice rectangle grown by the cap or the
/// band, whichever is wider.
pub(super) fn window(rect: [f64; 4], band: f64) -> [f64; 4] {
    grown(rect, CAP_M.max(band) + WINDOW_PAD_M)
}

// Bakes one tile. `prune` false evaluates every window seam at every sample: the reference the
// pruned bake must equal bitwise.
fn bake_tile(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    tile: TileKey,
    level: Level,
    prune: bool,
    scratch: &mut BakeScratch,
    out: &mut TilePayload,
) -> Result<(), BakeError> {
    let grid = BaseGrid::of(terrain);
    let band = level.band_m(grid.cell_m);
    let origin = tile.origin();
    lattice(terrain, tile, level, &mut scratch.xs, &mut scratch.zs);
    let (xs, zs) = (&scratch.xs, &scratch.zs);
    let rect = [xs[0], zs[0], xs[xs.len() - 1], zs[zs.len() - 1]];
    let window = window(rect, band);
    gather_road_seams(roadbed, window, &mut scratch.seams, &mut scratch.window);
    scratch.base.fill(terrain, window);
    let seams = &scratch.window;
    let mut stats = BakeStats {
        window_seams: seams.len(),
        ..BakeStats::default()
    };

    // Reach per seam, then the seams in each 4 m bin over the lattice (CSR).
    let slope = scratch.base.slope();
    let window_seams = seams
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |range, seam| {
            let (lo, hi) = seam.height_range();
            (range.0.min(lo), range.1.max(hi))
        });
    let bins = [0, 1].map(|axis| (((rect[axis + 2] - rect[axis]) / BIN_M).ceil() as usize).max(1));
    let bin_of = |axis: usize, value: f64| {
        (((value - rect[axis]) / BIN_M).floor().max(0.0) as usize).min(bins[axis] - 1)
    };
    scratch.seam_bins.clear();
    scratch.bin_starts.clear();
    scratch.bin_starts.resize(bins[0] * bins[1] + 1, 0);
    for seam in seams {
        let grow = if prune {
            let reach = reach(&ReachBounds {
                seam: seam.height_range(),
                base_under: scratch.base.range(seam.box_xz()),
                window_seams,
                slope,
            });
            stats.reach_sum_m += reach;
            stats.reach_max_m = stats.reach_max_m.max(reach);
            reach.max(band)
        } else {
            f64::INFINITY
        };
        let reach_box = grown(seam.box_xz(), grow);
        let range = if meets(reach_box, rect) {
            [
                bin_of(0, reach_box[0]) as u32,
                bin_of(0, reach_box[2]) as u32,
                bin_of(1, reach_box[1]) as u32,
                bin_of(1, reach_box[3]) as u32,
            ]
        } else {
            [1, 0, 1, 0]
        };
        scratch.seam_bins.push(range);
        for bz in range[2]..=range[3] {
            for bx in range[0]..=range[1] {
                scratch.bin_starts[bz as usize * bins[0] + bx as usize + 1] += 1;
            }
        }
    }
    for bin in 0..bins[0] * bins[1] {
        scratch.bin_starts[bin + 1] += scratch.bin_starts[bin];
    }
    scratch.bin_items.clear();
    scratch
        .bin_items
        .resize(scratch.bin_starts[bins[0] * bins[1]] as usize, 0);
    scratch.bin_cursor.clear();
    scratch
        .bin_cursor
        .extend_from_slice(&scratch.bin_starts[..bins[0] * bins[1]]);
    // Seams enter their bins in window order, so every bin lists them in key order.
    for (index, range) in scratch.seam_bins.iter().enumerate() {
        for bz in range[2]..=range[3] {
            for bx in range[0]..=range[1] {
                let cursor = &mut scratch.bin_cursor[bz as usize * bins[0] + bx as usize];
                scratch.bin_items[*cursor as usize] = index as u32;
                *cursor += 1;
            }
        }
    }

    rasterize(
        roadbed,
        xs,
        zs,
        &mut scratch.pieces,
        &mut scratch.road,
        Some(&mut out.inputs.triangles),
    );

    // Sample every lattice point.
    let spacing = level.spacing_m(grid.cell_m);
    let count = xs.len() * zs.len();
    out.texels.clear();
    out.texels.reserve(count);
    scratch.nearest.clear();
    let mut graded = false;
    for (j, &z) in zs.iter().enumerate() {
        let bz = bin_of(1, z);
        for (i, &x) in xs.iter().enumerate() {
            let bin = bz * bins[0] + bin_of(0, x);
            let list = &scratch.bin_items
                [scratch.bin_starts[bin] as usize..scratch.bin_starts[bin + 1] as usize];
            stats.candidates += list.len() as u64;
            let road = scratch.road[j * xs.len() + i];
            if list.is_empty() && road == f32::NEG_INFINITY {
                // Nothing within reach or band and uncovered: `evaluate` returns the base itself,
                // so the delta is zero and the distance the band, without reading the base.
                out.texels.push([0.0, band as f32, -1.0, 0.0]);
                scratch.nearest.push(NO_SEAM);
                continue;
            }
            let base = scratch.base.at(x, z);
            let value = evaluate(
                x,
                z,
                base,
                (road > f32::NEG_INFINITY).then_some(road),
                seams,
                list.iter().copied(),
                band,
            );
            if value.conflict {
                stats.conflicts += 1;
                if value.nearest_distance <= spacing {
                    stats.seam_conflicts += 1;
                }
            }
            let delta = (value.height - base) as f32;
            graded |= delta != 0.0 || value.sdf < band;
            out.texels.push([delta, value.sdf as f32, -1.0, 0.0]);
            scratch.nearest.push(value.nearest);
        }
    }

    // The seam row: the named seams in key order.
    scratch.row_of.clear();
    scratch.row_of.resize(seams.len(), NO_SEAM);
    for &nearest in &scratch.nearest {
        if nearest != NO_SEAM {
            scratch.row_of[nearest as usize] = 0;
        }
    }
    out.seams.clear();
    for (index, row) in scratch.row_of.iter_mut().enumerate() {
        if *row != NO_SEAM {
            *row = out.seams.len() as u32;
            let seam = &seams[index];
            out.seams.push([
                (seam.a[0] - origin[0]) as f32,
                (seam.a[2] - origin[1]) as f32,
                (seam.b[0] - origin[0]) as f32,
                (seam.b[2] - origin[1]) as f32,
            ]);
        }
    }
    for (texel, &nearest) in out.texels.iter_mut().zip(&scratch.nearest) {
        if nearest != NO_SEAM {
            texel[2] = scratch.row_of[nearest as usize] as f32;
        }
    }

    out.tile = tile;
    out.level = level;
    out.samples = [xs.len(), zs.len()];
    out.edges.clear();
    out.graded = graded;
    out.stats = stats;
    out.inputs.seams.clear();
    out.inputs.seams.extend(seams.iter().map(Seam::key));
    base_revisions(terrain, window, &mut out.inputs.base_revisions);
    let capacity = level.seam_capacity();
    if out.seams.len() > capacity {
        return Err(BakeError::SeamCapacity {
            tile,
            level,
            count: out.seams.len(),
            capacity,
        });
    }
    Ok(())
}

// The base revisions of the 512 m chunks under `window`, in key order.
fn base_revisions(terrain: &TerrainSystem, window: [f64; 4], out: &mut Vec<((i32, i32), u64)>) {
    out.clear();
    let chunk = f64::from(RegionGraph::CHUNK_SIZE);
    let key = |value: f64| (value / chunk).floor() as i32;
    for cx in key(window[0])..=key(window[2]) {
        for cz in key(window[1])..=key(window[3]) {
            out.push(((cx, cz), terrain.height_revision_for_chunk((cx, cz))));
        }
    }
}
