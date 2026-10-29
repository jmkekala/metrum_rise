// SPDX-License-Identifier: GPL-2.0-only

//! The exact seam mask's per-square extra candidates (`TERRAIN-07-G`, `docs/terrain.md` T48 (a)).
//!
//! Each lattice point names its own nearest seam, and the mask takes the nearest of the four
//! corners' seams. A seam shorter than the spacing whose region of nearest points holds no corner
//! of a square is missing from those four, and the mask then reads a neighbour's clamped end.
//! The bake lists, per square, every seam the mask can need there that no corner names.
//!
//! Why the list is complete. At a point `p` of a square the nearest seam `s*` is no farther than
//! any corner's named seam `n_c`, and the distance to a segment is convex, so its largest value
//! over the square is at a vertex:
//! `|p - s*| <= d = min over named corners c of (max over the square's vertices v of |v - n_c|)`;
//! with every corner named also no farther than the largest `|c - n_c|` plus half the diagonal,
//! since every point lies that near some corner. The mask's exact path runs only where the bilinear
//! `sdf` lies within two spacings of zero, and there `s*` lies within the band `B` of `p`: were it
//! farther, every corner's `sdf` would be at least `B - |p - c|`, and the bilinear weights' mean
//! corner distance is at most half the diagonal, so the bilinear `sdf` would be at least
//! `B - spacing / sqrt 2 > 2 x spacing` (the band is at least three spacings). So every seam within
//! `min(d, B)` of the square is a superset of the nearest seams of the points where the mask reads
//! them, and of those a seam that a quartering of the square proves farther than some named seam
//! everywhere in it is left out. A road seam within `B` of a point `q` of the square is listed by
//! pass 1 in `q`'s bin, at a bin distance at most `|q - s|`, so the bins meeting the square, read
//! up to bin distance `min(d, B)`, hold every such seam whether or not pass 1 pruned: the list does
//! not depend on the pruning. Bore seams and bridge mouth rows are not binned and are few: each is
//! tested directly.

use super::prune::{BinGrid, BinLists};
use super::sample::NO_SEAM;
use super::seams::Seam;
use rayon::prelude::*;

// Rows of squares one task lists (`square_extras`): a fixed partition of the tile, so the lists,
// shared within a chunk only, do not depend on the worker count.
const CHUNK_ROWS: usize = 4;

/// The bake's extra-candidate buffers, reused across tiles.
#[derive(Default)]
pub(super) struct ExtraScratch {
    chunks: Vec<ExtraChunk>,
    // The list starts with their first texels while packing.
    starts: Vec<u32>,
    // The squares' lists back to back, each its length then its seam codes ascending (the
    // window's seams, then the bore seams, then the wall seams, as the texels name them).
    lists: Vec<u32>,
    // Per square with a list, in lattice point order: the point index of its lower corner and
    // the start of its list in `lists`. Squares with equal lists in a row or column of one
    // chunk share one.
    squares: Vec<(u32, u32)>,
}

// One chunk's lists, starts relative to its own `lists`.
#[derive(Default)]
struct ExtraChunk {
    candidates: Vec<u32>,
    // Per square of the previous lattice row, the start of its list in `lists`, else `NO_SEAM`.
    below: Vec<u32>,
    lists: Vec<u32>,
    squares: Vec<(u32, u32)>,
}

/// The seams a tile's texels may name: the window's seams (the road seams first, `road_count`
/// of them, then the pad seams, which no texel names), the bore seams, and the wall seams with
/// whether each is a bridge mouth row (a wall seam with an abutment bottom, which texels name).
pub(super) struct Named<'a> {
    pub(super) seams: &'a [Seam],
    pub(super) road_count: usize,
    pub(super) bores: &'a [Seam],
    pub(super) walls: &'a [Seam],
    pub(super) mouth: &'a [Option<[f64; 2]>],
}

impl Named<'_> {
    fn seam(&self, code: u32) -> &Seam {
        let code = code as usize;
        let (window, bores) = (self.seams.len(), self.bores.len());
        if code < window {
            &self.seams[code]
        } else if code < window + bores {
            &self.bores[code - window]
        } else {
            &self.walls[code - window - bores]
        }
    }
}

