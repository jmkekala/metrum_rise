// SPDX-License-Identifier: GPL-2.0-only

//! Tiles, level lattices and the tile bake (`TERRAIN-07`): the pointwise function sampled on a
//! level's lattice over one 64 m tile, with the two-pass exact pruning (`prune`), coverage,
//! packing into texels and rows, and the tile's inputs.
//!
//! A bake reads only its tile's window (the lattice grown by the cap or the band), so a tile's
//! bytes never depend on the batch, tile order or worker count.

use super::coverage::{rasterize, visited_triangles};
use super::grade::{CAP_M, continues_ground};
use super::portal::{Finished, PortalWindow, open};
use super::prune::{
    BinGrid, BinLists, Lifted, bin_side, finish_second_pass, list_first_pass, list_second_pass,
};
use super::sample::{
    AxisStop, BaseGrid, BaseWindow, Listed, NO_SEAM, Value, evaluate, evaluate_lanes,
};
use super::seams::{Seam, SeamScratch, gather_road_seams, gather_wall_seams};
use super::sites::{NO_SITE, SiteBuffer, SiteSource};
use super::walls::{WallFace, WallScratch, abutment_bottom, build_structure_walls, build_walls};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem};
use crate::simulation::terrain::TerrainSystem;
use rayon::prelude::*;
use std::sync::{Mutex, PoisonError};

/// Tile side, metres; tiles are aligned to the world origin.
pub const TILE_M: f64 = 64.0;
// Extra window growth beyond the cap or band: gathering more seams changes no value.
const WINDOW_PAD_M: f64 = 1.0;
// Lattice rows per band of a tile's sampling (`TERRAIN-07-E`, T27 (a)): the unit the shared
// pool balances a batch's samples in, so a few dear tiles spread over every worker. Each
// sample's value is its own, so the bytes do not depend on it.
const BAND_ROWS: usize = 16;

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

/// A sampling level of a tile, ordered finest first.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

    /// Seam row capacity of one slot (`T7`): at least twice the measured maximum (`GL0` 245 in
    /// the 100k city, `GL1` 244 and `GL2` 212 in the 14,000-resident populated save). `GL3`'s
    /// 128 is its largest lattice (at most 11 x 11 points on a base of 8 m cells, each naming one
    /// seam) rounded up.
    pub fn seam_capacity(self) -> usize {
        match self {
            Self::Gl0 | Self::Gl1 | Self::Gl2 => 512,
            Self::Gl3 { .. } => 128,
        }
    }

    /// Material edge row capacity of one slot (`T7`, `TERRAIN-07-C`): at least twice the
    /// measured maximum (`GL0` 113 in the 100k city, `GL1` 108 and `GL2` 119 in the populated
    /// save: a wider band names more edges); `GL3` takes its lattice bound like its seams (each
    /// lattice point names at most one edge).
    pub fn edge_capacity(self) -> usize {
        match self {
            Self::Gl0 | Self::Gl1 | Self::Gl2 => 256,
            Self::Gl3 { .. } => 128,
        }
    }
}

/// What a tile's values were baked from, compared exactly to adopt a preview's tile at commit:
/// the window's seam keys (heights included) in key order, the covering top triangles' bits
/// sorted, and the base revisions of the 512 m chunks under the window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileInputs {
    /// The road seam keys, sorted, then the wall seam keys the structure walls read, sorted
    /// (`TERRAIN-07-C`), then those walls' abutment bottoms at bridge mouths as seam keys, in
    /// the walls' order, then the portal rows' corner segments and the bore seams the portal
    /// opening reads (T22 5 (a)), each sorted.
    pub seams: Vec<[u64; 6]>,
    /// Top triangle vertex bits, sorted and unique, then the portal opening's deck triangles,
    /// sorted and unique.
    pub triangles: Vec<[u64; 9]>,
    /// `((chunk x, chunk z), revision)` per 512 m chunk under the window, sorted.
    pub base_revisions: Vec<((i32, i32), u64)>,
    /// The window's building sites in canonical order (`TERRAIN-07-C`): per site its
    /// footprint corner count and corner bits, its support height's bits, its surface count and
    /// per surface the material class, corner count and corner bits.
    pub sites: Vec<u32>,
}

