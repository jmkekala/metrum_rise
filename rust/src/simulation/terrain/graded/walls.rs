// SPDX-License-Identifier: GPL-2.0-only

//! Conflict walls (`TERRAIN-07-C`, the user's `T2` (c) decision): where two seams stand closer
//! than 1:2 allows, the overlap rule takes the highest lower bound, so the graded ground just
//! outside the lower road stands above its seam (`g > h`). A retaining wall closes that step:
//! a vertical face along the seam from the seam's height up to the graded ground, built here
//! from the seams and the tile's finished lattice samples. The roadbed takes no terrain height.
//!
//! The terrain renders the lattice's triangles (each cell split along its `(i, j)`-
//! `(i + 1, j + 1)` diagonal) and the mask discards them exactly inside the road, so the cut
//! terrain edge over the seam is that surface along the seam line. A wall's vertices are the
//! seam's crossings with the lattice lines and cell diagonals, where that surface bends, and
//! its top is the higher of the graded ground `g` and that surface there: linear between
//! vertices like the surface, so the face covers the cut edge wherever its ends do. Pads are
//! not masked (`T11`): the terrain is continuous over a pad seam and has no step to close.

use super::grade::{CAP_M, TIE_IN_SLOPE};
use super::prune::box_distance;
use super::seams::Seam;
use super::tile::{TILE_M, TileKey};

// Slack on the setter tests' bounds, metres: rounding only keeps more setters, each evaluated
// exactly.
const SETTER_SLACK_M: f64 = 1.0e-6;

/// One wall face along a road seam: from `a` to `b` (tile-local XZ, metres from the tile
/// origin, the road on the left of `a -> b`), standing from `bottom` (the seam's height at each
/// end) to `top` (world heights, metres). Faces along a seam share their ends bitwise.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WallFace {
    /// Start, tile-local XZ.
    pub a: [f32; 2],
    /// End, tile-local XZ.
    pub b: [f32; 2],
    /// Seam heights at `a` and `b`.
    pub bottom: [f32; 2],
    /// Wall top at `a` and `b`.
    pub top: [f32; 2],
}

// One vertex along a seam: where, the seam's height, the wall top and whether the ground just
// outside stands above the seam there.
#[derive(Clone, Copy, Debug)]
struct WallVertex {
    x: f64,
    z: f64,
    bottom: f64,
    top: f64,
    walled: bool,
}

/// Reused buffers of [`build_walls`].
#[derive(Default)]
pub(super) struct WallScratch {
    setters: Vec<u32>,
    pairs: Vec<u32>,
    params: Vec<f64>,
    vertices: Vec<WallVertex>,
}

/// Builds the walls of `tile` into `out`: along every road seam (`seams[..road_count]`, key
/// order) its part in the tile, with vertices at its crossings with the lattice `xs` x `zs`
/// and its cell diagonals. A vertex is walled where the highest lower bound `L` over every
/// other seam within the cap stands above the seam (`g = L > h`; the seam's own cone gives
/// `h`), and its top is `max(L, the lattice surface)`; elsewhere `max(h, the surface)`. A face
/// joins two consecutive vertices of which one is walled. `heights(i, j)` is the finished
/// sample at lattice point `(i, j)` (the base plus the packed delta). Pruned, a seam is only
/// evaluated against the seams whose cone can rise above it at all (`h'_hi - k D > h_lo`,
/// `D` between their boxes), which leaves every walled `L` as it is: the others' lower bounds
/// stay below the seam. `O(window seams + road seams in the tile x their setters + crossings
/// of the walled seams x setters)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_walls(
    seams: &[Seam],
    road_count: usize,
    tile: TileKey,
    xs: &[f64],
    zs: &[f64],
    prune: bool,
    heights: impl Fn(usize, usize) -> f64,
    scratch: &mut WallScratch,
    out: &mut Vec<WallFace>,
) {
    out.clear();
    let origin = tile.origin();
    let rect = [origin[0], origin[1], origin[0] + TILE_M, origin[1] + TILE_M];
    let in_tile = |seam: &Seam| {
        let area = seam.box_xz();
        area[0] <= rect[2] && rect[0] <= area[2] && area[1] <= rect[3] && rect[1] <= area[3]
    };
    // The lowest road seam in the tile and the box of them all: no seam whose cone stays below
    // it over that box can wall any of them.
    let (mut low, mut area) = (
        f64::INFINITY,
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
    );
    for seam in seams[..road_count].iter().filter(|seam| in_tile(seam)) {
        let b = seam.box_xz();
        low = low.min(seam.height_range().0);
        area = [
            area[0].min(b[0]),
            area[1].min(b[1]),
            area[2].max(b[2]),
            area[3].max(b[3]),
        ];
    }
    if low == f64::INFINITY {
        return;
    }
    let rises = |setter: &Seam, over: [f64; 4], floor: f64| {
        setter.height_range().1 - TIE_IN_SLOPE * box_distance(setter.box_xz(), over)
            > floor - SETTER_SLACK_M
    };
    scratch.setters.clear();
    scratch.setters.extend(
        (0..seams.len() as u32).filter(|&index| !prune || rises(&seams[index as usize], area, low)),
    );
    for index in 0..road_count {
        let seam = &seams[index];
        if !in_tile(seam) {
            continue;
        }
        scratch.pairs.clear();
        scratch
            .pairs
            .extend(scratch.setters.iter().copied().filter(|&other| {
                other as usize != index
                    && (!prune
                        || rises(&seams[other as usize], seam.box_xz(), seam.height_range().0))
            }));
        if scratch.pairs.is_empty() {
            continue;
        }
        let Some((t0, t1)) = clip(seam, rect) else {
            continue;
        };
        crossings(seam, (t0, t1), xs, zs, &mut scratch.params);
        scratch.vertices.clear();
        for &t in &scratch.params {
            let (x, z, h) = point_at(seam, t);
            let lower = scratch
                .pairs
                .iter()
                .map(|&other| seams[other as usize].at(x, z))
                .filter(|at| at.distance <= CAP_M)
                .map(|at| at.height - TIE_IN_SLOPE * at.distance)
                .fold(f64::NEG_INFINITY, f64::max);
            let surface = surface_at(xs, zs, &heights, x, z);
            let walled = lower > h;
            scratch.vertices.push(WallVertex {
                x,
                z,
                bottom: h,
                top: if walled {
                    lower.max(surface)
                } else {
                    h.max(surface)
                },
                walled,
            });
        }
        for pair in scratch.vertices.windows(2) {
            let [v, w] = [pair[0], pair[1]];
            if !(v.walled || w.walled) {
                continue;
            }
            let local = |v: WallVertex| [(v.x - origin[0]) as f32, (v.z - origin[1]) as f32];
            out.push(WallFace {
                a: local(v),
                b: local(w),
                bottom: [v.bottom as f32, w.bottom as f32],
                top: [v.top as f32, w.top as f32],
            });
        }
    }
}

