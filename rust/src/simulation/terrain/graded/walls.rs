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
use super::portal::{PORTAL_CLEARANCE_M, PortalWindow};
use super::prune::box_distance;
use super::seams::Seam;
use super::tile::{TILE_M, TileKey};

// Slack on the setter tests' bounds, metres: rounding only keeps more setters, each evaluated
// exactly.
const SETTER_SLACK_M: f64 = 1.0e-6;

/// One wall face along a seam: from `a` to `b` (tile-local XZ, metres from the tile origin, the
/// lower side on the left of `a -> b`: the road for a conflict wall and a portal's headwall, the
/// space under the bridge for an abutment), standing from `bottom` to `top` (world heights,
/// metres): a conflict wall from the seam's height up to the wall top, a structure wall
/// (`TERRAIN-07-C`, T22 4 (a)) between the seam's height and the graded ground, and a portal
/// opening's side wall (T22 5 (a)) along a bore seam, the deck on its left where the ground
/// beside stands above it. Faces along a seam share their ends bitwise.
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
    setters: Vec<Setter>,
    // Per block of the tile: its road seams' box and lowest height, then its setters (CSR).
    blocks: Vec<([f64; 4], f64)>,
    block_starts: Vec<u32>,
    block_setters: Vec<u32>,
    pairs: Vec<u32>,
    params: Vec<f64>,
    vertices: Vec<WallVertex>,
}

// A seam that may set `L` over a road seam in the tile: its index, box and highest point.
#[derive(Clone, Copy, Debug)]
struct Setter {
    index: u32,
    area: [f64; 4],
    high: f64,
}

impl Setter {
    // Whether its cone can rise above `floor` anywhere in `over` (`h_hi - k D > floor`).
    fn rises(&self, over: [f64; 4], floor: f64) -> bool {
        self.high - TIE_IN_SLOPE * box_distance(self.area, over) > floor - SETTER_SLACK_M
    }
}

// Blocks per tile side for the setter lists, 16 m each.
const BLOCKS: usize = 4;