/// Lists every square's extra candidates (the module's rule) into `out.lists` and
/// `out.squares`, for the squares the cells address: lower corner index `1..len - 2` on each
/// axis, the tile's own squares (the apron's are never a fragment's). `nearest` holds the final
/// names, `bins` and `first` pass 1's lists, `band` the level's. `O(squares with a named corner x (16 seam distances
/// + their bins' candidates up to distance d + bore and mouth rows))`, allocation-free once warm.
pub(super) fn square_extras(
    named: &Named<'_>,
    (xs, zs): (&[f64], &[f64]),
    nearest: &[u32],
    (bins, first): (&BinGrid, &BinLists),
    band: f64,
    out: &mut ExtraScratch,
) {
    out.lists.clear();
    out.squares.clear();
    let width = xs.len();
    if width < 4 || zs.len() < 4 {
        return;
    }
    let rows = zs.len() - 3;
    let count = rows.div_ceil(CHUNK_ROWS);
    if out.chunks.len() < count {
        out.chunks.resize_with(count, ExtraChunk::default);
    }
    let reach_of = Reach {
        named,
        xs,
        zs,
        nearest,
        bins,
        first,
        band,
    };
    out.chunks[..count]
        .par_iter_mut()
        .enumerate()
        .for_each(|(index, chunk)| {
            let first_row = 1 + index * CHUNK_ROWS;
            reach_of.list_rows(first_row..(first_row + CHUNK_ROWS).min(zs.len() - 2), chunk);
        });
    for chunk in &out.chunks[..count] {
        let offset = out.lists.len() as u32;
        out.lists.extend_from_slice(&chunk.lists);
        out.squares.extend(
            chunk
                .squares
                .iter()
                .map(|&(point, start)| (point, start + offset)),
        );
    }
}

// What every chunk of `square_extras` reads.
struct Reach<'a> {
    named: &'a Named<'a>,
    xs: &'a [f64],
    zs: &'a [f64],
    nearest: &'a [u32],
    bins: &'a BinGrid,
    first: &'a BinLists,
    band: f64,
}

