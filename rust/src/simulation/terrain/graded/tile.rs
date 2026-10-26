// SPDX-License-Identifier: GPL-2.0-only

//! Tiles, level lattices and the tile bake (`TERRAIN-07`): the pointwise function sampled on a
//! level's lattice over one 64 m tile, with the two-pass exact pruning (`prune`), coverage,
//! packing into texels and rows, and the tile's inputs.
//!
//! A bake reads only its tile's window (the lattice grown by the cap or the band), so a tile's
//! bytes never depend on the batch, tile order or worker count.

use super::coverage::rasterize;
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
use std::cell::RefCell;

/// Tile side, metres; tiles are aligned to the world origin.
pub const TILE_M: f64 = 64.0;
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

/// Per-worker bake buffers, reused across tiles.
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
    lifted: Vec<Lifted>,
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

thread_local! {
    // One scratch per thread: pool workers keep theirs between bakes.
    static SCRATCH: RefCell<BakeScratch> = RefCell::default();
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
                SCRATCH.with_borrow_mut(|scratch| {
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
        lifted,
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
    // which reads as no pad, class 0 and no edge.
    if sites.is_empty() {
        pads.clear();
        classes.clear();
        edges_near.clear();
    } else {
        sites.rasterize_pads(xs, zs, prune, pads);
        sites.rasterize_classes(xs, zs, prune, classes);
        sites.nearest_edges(xs, zs, band, prune, edges_near);
    }

    // Sample every lattice point over pass 1's seams, a row's points in one bin together in
    // lanes. A ground sample left below its base is finished by pass 2; every other value is
    // final.
    let count = xs.len() * zs.len();
    out.texels.clear();
    out.texels.reserve(count);
    nearest.clear();
    lifted.clear();
    bin_floor.clear();
    bin_floor.resize(bins.len(), f64::INFINITY);
    let mut graded = false;
    let lanes = if evaluation == Evaluation::Lanes {
        LANES
    } else {
        1
    };
    columns.clear();
    columns.extend(xs.iter().map(|&x| (bins.of(0, x), base.stop(0, x))));
    for (j, &z) in zs.iter().enumerate() {
        let (bin_row, row_stop) = (bins.of(1, z), base.stop(1, z));
        let mut i = 0;
        while i < xs.len() {
            // The run of this row's points in the bin of point `i`, at most `lanes` long.
            let bin_column = columns[i].0;
            let bin = bins.index(bin_column, bin_row);
            let mut run = 1;
            while run < lanes && i + run < xs.len() && columns[i + run].0 == bin_column {
                run += 1;
            }
            let list = first.of(bin);
            stats.listed += (list.len() * run) as u64;
            let row = j * xs.len();
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
                // Nothing within reach or band and uncovered: `evaluate` returns the base itself,
                // so the delta is zero and the distance the band, without reading the base.
                for _ in 0..run {
                    out.texels.push([0.0, band as f32, -1.0, 0.0]);
                    nearest.push(NO_SEAM);
                }
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
                stats.candidates += u64::from(value.evaluated);
                let delta = (value.height - base_m) as f32;
                let (in_band, near_seam) = (value.sdf < band, value.nearest_distance <= spacing);
                if evaluation != Evaluation::Unpruned
                    && roads[lane].is_none()
                    && pad_heights[lane].is_none()
                    && value.height < base_m
                {
                    bin_floor[bin] = bin_floor[bin].min(value.height);
                    lifted.push(Lifted {
                        sample: out.texels.len() as u32,
                        bin: bin as u32,
                        x,
                        z,
                        base: base_m,
                        cones: value.cones,
                        near_seam,
                        in_band,
                    });
                } else {
                    if value.conflict {
                        stats.conflicts += 1;
                        stats.seam_conflicts += u32::from(near_seam);
                    }
                    graded |= delta != 0.0 || in_band || pad_heights[lane].is_some();
                }
                out.texels.push([delta, value.sdf as f32, -1.0, 0.0]);
                nearest.push(value.nearest);
            }
            i += run;
        }
    }
    if !lifted.is_empty() {
        list_second_pass(seams, bins, band, bin_floor, seam_bins, pairs);
        second.build(bins.len(), pairs);
        graded |= finish_second_pass(seams, second, lifted, &mut out.texels, &mut stats);
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

    // The seam row: the named road seams in key order, then the named bore seams.
    let seam_at = |index: usize| match index.checked_sub(seams.len()) {
        Some(bore) => &portals.bores()[bore],
        None => &seams[index],
    };
    row_of.clear();
    row_of.resize(seams.len() + portals.bores().len(), NO_SEAM);
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
    out.inputs.seams.clear();
    out.inputs.seams.extend(seams[..road_count].iter().map(Seam::key));
    out.inputs.seams.extend(wall_seams.iter().map(Seam::key));
    out.inputs.seams.extend(
        wall_seams
            .iter()
            .zip(wall_bottoms.iter())
            .filter_map(|(seam, bottom)| bottom.map(|bottom| abutment_bottom(seam, bottom).key())),
    );
    portals.inputs(&mut out.inputs.triangles, &mut out.inputs.seams);
    sites.inputs(&mut out.inputs.sites);
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