/// Measurements of one bake, for `T1`, `T2` and `T7`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BakeStats {
    /// Seams in the window.
    pub window_seams: usize,
    /// Sum of the seams' first-pass reaches (the boxes their bins are visited in), metres.
    pub reach_sum_m: f64,
    /// Largest first-pass reach, metres.
    pub reach_max_m: f64,
    /// Pass 1's candidate seams over every sample (the bins' list lengths).
    pub listed: u64,
    /// Seams evaluated over every sample, both passes: pass 1's candidates before each sample's
    /// early exit, and pass 2's.
    pub candidates: u64,
    /// Ground samples the second pass finished: a cut's upper bound held them below the base.
    pub second_pass_samples: u32,
    /// The second pass's candidate seams over those samples, included in `candidates`.
    pub second_pass_candidates: u64,
    /// Ground samples whose evaluated cones conflict (`L > U`, where the ground takes `L`). A
    /// fill's `L` above the upper bound of a seam whose cone holds the base is not evaluated,
    /// and not counted: the ground is that `L` either way.
    pub conflicts: u32,
    /// Conflict samples within one lattice spacing of a road seam.
    pub seam_conflicts: u32,
    /// Samples the portal opening cut (`TERRAIN-07-C`, T22 5 (a)).
    pub portal_cut: u32,
    /// Ground samples beside a portal opening that name a bore seam.
    pub bore_named: u32,
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
    /// `sdf` (distance to the nearest road seam clamped to the band, negative under a road;
    /// near a portal the nearer of it and the nearest bore seam, negative in the opening, T22
    /// 5 (a)), that seam's index in [`Self::seams`] within the band or `-1`, and the
    /// material word `16 (e + 1) + class`: `class` the material class of the first site
    /// surface holding the point (0 none, 1 asphalt, 2 concrete), `e` the nearest material
    /// edge's index in [`Self::edges`] within the band, else `-1`.
    pub texels: Vec<[f32; 4]>,
    /// The road seams the texels name, tile-local `[ax, az, bx, bz]` (metres from the tile
    /// origin), road on the left, in key order, then the bore seams they name, the deck on the
    /// left, in key order.
    pub seams: Vec<[f32; 4]>,
    /// Each named seam's heights at its ends `a` and `b` (world metres), in [`Self::seams`]'
    /// order: the road top along the seam, which the cells' outer-band overlap compares the
    /// terrain with (T24 (a)).
    pub seam_heights: Vec<[f32; 2]>,
    /// The material edges the texels name, tile-local `[ax, az, bx, bz]`, the surface on the
    /// left, in key order.
    pub edges: Vec<[f32; 4]>,
    /// The conflict walls along the road seams in the tile (`T2` (c)), seam by seam in key
    /// order, each seam's faces along it; then the structure walls along its wall seams
    /// (`TERRAIN-07-C`, T22 4 (a)), likewise, then the portal openings' side walls along the
    /// bore seams (T22 5 (a)), likewise.
    pub walls: Vec<WallFace>,
    /// Whether any sample is graded, covered, in a pad, paved or within the band of a seam or
    /// a material edge.
    pub graded: bool,
    /// What the tile was baked from.
    pub inputs: TileInputs,
    /// Bake measurements.
    pub stats: BakeStats,
}

impl BakeStats {
    // Adds the sample counts of one row band's passes (`TERRAIN-07-E`).
    fn add_counts(&mut self, band: &Self) {
        self.listed += band.listed;
        self.candidates += band.candidates;
        self.second_pass_samples += band.second_pass_samples;
        self.second_pass_candidates += band.second_pass_candidates;
        self.conflicts += band.conflicts;
        self.seam_conflicts += band.seam_conflicts;
    }
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
    /// More material edges are named than the level's slot holds.
    EdgeCapacity {
        /// The tile.
        tile: TileKey,
        /// The level.
        level: Level,
        /// Edges named.
        count: usize,
        /// The level's capacity.
        capacity: usize,
    },
    /// The site source cannot answer rectangle queries without a scan (its index is being
    /// rebuilt): the caller prepares it and bakes again. Nothing is baked.
    SitesUnavailable,
}

impl BakeError {
    // The failing tile, for the lowest-tile rule; none before any tile is baked.
    fn tile(&self) -> Option<TileKey> {
        match *self {
            Self::SeamCapacity { tile, .. } | Self::EdgeCapacity { tile, .. } => Some(tile),
            Self::SitesUnavailable => None,
        }
    }
}

/// One tile's bake buffers, reused across tiles: a tile holds them from its gathers to its rows
/// while its row bands sample on any worker.
#[derive(Default)]
pub(super) struct BakeScratch {
    seams: SeamScratch,
    window: Vec<Seam>,
    // The wall seams within the band of the lattice, in key order (`TERRAIN-07-C`): no cone,
    // so outside the window's grading seams; per seam its abutment bottom at a bridge's mouth.
    wall_seams: Vec<Seam>,
    wall_bottoms: Vec<Option<[f64; 2]>>,
    // The covered lattice points that continue the ground across a structure mouth.
    mouth_points: Vec<u32>,
    base: BaseWindow,
    xs: Vec<f64>,
    zs: Vec<f64>,
    // Per lattice column its bin column and base stop.
    columns: Vec<(u32, AxisStop)>,
    // Each seam's first-pass bin range `[x0, x1, z0, z1]`, empty as `[1, 0, 1, 0]`.
    seam_bins: Vec<[u32; 4]>,
    bins: BinGrid,
    // Each bin's lowest first-pass ground below the base, `+inf` without one.
    bin_floor: Vec<f64>,
    pairs: Vec<(u32, Listed)>,
    first: BinLists,
    second: BinLists,
    pieces: Vec<PieceId>,
    road: Vec<f32>,
    nearest: Vec<u32>,
    row_of: Vec<u32>,
    // Per row band its ground samples left below their base and its counts.
    bands: Vec<BandScratch>,
    sites: SiteBuffer,
    // Per lattice point: the canonical site whose footprint holds it, the material class, and
    // the nearest material edge within the band with its distance.
    pads: Vec<u32>,
    classes: Vec<u8>,
    edges_near: Vec<(u32, f64)>,
    walls: WallScratch,
    // The portal opening's parts around the lattice, the deck per lattice point and each
    // point's nearest bore seam within the band (`TERRAIN-07-C`).
    portals: PortalWindow,
    decks: Vec<f32>,
    bores_near: Vec<(u32, f64)>,
}