const EMPTY: [f64; 4] = [
    f64::INFINITY,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NEG_INFINITY,
];

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// Builds the walls of `tile` into `out`: along every road seam (`seams[..road_count]`, key
/// order) its part in the tile, with vertices at its crossings with the lattice `xs` x `zs`
/// and its cell diagonals. A vertex is walled where the highest lower bound `L` over every
/// other seam within the cap stands above the seam (`g = L > h`; the seam's own cone gives
/// `h`), and its top is `max(L, the lattice surface)`; elsewhere `max(h, the surface)`. A face
/// joins two consecutive vertices of which one is walled. `heights(i, j)` is the finished
/// sample at lattice point `(i, j)` (the base plus the packed delta). Pruned, a seam is only
/// evaluated against the seams whose cone can rise above it at all (`h'_hi - k D > h_lo`,
/// `D` between their boxes), found through the tile's road seams and then 16 m blocks of them,
/// which leaves every walled `L` as it is: the others' lower bounds stay below the seam.
/// `O(window seams x (1 + blocks) + road seams in the tile x their block's setters + crossings
/// of the seams with setters x setters)`.
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
    // The block of a road seam: the one holding its box's centre, clamped to the tile.
    let block_of = |seam: &Seam| {
        let b = seam.box_xz();
        let cell = |centre: f64, low: f64| {
            ((((centre - low) / (TILE_M / BLOCKS as f64)).floor()).max(0.0) as usize)
                .min(BLOCKS - 1)
        };
        cell(0.5 * (b[1] + b[3]), rect[1]) * BLOCKS + cell(0.5 * (b[0] + b[2]), rect[0])
    };
    // Each block's road seams' box and lowest height, and the tile's: no seam whose cone stays
    // below them over that box can wall any of them.
    scratch.blocks.clear();
    scratch
        .blocks
        .resize(BLOCKS * BLOCKS, (EMPTY, f64::INFINITY));
    let (mut area, mut low) = (EMPTY, f64::INFINITY);
    for seam in seams[..road_count].iter().filter(|seam| in_tile(seam)) {
        let block = &mut scratch.blocks[block_of(seam)];
        let h_lo = seam.height_range().0;
        *block = (union(block.0, seam.box_xz()), block.1.min(h_lo));
        (area, low) = (union(area, seam.box_xz()), low.min(h_lo));
    }
    if low == f64::INFINITY {
        return;
    }
    let setter = |index: usize| Setter {
        index: index as u32,
        area: seams[index].box_xz(),
        high: seams[index].height_range().1,
    };
    scratch.setters.clear();
    scratch.setters.extend(
        (0..seams.len())
            .map(setter)
            .filter(|setter| !prune || setter.rises(area, low)),
    );
    scratch.block_starts.clear();
    scratch.block_setters.clear();
    for &(block_area, block_low) in &scratch.blocks {
        scratch
            .block_starts
            .push(scratch.block_setters.len() as u32);
        if block_low == f64::INFINITY {
            continue;
        }
        scratch.block_setters.extend(
            (0..scratch.setters.len() as u32)
                .filter(|&k| !prune || scratch.setters[k as usize].rises(block_area, block_low)),
        );
    }
    scratch
        .block_starts
        .push(scratch.block_setters.len() as u32);
    for index in 0..road_count {
        let seam = &seams[index];
        if !in_tile(seam) {
            continue;
        }
        let block = block_of(seam);
        let listed = &scratch.block_setters
            [scratch.block_starts[block] as usize..scratch.block_starts[block + 1] as usize];
        let (own, own_low) = (seam.box_xz(), seam.height_range().0);
        scratch.pairs.clear();
        scratch.pairs.extend(listed.iter().filter_map(|&k| {
            let setter = scratch.setters[k as usize];
            (setter.index as usize != index && (!prune || setter.rises(own, own_low)))
                .then_some(setter.index)
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
            let walled = lower > h;
            scratch.vertices.push(WallVertex {
                x,
                z,
                bottom: h,
                top: lower,
                walled,
            });
        }
        if !scratch.vertices.iter().any(|v| v.walled) {
            continue;
        }
        // The tops, against the lattice surface, only along seams with a wall.
        for v in scratch.vertices.iter_mut() {
            let surface = surface_at(xs, zs, &heights, v.x, v.z);
            v.top = if v.walled {
                v.top.max(surface)
            } else {
                v.bottom.max(surface)
            };
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

/// Builds the structure walls of `tile` (`TERRAIN-07-C`, `docs/terrain.md` T22 4 (a)) onto
/// `out`, after its conflict walls: along every wall seam (`walls`, key order) its part in the
/// tile, with vertices at its crossings with the lattice `xs` x `zs` and its cell diagonals,
/// and where the lattice surface crosses the seam's height between two of them (both are linear
/// there, so the crossing is exact). A face joins two consecutive vertices where the surface
/// differs from the seam's height at either end, between the two: a portal's headwall where the
/// graded ground stands above a tunnel's mouth, an abutment where it stands below a bridge's.
/// The terrain is masked on the road's side of the seam, so the faces close its cut edge
/// against the structure's mouth row. A headwall runs along the seam (the road on its left), an
/// abutment against it (the space under the bridge on its left).
///
/// At a bridge's mouth (`bottoms`, per wall seam the abutment face's bottom edge below its ends,
/// `TERRAIN-07-D`, the abutment band) the bridge span closes the row from the deck down to that
/// edge with its slab and abutment face, so the abutment wall starts there: faces where the
/// surface stands below the edge, from the surface up to it, meeting the face on its bottom
/// edge, and the headwall's faces where it stands above the deck as before; between the two
/// nothing, which the face and the slab close.
///
/// Then the portal opening's (T22 5 (a)): along a portal row's wall seam
/// ([`PortalWindow::opens`]) no face where the surface at both ends stands below the seam's
/// height plus the clearance `H`, since the terrain beyond the row is cut; and along every bore
/// seam (`portals`' bores, key order) a face from the seam's height to the surface wherever a
/// vertex within the portal reach has the surface there below `h + H`, the opening's sides.
/// `heights(i, j)` is the finished sample at lattice point `(i, j)`. `O(wall and bore seams x
/// their crossings x portal rows)`.
pub(super) fn build_structure_walls(
    (walls, bottoms): (&[Seam], &[Option<[f64; 2]>]),
    portals: &PortalWindow,
    tile: TileKey,
    lattice: (&[f64], &[f64]),
    heights: impl Fn(usize, usize) -> f64,
    scratch: &mut WallScratch,
    out: &mut Vec<WallFace>,
) {
    let below_crown = |v: &WallVertex| v.top < v.bottom + PORTAL_CLEARANCE_M;
    for (seam, bottom) in walls.iter().zip(bottoms) {
        if let Some(bottom) = *bottom {
            let all = |_: &WallVertex, _: &WallVertex| true;
            let above = |_: (f64, f64), surface: f64, h: f64| surface > h;
            seam_walls(seam, tile, lattice, &heights, above, all, scratch, out);
            let below = |_: (f64, f64), surface: f64, u: f64| surface < u;
            let edge = abutment_bottom(seam, bottom);
            seam_walls(&edge, tile, lattice, &heights, below, all, scratch, out);
            continue;
        }
        let opens = portals.opens(seam);
        seam_walls(
            seam,
            tile,
            lattice,
            &heights,
            |_, surface, h| surface != h,
            |v, w| !(opens && below_crown(v) && below_crown(w)),
            scratch,
            out,
        );
    }
    for seam in portals.bores() {
        seam_walls(
            seam,
            tile,
            lattice,
            &heights,
            |(x, z), surface, h| {
                surface != h && surface < h + PORTAL_CLEARANCE_M && portals.reaches(x, z)
            },
            |_, _| true,
            scratch,
            out,
        );
    }
}

/// The abutment face's bottom edge under a bridge mouth's wall seam (`bottom`, its heights
/// below the seam's ends): the seam's XZ at those heights.
pub(super) fn abutment_bottom(seam: &Seam, bottom: [f64; 2]) -> Seam {
    Seam {
        a: [seam.a[0], bottom[0], seam.a[2]],
        b: [seam.b[0], bottom[1], seam.b[2]],
        kind: seam.kind,
    }
}

// The structure walls along one seam's part in the tile: a vertex is walled where `walled((x,
// z), surface, h)` says, a face joins two consecutive vertices of which one is walled and that
// `keep` keeps, from the seam's height to the surface, its lower side on its left.
#[allow(clippy::too_many_arguments)]
fn seam_walls(
    seam: &Seam,
    tile: TileKey,
    (xs, zs): (&[f64], &[f64]),
    heights: &impl Fn(usize, usize) -> f64,
    walled: impl Fn((f64, f64), f64, f64) -> bool,
    keep: impl Fn(&WallVertex, &WallVertex) -> bool,
    scratch: &mut WallScratch,
    out: &mut Vec<WallFace>,
) {
    let origin = tile.origin();
    let rect = [origin[0], origin[1], origin[0] + TILE_M, origin[1] + TILE_M];
    let local = |v: WallVertex| [(v.x - origin[0]) as f32, (v.z - origin[1]) as f32];
    // Each vertex's `bottom` is the seam's height and its `top` the lattice surface there.
    let face = |v: WallVertex, w: WallVertex| WallFace {
        a: local(v),
        b: local(w),
        bottom: [v.bottom.min(v.top) as f32, w.bottom.min(w.top) as f32],
        top: [v.bottom.max(v.top) as f32, w.bottom.max(w.top) as f32],
    };
    let Some(range) = clip(seam, rect) else {
        return;
    };
    crossings(seam, range, xs, zs, &mut scratch.params);
    scratch.vertices.clear();
    for (k, &t) in scratch.params.iter().enumerate() {
        let (x, z, h) = point_at(seam, t);
        let surface = surface_at(xs, zs, heights, x, z);
        if let Some(&last) = scratch.vertices.last() {
            let (fa, fb) = (last.top - last.bottom, surface - h);
            if (fa < 0.0 && fb > 0.0) || (fa > 0.0 && fb < 0.0) {
                let previous = scratch.params[k - 1];
                let (cx, cz, ch) = point_at(seam, previous + (t - previous) * (fa / (fa - fb)));
                scratch.vertices.push(WallVertex {
                    x: cx,
                    z: cz,
                    bottom: ch,
                    top: ch,
                    walled: false,
                });
            }
        }
        scratch.vertices.push(WallVertex {
            x,
            z,
            bottom: h,
            top: surface,
            walled: walled((x, z), surface, h),
        });
    }
    for pair in scratch.vertices.windows(2) {
        let [v, w] = [pair[0], pair[1]];
        if !(v.walled || w.walled) || !keep(&v, &w) {
            continue;
        }
        let above = v.top > v.bottom || w.top > w.bottom;
        out.push(if above { face(v, w) } else { face(w, v) });
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