// The point at parameter `t` of a seam with its height, `a` and `b` themselves at the ends, so
// chained seams share their vertices bitwise.
fn point_at(seam: &Seam, t: f64) -> (f64, f64, f64) {
    if t == 0.0 {
        return (seam.a[0], seam.a[2], seam.a[1]);
    }
    if t == 1.0 {
        return (seam.b[0], seam.b[2], seam.b[1]);
    }
    let along = |axis: usize| seam.a[axis] + (seam.b[axis] - seam.a[axis]) * t;
    (along(0), along(2), along(1))
}

// The parameter range of the seam's part within the closed rectangle, if any.
fn clip(seam: &Seam, rect: [f64; 4]) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (axis, lo, hi) in [(0, rect[0], rect[2]), (2, rect[1], rect[3])] {
        let (a, d) = (seam.a[axis], seam.b[axis] - seam.a[axis]);
        if d == 0.0 {
            if a < lo || a > hi {
                return None;
            }
            continue;
        }
        let (ta, tb) = ((lo - a) / d, (hi - a) / d);
        t0 = t0.max(ta.min(tb));
        t1 = t1.min(ta.max(tb));
    }
    (t0 <= t1).then_some((t0, t1))
}

// The parameters in `[t0, t1]` where the seam crosses a lattice line or a cell diagonal, with
// the ends, sorted and unique.
fn crossings(seam: &Seam, (t0, t1): (f64, f64), xs: &[f64], zs: &[f64], out: &mut Vec<f64>) {
    out.clear();
    out.push(t0);
    out.push(t1);
    for (axis, lines) in [(0, xs), (2, zs)] {
        let (a, d) = (seam.a[axis], seam.b[axis] - seam.a[axis]);
        if d == 0.0 {
            continue;
        }
        let (lo, hi) = {
            let (p, q) = (a + d * t0, a + d * t1);
            (p.min(q), p.max(q))
        };
        // The lines strictly between the piece's ends (none where they coincide).
        let first = lines.partition_point(|&v| v <= lo);
        let last = lines.partition_point(|&v| v < hi).max(first);
        for &line in &lines[first..last] {
            out.push((line - a) / d);
        }
    }
    out.retain(|&t| t >= t0 && t <= t1);
    out.sort_unstable_by(f64::total_cmp);
    out.dedup();
    // Each piece between lattice lines lies in one cell: add its diagonal crossing.
    let pieces = out.len().saturating_sub(1);
    for k in 0..pieces {
        let (ta, tb) = (out[k], out[k + 1]);
        let (mx, mz, _) = point_at(seam, 0.5 * (ta + tb));
        let (i, j) = (cell(xs, mx), cell(zs, mz));
        let diagonal = |t: f64| {
            let (x, z, _) = point_at(seam, t);
            (x - xs[i]) / (xs[i + 1] - xs[i]) - (z - zs[j]) / (zs[j + 1] - zs[j])
        };
        let (fa, fb) = (diagonal(ta), diagonal(tb));
        if (fa < 0.0 && fb > 0.0) || (fa > 0.0 && fb < 0.0) {
            out.push(ta + (tb - ta) * (fa / (fa - fb)));
        }
    }
    out.sort_unstable_by(f64::total_cmp);
    out.dedup();
}

// The lattice cell index holding coordinate `value` on one axis: the cell whose low line is the
// last at or below it, within the lattice.
fn cell(lines: &[f64], value: f64) -> usize {
    lines
        .partition_point(|&v| v <= value)
        .saturating_sub(1)
        .min(lines.len() - 2)
}

/// The lattice surface at a world point of the lattice: the samples of the cell holding it on
/// the triangle under it, each cell split along its `(i, j)`-`(i + 1, j + 1)` diagonal, as the
/// queries and the `TERRAIN-07-D` mesh read it.
pub(super) fn surface_at(
    xs: &[f64],
    zs: &[f64],
    heights: &impl Fn(usize, usize) -> f64,
    x: f64,
    z: f64,
) -> f64 {
    let (i, j) = (cell(xs, x), cell(zs, z));
    let fx = (x - xs[i]) / (xs[i + 1] - xs[i]);
    let fz = (z - zs[j]) / (zs[j + 1] - zs[j]);
    if fx >= fz {
        (1.0 - fx) * heights(i, j) + (fx - fz) * heights(i + 1, j) + fz * heights(i + 1, j + 1)
    } else {
        (1.0 - fz) * heights(i, j) + (fz - fx) * heights(i, j + 1) + fx * heights(i + 1, j + 1)
    }
}