// One row band's sampling products (`TERRAIN-07-E`): its ground samples left below their base
// for pass 2, its sample counts and whether a sample it finished is graded.
#[derive(Default)]
struct BandScratch {
    lifted: Vec<Lifted>,
    stats: BakeStats,
    graded: bool,
}

// Bake buffers not in use. A tile takes one set for its whole bake and its row bands read it on
// other workers, so the buffers belong to the tile, not to a worker; the pool holds as many as
// were ever in use at once.
static SCRATCHES: Mutex<Vec<BakeScratch>> = Mutex::new(Vec::new());

// Runs `op` with bake buffers from the pool and returns them after.
fn with_scratch<R>(op: impl FnOnce(&mut BakeScratch) -> R) -> R {
    let pool = || SCRATCHES.lock().unwrap_or_else(PoisonError::into_inner);
    let mut scratch = pool().pop().unwrap_or_default();
    let result = op(&mut scratch);
    pool().push(scratch);
    result
}

/// Bakes `tiles` at `level` into `out` (resized to one payload per tile, in tile order, its
/// buffers reused), in parallel on the shared pool, over the base, the roadbed's grounded
/// roads and the building sites of `sites`. Each tile reads only its own window, so the bytes
/// do not depend on the batch, the order or the worker count. Returns the error of the lowest
/// failing tile.
pub fn bake_tiles(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    tiles: &[TileKey],
    level: Level,
    out: &mut Vec<TilePayload>,
) -> Result<(), BakeError> {
    bake_tiles_with(terrain, roadbed, sites, tiles, level, Evaluation::Lanes, out)
}

/// How a bake evaluates its samples: the reference over every seam within the cap, or pruned,
/// one point at a time or a row's points in one bin together in lanes (the bake's own way).
/// The three give the same bits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Evaluation {
    Unpruned,
    // The lanes' reference in the tests.
    #[cfg(test)]
    Scalar,
    Lanes,
}

// Points per lane group: one bin row at `GL1` (4 m bins of 1 m), half of one at `GL0`.
const LANES: usize = 4;