impl Reach<'_> {
    // Lists the squares of lattice rows `rows` into `chunk`. `O(squares x candidates)`.
    fn list_rows(&self, rows: std::ops::Range<usize>, chunk: &mut ExtraChunk) {
        let Self {
            named,
            xs,
            zs,
            nearest,
            bins,
            first,
            band,
        } = *self;
        let ExtraChunk {
            candidates,
            below,
            lists,
            squares,
        } = chunk;
        lists.clear();
        squares.clear();
        let width = xs.len();
        let window = named.seams.len() as u32;
        let unbinned = (window..window + named.bores.len() as u32).chain(
            named
                .mouth
                .iter()
                .enumerate()
                .filter(|(_, bottom)| bottom.is_some())
                .map(|(k, _)| window + named.bores.len() as u32 + k as u32),
        );
        below.clear();
        below.resize(width, NO_SEAM);
        for j in rows {
            let mut left = NO_SEAM;
            for i in 1..width - 2 {
                let corners = [
                    j * width + i,
                    j * width + i + 1,
                    (j + 1) * width + i,
                    (j + 1) * width + i + 1,
                ];
                let names = corners.map(|c| nearest[c]);
                let mut start = NO_SEAM;
                if names.iter().any(|&code| code != NO_SEAM) {
                    let area = [xs[i], zs[j], xs[i + 1], zs[j + 1]];
                    let vertices = [
                        [area[0], area[1]],
                        [area[2], area[1]],
                        [area[0], area[3]],
                        [area[2], area[3]],
                    ];
                    let corner_seams =
                        names.map(|code| (code != NO_SEAM).then(|| named.seam(code)));
                    // Each named corner's seam's distance at each vertex, once for both bounds.
                    let at_vertices = corner_seams
                        .map(|seam| seam.map(|seam| vertices.map(|[x, z]| seam.at(x, z).distance)));
                    // The farthest any point of the square lies from a corner's named seam is at a
                    // vertex (the distance to a segment is convex); the least of those over the
                    // named corners bounds every point's nearest distance, and the band bounds it
                    // where the mask reads it.
                    let reach = (at_vertices.iter().flatten())
                        .map(|distances| distances.iter().copied().fold(0.0, f64::max))
                        .fold(band, f64::min);
                    // Every point lies within half the diagonal of some corner, so with all four
                    // corners named their largest named distance plus half the diagonal bounds it
                    // too.
                    let reach = if let [Some(a), Some(b), Some(c), Some(d)] = at_vertices {
                        let half = 0.5 * (area[2] - area[0]).hypot(area[3] - area[1]);
                        let farthest = [a[0], b[1], c[2], d[3]].into_iter().fold(0.0, f64::max);
                        reach.min(farthest + half)
                    } else {
                        reach
                    };
                    candidates.clear();
                    let range = bins.range(area);
                    for bz in range[2]..=range[3] {
                        for bx in range[0]..=range[1] {
                            let listed = first.of(bins.index(bx, bz));
                            // By bin distance: a seam beyond `reach` of the bin is beyond it of the
                            // square's part in the bin.
                            let near = listed.iter().take_while(|listed| listed.distance <= reach);
                            candidates.extend(
                                near.map(|listed| listed.index)
                                    .filter(|&index| (index as usize) < named.road_count),
                            );
                        }
                    }
                    candidates.extend(unbinned.clone());
                    candidates.sort_unstable();
                    candidates.dedup();
                    let mut tests = NearTests::new(area, &corner_seams);
                    candidates.retain(|&code| {
                        let seam = named.seam(code);
                        !names.contains(&code)
                            && box_distance(area, seam) <= reach
                            && tests.may_be_nearer(seam)
                    });
                    if !candidates.is_empty() {
                        let same = |at: u32| {
                            at != NO_SEAM && {
                                let at = at as usize;
                                lists[at] as usize == candidates.len()
                                    && lists[at + 1..at + 1 + candidates.len()] == candidates[..]
                            }
                        };
                        start = if same(left) {
                            left
                        } else if same(below[i]) {
                            below[i]
                        } else {
                            let at = lists.len() as u32;
                            lists.push(candidates.len() as u32);
                            lists.extend_from_slice(candidates);
                            at
                        };
                        squares.push(((j * width + i) as u32, start));
                    }
                }
                left = start;
                below[i] = start;
            }
        }
    }
}

// Whether a seam may be at least as near as every corner's named seam at some point of a
// square: `g = |p - seam| - |p - n|` changes by at most twice the distance moved, so where `g`
// exceeds a rectangle's diagonal at its centre for some named `n`, the seam is farther than `n`
// everywhere in it; the square is tried, then, unless that holds for it, each of its quarters
// once. Exact: it keeps every seam that is the nearest somewhere, and some that are not. The
// named seams' distances at the five centres are the same for every candidate of the square,
// so each is taken once, when a candidate first needs it (`TERRAIN-07-G` round 9).
struct NearTests<'a> {
    named: &'a [Option<&'a Seam>; 4],
    // The square, then its quarters.
    areas: [[f64; 4]; 5],
    // Per area its centre, diagonal and each corner's named seam's distance there, once taken.
    centres: [Option<NearCentre>; 5],
}

#[derive(Clone, Copy)]
struct NearCentre {
    x: f64,
    z: f64,
    diagonal: f64,
    named: [Option<f64>; 4],
}

impl<'a> NearTests<'a> {
    fn new(area: [f64; 4], named: &'a [Option<&'a Seam>; 4]) -> Self {
        let (x, z) = (0.5 * (area[0] + area[2]), 0.5 * (area[1] + area[3]));
        Self {
            named,
            areas: [
                area,
                [area[0], area[1], x, z],
                [x, area[1], area[2], z],
                [area[0], z, x, area[3]],
                [x, z, area[2], area[3]],
            ],
            centres: [None; 5],
        }
    }

    // Whether `seam` is farther than some named seam everywhere in area `k`.
    fn farther(&mut self, seam: &Seam, k: usize) -> bool {
        let (area, named) = (self.areas[k], self.named);
        let centre = *self.centres[k].get_or_insert_with(|| {
            let (x, z) = (0.5 * (area[0] + area[2]), 0.5 * (area[1] + area[3]));
            NearCentre {
                x,
                z,
                diagonal: (area[2] - area[0]).hypot(area[3] - area[1]),
                named: named.map(|other| other.map(|other| other.at(x, z).distance)),
            }
        });
        let own = seam.at(centre.x, centre.z).distance;
        (centre.named.iter().flatten()).any(|&other| own - other > centre.diagonal)
    }

    // The module's rule for `seam`. `O(5 x named seams)` the first time, `O(5)` after.
    fn may_be_nearer(&mut self, seam: &Seam) -> bool {
        !self.farther(seam, 0) && (1..5).any(|k| !self.farther(seam, k))
    }
}