pub(super) fn bake_tiles_with(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    tiles: &[TileKey],
    level: Level,
    evaluation: Evaluation,
    out: &mut Vec<TilePayload>,
) -> Result<(), BakeError> {
    if !sites.ready() {
        return Err(BakeError::SitesUnavailable);
    }
    out.resize_with(tiles.len(), TilePayload::default);
    let inputs = Inputs {
        terrain,
        roadbed,
        sites,
    };
    let failure = crate::thread_pools::install(|| {
        out.par_iter_mut()
            .zip(tiles.par_iter())
            .filter_map(|(payload, &tile)| {
                with_scratch(|scratch| {
                    bake_tile(&inputs, tile, level, evaluation, scratch, payload).err()
                })
            })
            .min_by_key(BakeError::tile)
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

// What a bake reads.
struct Inputs<'a, S> {
    terrain: &'a TerrainSystem,
    roadbed: &'a RoadbedSystem,
    sites: &'a S,
}

// The covered lattice points that continue the ground across a structure mouth
// ([`continues_ground`]) take the ground's value, evaluated over every window seam, so a pruned
// bake equals the unpruned one by construction; their `sdf` and nearest seam stay the road's,
// so the mask is unchanged. Visits the lattice points within the band of each wall seam's box.
// `O(points within the band of the wall seams x (walls + window seams))`.
#[allow(clippy::too_many_arguments)]
fn continue_ground_at_mouths(
    (seams, walls): (&[Seam], &[Seam]),
    (xs, zs): (&[f64], &[f64]),
    road: &[f32],
    pad: impl Fn(usize) -> Option<f64>,
    base_at: impl Fn(usize, usize) -> f64,
    (nearest, band): (&[u32], f64),
    points: &mut Vec<u32>,
    texels: &mut [[f32; 4]],
) {
    let span = |lines: &[f64], lo: f64, hi: f64| {
        lines.partition_point(|&v| v < lo - band)..lines.partition_point(|&v| v <= hi + band)
    };
    points.clear();
    for wall in walls {
        let area = wall.box_xz();
        for j in span(zs, area[1], area[3]) {
            for i in span(xs, area[0], area[2]) {
                let index = j * xs.len() + i;
                if road[index] == f32::NEG_INFINITY {
                    continue;
                }
                let (x, z) = (xs[i], zs[j]);
                let road_distance = match nearest[index] {
                    NO_SEAM => f64::INFINITY,
                    seam => seams[seam as usize].at(x, z).distance,
                };
                if continues_ground(walls, x, z, road_distance, band) {
                    points.push(index as u32);
                }
            }
        }
    }
    points.sort_unstable();
    points.dedup();
    for &index in points.iter() {
        let (i, j) = (index as usize % xs.len(), index as usize / xs.len());
        let base = base_at(i, j);
        let all = (0..seams.len() as u32).map(Listed::unbounded);
        let ground = evaluate(xs[i], zs[j], base, None, pad(index as usize), seams, all, band);
        texels[index as usize][0] = (ground.height - base) as f32;
    }
}

// A texel's named seam: the window's seams (`0..seams.len()`), then the bore seams, then the wall
// seams.
fn named_seam<'a>(
    (seams, bores, walls): (&'a [Seam], &'a [Seam], &'a [Seam]),
    index: usize,
) -> &'a Seam {
    if index < seams.len() {
        &seams[index]
    } else if index < seams.len() + bores.len() {
        &bores[index - seams.len()]
    } else {
        &walls[index - seams.len() - bores.len()]
    }
}

// Names the bridge mouth rows (the wall seams with an abutment bottom) in the mask
// (`TERRAIN-07-D` round 9): a lattice point within the band of a row that is nearer than its
// named seam, by the nearest order with the rows after the window and bore seams, names that row,
// its `sdf` the distance, negative where a road covers it. Coverage ends at the row, which is no
// road seam, so before the cut edge across a bridge mouth fell on the bilinear zero of the road
// seams' clamped distances, up to a metre past the row on the bridge's side, and the ground
// stopped short of the abutment wall there (the `bridge_t` `under_bridge` dashed line). Only the
// mask moves: heights are final. Returns the points named. `O(points within the band of the rows
// x rows)`.
fn name_mouth_rows(
    (seams, bores): (&[Seam], &[Seam]),
    (walls, bottoms): (&[Seam], &[Option<[f64; 2]>]),
    (xs, zs): (&[f64], &[f64]),
    road: &[f32],
    band: f64,
    nearest: &mut [u32],
    texels: &mut [[f32; 4]],
) -> u32 {
    let span = |lines: &[f64], lo: f64, hi: f64| {
        lines.partition_point(|&v| v < lo - band)..lines.partition_point(|&v| v <= hi + band)
    };
    let first_wall = (seams.len() + bores.len()) as u32;
    let mut named = 0;
    for (k, (row, bottom)) in walls.iter().zip(bottoms).enumerate() {
        if bottom.is_none() {
            continue;
        }
        let code = first_wall + k as u32;
        let area = row.box_xz();
        for j in span(zs, area[1], area[3]) {
            for i in span(xs, area[0], area[2]) {
                let index = j * xs.len() + i;
                let (x, z) = (xs[i], zs[j]);
                let at = row.at(x, z);
                if at.distance >= band {
                    continue;
                }
                let current = nearest[index];
                let nearer = current == NO_SEAM || {
                    let other = named_seam((seams, bores, walls), current as usize).at(x, z);
                    at.nearer_than(code, &other, current)
                };
                if !nearer {
                    continue;
                }
                named += u32::from(current == NO_SEAM || current < first_wall);
                nearest[index] = code;
                let covered = road[index] > f32::NEG_INFINITY;
                texels[index][1] = if covered { -at.distance } else { at.distance } as f32;
            }
        }
    }
    named
}

// Bakes one tile with `evaluation`.
fn bake_tile(
    inputs: &Inputs<'_, impl SiteSource>,
    tile: TileKey,
    level: Level,
    evaluation: Evaluation,
    scratch: &mut BakeScratch,
    out: &mut TilePayload,
) -> Result<(), BakeError> {
    let (terrain, roadbed) = (inputs.terrain, inputs.roadbed);
    let BakeScratch {
        seams: seam_scratch,
        window: seams,
        wall_seams,
        wall_bottoms,
        mouth_points,
        base,
        xs,
        zs,
        columns,
        seam_bins,
        bins,
        bin_floor,
        pairs,
        first,
        second,
        pieces,
        road,
        nearest,
        row_of,
        bands,
        sites,
        pads,
        classes,
        edges_near,
        walls,
        portals,
        decks,
        bores_near,
    } = scratch;
    let grid = BaseGrid::of(terrain);
    let band = level.band_m(grid.cell_m);
    let spacing = level.spacing_m(grid.cell_m);
    let origin = tile.origin();
    lattice(terrain, tile, level, xs, zs);
    let rect = [xs[0], zs[0], xs[xs.len() - 1], zs[zs.len() - 1]];
    let window = window(rect, band);
    gather_road_seams(roadbed, window, seam_scratch, seams);
    gather_wall_seams(
        roadbed,
        grown(rect, band),
        seam_scratch,
        (wall_seams, wall_bottoms),
    );
    // Road seams first (indices `0..road_count`, in key order, the rows the texels name), then
    // the sites' pad seams in key order.
    let road_count = seams.len();
    sites.gather(inputs.sites, window);
    sites.push_pad_seams(seams);
    base.fill(terrain, window);
    let mut stats = BakeStats {
        window_seams: seams.len(),
        ..BakeStats::default()
    };

    // Pass 1's seams in each bin over the lattice.
    bins.fill(base, rect, bin_side(spacing));
    let prune = evaluation != Evaluation::Unpruned;
    list_first_pass(seams, bins, base, band, prune, seam_bins, pairs, &mut stats);
    first.build(bins.len(), pairs);

    rasterize(
        roadbed,
        xs,
        zs,
        pieces,
        road,
        Some(&mut out.inputs.triangles),
    );
    portals.gather(roadbed, rect, band, pieces);
    portals.rasterize(xs, zs, decks);
    // Without sites every lattice point is outside pads and surfaces: the rasters stay empty,
    // which reads as no pad, class 0 and no edge. The nearest edges are searched in the row
    // bands below.
    let (width, count) = (xs.len(), xs.len() * zs.len());
    edges_near.clear();
    if sites.is_empty() {
        pads.clear();
        classes.clear();
    } else {
        sites.rasterize_pads(xs, zs, prune, pads);
        sites.rasterize_classes(xs, zs, prune, classes);
        edges_near.resize(count, (NO_SITE, f64::INFINITY));
    }

    // Sample every lattice point over pass 1's seams in bands of rows on the shared pool
    // (`TERRAIN-07-E`, T27 (a)), beside the nearest material edges' search in the same bands,
    // then finish the ground samples left below their base by pass 2, band by band. Bands
    // write disjoint rows and their counts are summed in band order.
    out.texels.clear();
    out.texels.resize(count, [0.0; 4]);
    nearest.clear();
    nearest.resize(count, NO_SEAM);
    let band_count = zs.len().div_ceil(BAND_ROWS);
    if bands.len() < band_count {
        bands.resize_with(band_count, BandScratch::default);
    }
    let bands = &mut bands[..band_count];
    columns.clear();
    columns.extend(xs.iter().map(|&x| (bins.of(0, x), base.stop(0, x))));
    let sampling = Sampling {
        xs,
        zs,
        columns,
        bins,
        first,
        seams,
        base,
        road,
        pads,
        sites,
        band,
        spacing,
        evaluation,
    };
    let chunk = BAND_ROWS * width;
    let (xs, zs, edge_sites) = (&*xs, &*zs, &*sites);
    rayon::join(
        || {
            out.texels
                .par_chunks_mut(chunk)
                .zip(nearest.par_chunks_mut(chunk))
                .zip(bands.par_iter_mut())
                .enumerate()
                .for_each(|(index, ((texels, nearest), out))| {
                    sampling.rows(index * BAND_ROWS, texels, nearest, out)
                })
        },
        || {
            edges_near
                .par_chunks_mut(chunk)
                .zip(zs.par_chunks(BAND_ROWS))
                .for_each(|(nearest, zs)| edge_sites.nearest_edges(xs, zs, band, prune, nearest))
        },
    );
    let mut graded = false;
    bin_floor.clear();
    bin_floor.resize(bins.len(), f64::INFINITY);
    let mut lifted = 0;
    for rows in bands.iter() {
        stats.add_counts(&rows.stats);
        graded |= rows.graded;
        lifted += rows.lifted.len();
        for sample in &rows.lifted {
            let floor = &mut bin_floor[sample.bin as usize];
            *floor = floor.min(sample.ground);
        }
    }
    if lifted > 0 {
        list_second_pass(seams, bins, band, bin_floor, seam_bins, pairs);
        second.build(bins.len(), pairs);
        let (seams, second) = (&*seams, &*second);
        graded |= out
            .texels
            .par_chunks_mut(chunk)
            .zip(bands.par_iter_mut())
            .enumerate()
            .map(|(index, (texels, rows))| {
                rows.stats = BakeStats::default();
                let first = (index * chunk) as u32;
                finish_second_pass(seams, second, &rows.lifted, first, texels, &mut rows.stats)
            })
            .reduce(|| false, |a, b| a | b);
        for rows in bands.iter() {
            stats.add_counts(&rows.stats);
        }
    }
    if !wall_seams.is_empty() {
        let pad = |index: usize| {
            let site = pads.get(index).copied().unwrap_or(NO_SITE);
            (site != NO_SITE).then(|| sites.height(site))
        };
        let base_at = |i: usize, j: usize| base.at_stops(&columns[i].1, &base.stop(1, zs[j]));
        continue_ground_at_mouths(
            (seams, wall_seams),
            (xs, zs),
            road,
            pad,
            base_at,
            (nearest, band),
            mouth_points,
            &mut out.texels,
        );
    }
    // The portal opening, on the final values (T22 5 (a)).
    if !portals.is_empty() {
        let finished = Finished {
            xs,
            zs,
            road,
            pads,
            decks,
            band,
        };
        let base_at = |i: usize, j: usize| base.at_stops(&columns[i].1, &base.stop(1, zs[j]));
        (stats.portal_cut, stats.bore_named) = open(
            portals,
            seams,
            &finished,
            base_at,
            bores_near,
            &mut out.texels,
            nearest,
        );
        graded |= stats.portal_cut + stats.bore_named > 0;
    }
    // The bridge mouths' rows in the mask, on the final names.
    if wall_bottoms.iter().any(Option::is_some) {
        let named = name_mouth_rows(
            (seams, portals.bores()),
            (wall_seams, wall_bottoms),
            (xs, zs),
            road,
            band,
            nearest,
            &mut out.texels,
        );
        graded |= named > 0;
    }

    // The seam row: the named road seams in key order, then the named bore seams, then the
    // named bridge mouth rows.
    let bores = portals.bores();
    let seam_at = |index: usize| named_seam((&seams[..], bores, &wall_seams[..]), index);
    row_of.clear();
    row_of.resize(seams.len() + bores.len() + wall_seams.len(), NO_SEAM);
    for &named in nearest.iter() {
        if named != NO_SEAM {
            row_of[named as usize] = 0;
        }
    }
    out.seams.clear();
    out.seam_heights.clear();
    for (index, row) in row_of.iter_mut().enumerate() {
        if *row != NO_SEAM {
            *row = out.seams.len() as u32;
            let seam = seam_at(index);
            out.seams.push([
                (seam.a[0] - origin[0]) as f32,
                (seam.a[2] - origin[1]) as f32,
                (seam.b[0] - origin[0]) as f32,
                (seam.b[2] - origin[1]) as f32,
            ]);
            out.seam_heights.push([seam.a[1] as f32, seam.b[1] as f32]);
        }
    }
    for (texel, &named) in out.texels.iter_mut().zip(nearest.iter()) {
        if named != NO_SEAM {
            texel[2] = row_of[named as usize] as f32;
        }
    }

    // The edge row: the named material edges in key order, and each texel's material word.
    let edges = sites.edges();
    row_of.clear();
    row_of.resize(edges.len(), NO_SEAM);
    for &(named, _) in edges_near.iter() {
        if named != NO_SITE {
            row_of[named as usize] = 0;
        }
    }
    out.edges.clear();
    for (index, row) in row_of.iter_mut().enumerate() {
        if *row != NO_SEAM {
            *row = out.edges.len() as u32;
            let [ax, az, bx, bz] = edges[index];
            out.edges.push([
                (ax - origin[0]) as f32,
                (az - origin[1]) as f32,
                (bx - origin[0]) as f32,
                (bz - origin[1]) as f32,
            ]);
        }
    }
    let words = classes.iter().zip(edges_near.iter());
    for (texel, (&class, &(named, _))) in out.texels.iter_mut().zip(words) {
        let edge = if named == NO_SITE {
            -1.0
        } else {
            row_of[named as usize] as f32
        };
        texel[3] = 16.0 * (edge + 1.0) + f32::from(class);
    }
    graded |= !out.edges.is_empty() || classes.iter().any(|&class| class != 0);

    // Conflict walls along the road seams, from the seams and the finished samples.
    let heights = |i: usize, j: usize| {
        let delta = out.texels[j * xs.len() + i][0];
        base.at_stops(&columns[i].1, &base.stop(1, zs[j])) + f64::from(delta)
    };
    build_walls(
        seams,
        road_count,
        tile,
        xs,
        zs,
        evaluation != Evaluation::Unpruned,
        heights,
        walls,
        &mut out.walls,
    );
    build_structure_walls(
        (wall_seams, wall_bottoms),
        portals,
        tile,
        (xs, zs),
        heights,
        walls,
        &mut out.walls,
    );

    out.tile = tile;
    out.level = level;
    out.samples = [xs.len(), zs.len()];
    out.graded = graded;
    out.stats = stats;
    finish_inputs(
        terrain,
        window,
        &seams[..road_count],
        (wall_seams, wall_bottoms),
        portals,
        sites,
        &mut out.inputs,
    );
    let capacity = level.seam_capacity();
    if out.seams.len() > capacity {
        return Err(BakeError::SeamCapacity {
            tile,
            level,
            count: out.seams.len(),
            capacity,
        });
    }
    let capacity = level.edge_capacity();
    if out.edges.len() > capacity {
        return Err(BakeError::EdgeCapacity {
            tile,
            level,
            count: out.edges.len(),
            capacity,
        });
    }
    Ok(())
}

// What the row bands of one tile read (`TERRAIN-07-E`, T27 (a)): the set-up's products, shared
// while the bands sample on any worker.
struct Sampling<'a> {
    xs: &'a [f64],
    zs: &'a [f64],
    columns: &'a [(u32, AxisStop)],
    bins: &'a BinGrid,
    first: &'a BinLists,
    seams: &'a [Seam],
    base: &'a BaseWindow,
    road: &'a [f32],
    pads: &'a [u32],
    sites: &'a SiteBuffer,
    band: f64,
    spacing: f64,
    evaluation: Evaluation,
}

impl Sampling<'_> {
    // Samples the lattice rows from `first_row` on into `texels` and `nearest` (those rows'
    // points, row-major) over pass 1's seams, a row's points in one bin together in lanes. A
    // ground sample left below its base goes to `out.lifted` for pass 2 (by its index in the
    // tile); every other value is final. `O(points x their candidates)`, allocation-free once
    // `out.lifted` is warm.
    fn rows(
        &self,
        first_row: usize,
        texels: &mut [[f32; 4]],
        nearest: &mut [u32],
        out: &mut BandScratch,
    ) {
        let Self {
            xs,
            zs,
            columns,
            bins,
            first,
            seams,
            base,
            road,
            pads,
            sites,
            band,
            spacing,
            evaluation,
        } = *self;
        out.lifted.clear();
        out.stats = BakeStats::default();
        out.graded = false;
        let lanes = if evaluation == Evaluation::Lanes {
            LANES
        } else {
            1
        };
        let width = xs.len();
        let rows = texels.chunks_mut(width).zip(nearest.chunks_mut(width));
        for (j, (texels, nearest)) in (first_row..).zip(rows) {
            let z = zs[j];
            let (bin_row, row_stop) = (bins.of(1, z), base.stop(1, z));
            let row = j * width;
            let mut i = 0;
            while i < width {
                // The run of this row's points in the bin of point `i`, at most `lanes` long.
                let bin_column = columns[i].0;
                let bin = bins.index(bin_column, bin_row);
                let mut run = 1;
                while run < lanes && i + run < width && columns[i + run].0 == bin_column {
                    run += 1;
                }
                let list = first.of(bin);
                out.stats.listed += (list.len() * run) as u64;
                let covering = |lane: usize| {
                    let height = road[row + i + lane];
                    (height > f32::NEG_INFINITY).then_some(height)
                };
                let pad = |lane: usize| {
                    let site = pads.get(row + i + lane).copied().unwrap_or(NO_SITE);
                    (site != NO_SITE).then(|| sites.height(site))
                };
                let bare = |lane: usize| covering(lane).is_none() && pad(lane).is_none();
                if list.is_empty() && (0..run).all(bare) {
                    // Nothing within reach or band and uncovered: `evaluate` returns the base
                    // itself, so the delta is zero and the distance the band, without reading
                    // the base.
                    texels[i..i + run].fill([0.0, band as f32, -1.0, 0.0]);
                    nearest[i..i + run].fill(NO_SEAM);
                    i += run;
                    continue;
                }
                // Lanes past the run repeat its last point; their values are dropped.
                let point = |lane: usize| xs[i + lane.min(run - 1)];
                let points: [f64; LANES] = std::array::from_fn(point);
                let bases: [f64; LANES] = std::array::from_fn(|lane| {
                    base.at_stops(&columns[i + lane.min(run - 1)].1, &row_stop)
                });
                let roads: [Option<f32>; LANES] =
                    std::array::from_fn(|lane| covering(lane.min(run - 1)));
                let pad_heights: [Option<f64>; LANES] =
                    std::array::from_fn(|lane| pad(lane.min(run - 1)));
                let values: [Value; LANES] = if run == 1 {
                    let value = evaluate(
                        points[0],
                        z,
                        bases[0],
                        roads[0],
                        pad_heights[0],
                        seams,
                        list.iter().copied(),
                        band,
                    );
                    [value; LANES]
                } else {
                    evaluate_lanes(
                        &points,
                        z,
                        &bases,
                        &roads,
                        &pad_heights,
                        seams,
                        list.iter().copied(),
                        band,
                    )
                };
                for lane in 0..run {
                    let (x, base_m, value) = (points[lane], bases[lane], values[lane]);
                    out.stats.candidates += u64::from(value.evaluated);
                    let delta = (value.height - base_m) as f32;
                    let (in_band, near_seam) =
                        (value.sdf < band, value.nearest_distance <= spacing);
                    if evaluation != Evaluation::Unpruned
                        && roads[lane].is_none()
                        && pad_heights[lane].is_none()
                        && value.height < base_m
                    {
                        out.lifted.push(Lifted {
                            sample: (row + i + lane) as u32,
                            bin: bin as u32,
                            x,
                            z,
                            base: base_m,
                            ground: value.height,
                            cones: value.cones,
                            near_seam,
                            in_band,
                        });
                    } else {
                        if value.conflict {
                            out.stats.conflicts += 1;
                            out.stats.seam_conflicts += u32::from(near_seam);
                        }
                        out.graded |= delta != 0.0 || in_band || pad_heights[lane].is_some();
                    }
                    texels[i + lane] = [delta, value.sdf as f32, -1.0, 0.0];
                    nearest[i + lane] = value.nearest;
                }
                i += run;
            }
        }
    }
}