// Distance in XZ from the rectangle `[x0, z0, x1, z1]` to the seam: 0 where it meets the
// rectangle, else the least of its ends' distances to the rectangle and the rectangle's
// corners' distances to the seam (two disjoint convex sets are nearest at a vertex of one).
fn box_distance(area: [f64; 4], seam: &Seam) -> f64 {
    let (a, b) = ([seam.a[0], seam.a[2]], [seam.b[0], seam.b[2]]);
    let to_box = |p: [f64; 2]| {
        let dx = (area[0] - p[0]).max(p[0] - area[2]).max(0.0);
        let dz = (area[1] - p[1]).max(p[1] - area[3]).max(0.0);
        dx.hypot(dz)
    };
    if crosses(area, a, b) {
        return 0.0;
    }
    let corners = [
        [area[0], area[1]],
        [area[2], area[1]],
        [area[0], area[3]],
        [area[2], area[3]],
    ];
    corners
        .into_iter()
        .map(|[x, z]| seam.at(x, z).distance)
        .fold(to_box(a).min(to_box(b)), f64::min)
}

// Whether the segment `a`-`b` meets the closed rectangle (Liang-Barsky clipping).
fn crosses(area: [f64; 4], a: [f64; 2], b: [f64; 2]) -> bool {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for axis in 0..2 {
        let delta = b[axis] - a[axis];
        let (min, max) = (area[axis], area[axis + 2]);
        if delta == 0.0 {
            if a[axis] < min || a[axis] > max {
                return false;
            }
            continue;
        }
        let (t0, t1) = ((min - a[axis]) / delta, (max - a[axis]) / delta);
        lo = lo.max(t0.min(t1));
        hi = hi.min(t0.max(t1));
        if lo > hi {
            return false;
        }
    }
    true
}

impl ExtraScratch {
    /// Marks every listed seam as named (`row_of[code] = 0`), so the row compaction gives it a
    /// row. `O(listed codes)`.
    pub(super) fn mark_rows(&self, row_of: &mut [u32]) {
        let mut at = 0;
        while at < self.lists.len() {
            let count = self.lists[at] as usize;
            for &code in &self.lists[at + 1..at + 1 + count] {
                row_of[code as usize] = 0;
            }
            at += 1 + count;
        }
    }

    /// Writes the lists in rows (`row_of`, after the compaction) as row texels, each list from
    /// a texel of its own: `(count, row, row, row)`, then four rows a texel, zero padded; and
    /// per square its point and its list's first texel. `O(listed codes + squares)`.
    pub(super) fn pack(
        &mut self,
        row_of: &[u32],
        texels: &mut Vec<[f32; 4]>,
        squares: &mut Vec<(u32, u32)>,
    ) {
        texels.clear();
        squares.clear();
        // Each list's start in `lists` with its first texel, ascending by start.
        self.starts.clear();
        let mut at = 0;
        while at < self.lists.len() {
            let count = self.lists[at] as usize;
            self.starts.extend([at as u32, texels.len() as u32]);
            let values = std::iter::once(count as f32).chain(
                self.lists[at + 1..at + 1 + count]
                    .iter()
                    .map(|&code| row_of[code as usize] as f32),
            );
            let first = texels.len();
            texels.resize(first + (count + 1).div_ceil(4), [0.0; 4]);
            for (k, value) in values.enumerate() {
                texels[first + k / 4][k % 4] = value;
            }
            at += 1 + count;
        }
        let starts = self.starts.as_chunks::<2>().0;
        squares.extend(self.squares.iter().map(|&(point, start)| {
            let k = starts.partition_point(|pair| pair[0] < start);
            (point, starts[k][1])
        }));
    }
}