/// The inputs a bake of `tile` at `level` records ([`TilePayload::inputs`]), gathered without
/// sampling (`TERRAIN-07-E`): the window's seams, the coverage's visited triangles, the portal
/// parts, the sites and the base revisions, by the bake's own gathers and assembly, so they
/// equal the bake's inputs exactly. A preview, commit or undo compares them with a held tile's
/// to adopt it. `O(P + S log S + T + sites)` for the window, without the lattice's samples.
pub fn tile_inputs(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    tile: TileKey,
    level: Level,
    out: &mut TileInputs,
) -> Result<(), BakeError> {
    if !sites.ready() {
        return Err(BakeError::SitesUnavailable);
    }
    let inputs = Inputs {
        terrain,
        roadbed,
        sites,
    };
    with_scratch(|scratch| gather_tile_inputs(&inputs, tile, level, scratch, out));
    Ok(())
}

/// The rectangle `[min_x, min_z, max_x, max_z]` a bake of `tile` at `level` reads its seams and
/// sites from: the lattice grown by the cap or the band, the window of [`tile_inputs`]. A site
/// snapshot holding it serves the bake.
pub fn bake_window(terrain: &TerrainSystem, tile: TileKey, level: Level) -> [f64; 4] {
    with_scratch(|scratch| {
        let band = level.band_m(BaseGrid::of(terrain).cell_m);
        lattice(terrain, tile, level, &mut scratch.xs, &mut scratch.zs);
        let (xs, zs) = (&scratch.xs, &scratch.zs);
        window([xs[0], zs[0], xs[xs.len() - 1], zs[zs.len() - 1]], band)
    })
}

/// [`tile_inputs`] of every `(level, tile)` of `wanted` into `out` (one per entry, in order,
/// its buffers reused), in parallel on the shared pool.
pub fn gather_inputs(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    wanted: &[(Level, TileKey)],
    out: &mut Vec<TileInputs>,
) -> Result<(), BakeError> {
    if !sites.ready() {
        return Err(BakeError::SitesUnavailable);
    }
    out.resize_with(wanted.len(), TileInputs::default);
    let inputs = Inputs {
        terrain,
        roadbed,
        sites,
    };
    crate::thread_pools::install(|| {
        out.par_iter_mut()
            .zip(wanted.par_iter())
            .for_each(|(out, &(level, tile))| {
                with_scratch(|scratch| gather_tile_inputs(&inputs, tile, level, scratch, out))
            })
    });
    Ok(())
}

// The bake's gathers of one tile's window, without its samples.
fn gather_tile_inputs(
    inputs: &Inputs<'_, impl SiteSource>,
    tile: TileKey,
    level: Level,
    scratch: &mut BakeScratch,
    out: &mut TileInputs,
) {
    let (terrain, roadbed) = (inputs.terrain, inputs.roadbed);
    let BakeScratch {
        seams: seam_scratch,
        window: seams,
        wall_seams,
        wall_bottoms,
        xs,
        zs,
        pieces,
        sites,
        portals,
        ..
    } = scratch;
    let band = level.band_m(BaseGrid::of(terrain).cell_m);
    lattice(terrain, tile, level, xs, zs);
    let rect = [xs[0], zs[0], xs[xs.len() - 1], zs[zs.len() - 1]];
    let window = window(rect, band);
    gather_road_seams(roadbed, window, seam_scratch, seams);
    gather_wall_seams(
        roadbed,
        grown(rect, band),
        seam_scratch,
        (wall_seams, wall_bottoms),
    );
    sites.gather(inputs.sites, window);
    visited_triangles(roadbed, xs, zs, pieces, &mut out.triangles);
    portals.gather(roadbed, rect, band, pieces);
    finish_inputs(
        terrain,
        window,
        seams,
        (wall_seams, wall_bottoms),
        portals,
        sites,
        out,
    );
}

// The inputs once the window's road and wall seams, portal parts and sites are gathered and
// `out.triangles` holds the coverage's visited triangles: one assembly for the bake and
// [`tile_inputs`].
fn finish_inputs(
    terrain: &TerrainSystem,
    window: [f64; 4],
    road_seams: &[Seam],
    (wall_seams, wall_bottoms): (&[Seam], &[Option<[f64; 2]>]),
    portals: &PortalWindow,
    sites: &SiteBuffer,
    out: &mut TileInputs,
) {
    out.seams.clear();
    out.seams.extend(road_seams.iter().map(Seam::key));
    out.seams.extend(wall_seams.iter().map(Seam::key));
    out.seams.extend(
        wall_seams
            .iter()
            .zip(wall_bottoms.iter())
            .filter_map(|(seam, bottom)| bottom.map(|bottom| abutment_bottom(seam, bottom).key())),
    );
    portals.inputs(&mut out.triangles, &mut out.seams);
    sites.inputs(&mut out.sites);
    base_revisions(terrain, window, &mut out.base_revisions);
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
