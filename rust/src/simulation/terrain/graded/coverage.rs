// SPDX-License-Identifier: GPL-2.0-only

//! Road coverage (`TERRAIN-07`): which lattice points a grounded road top triangle covers, by
//! the roadbed's own point-in-triangle predicate, so it equals `height_at(p).is_some()` wherever
//! no structure span lies over the point. No loops are joined and no side is inferred, so an
//! open chain flips no region.

use crate::simulation::network::roadbed::{
    CoverageTriangle, GroundedPieceScratch, PieceId, RoadVec3, RoadbedSystem,
};

// Bounds padding. The predicate is exact (it holds no point outside a triangle's closed XZ box),
// so the closed rectangle would visit every triangle that can hold one of its points; the pad,
// wider than the piece grids' padding, is the margin the dirty-tile selection's
// `PIECE_PAD_M` assumes for the visited triangles (the coverage inputs).
const PAD_M: f64 = 0.01;

/// The grounded top triangles whose closed boxes, grown by the pad, meet the lattice `xs` x
/// `zs`: into `out` in the roadbed's visit order, and their bits, sorted and unique, into
/// `bits`, the coverage inputs. One visit serves a tile's inputs and its raster
/// ([`rasterize`]), so a stage that compares a tile's inputs before baking it visits the
/// triangles once (`TERRAIN-07-E` round 3). `O(T log T)` for the `T` triangles met.
pub(super) fn covering_triangles(
    roadbed: &RoadbedSystem,
    xs: &[f64],
    zs: &[f64],
    pieces: &mut Vec<PieceId>,
    out: &mut Vec<[RoadVec3; 3]>,
    bits: &mut Vec<[u64; 9]>,
) {
    out.clear();
    bits.clear();
    let Some(bounds) = lattice_bounds(xs, zs) else {
        return;
    };
    roadbed.for_each_grounded_top_triangle_in_bounds(bounds, pieces, |triangle| {
        bits.push(triangle_bits(&triangle));
        out.push(triangle);
    });
    bits.sort_unstable();
    bits.dedup();
}

/// Rasterizes `triangles` ([`covering_triangles`] of the lattice) over the lattice `xs` x `zs`
/// (sorted, row-major by `z`): `road[k]` is the highest covering triangle's height at lattice
/// point `k`, `-inf` where none covers it. Heights are `+ 0.0` so a negative zero never decides
/// a maximum. `O(K)` for the `K` lattice points in the triangles' boxes.
pub(super) fn rasterize(triangles: &[[RoadVec3; 3]], xs: &[f64], zs: &[f64], road: &mut Vec<f32>) {
    road.clear();
    road.resize(xs.len() * zs.len(), f32::NEG_INFINITY);
    for triangle in triangles {
        raise_covered(triangle, xs, zs, road);
    }
}

// The rectangle whose triangles the coverage of a lattice visits: its closed box grown by the
// pad.
fn lattice_bounds(xs: &[f64], zs: &[f64]) -> Option<[f64; 4]> {
    let (Some(&x_first), Some(&x_last), Some(&z_first), Some(&z_last)) =
        (xs.first(), xs.last(), zs.first(), zs.last())
    else {
        return None;
    };
    Some([
        x_first - PAD_M,
        z_first - PAD_M,
        x_last + PAD_M,
        z_last + PAD_M,
    ])
}

/// Raises each lattice point of `xs` x `zs` (row-major by `z`) that `triangle` covers by the
/// roadbed's predicate to its height there: `out[k]` the highest so far. Heights are `+ 0.0`
/// so a negative zero never decides a maximum. The one raster of road coverage and the portal
/// opening's decks (`TERRAIN-07-C`). Only the lattice inside the triangle's closed box can be
/// held; a triangle whose box holds no lattice point (most of the small ones) is not prepared,
/// and one without area covers nothing. The triangle is prepared once ([`CoverageTriangle`]) and
/// each row's scan ends with its held run ([`CoverageTriangle::for_each_held_in_row`]). `O(K)`
/// for the `K` lattice points in its box.
pub(super) fn raise_covered(triangle: &[RoadVec3; 3], xs: &[f64], zs: &[f64], out: &mut [f32]) {
    let width = xs.len();
    let [min_x, min_z, max_x, max_z] = triangle_box(triangle);
    let (i0, i1) = (
        xs.partition_point(|&x| x < min_x),
        xs.partition_point(|&x| x <= max_x),
    );
    let (j0, j1) = (
        zs.partition_point(|&z| z < min_z),
        zs.partition_point(|&z| z <= max_z),
    );
    if i0 == i1 || j0 == j1 {
        return;
    }
    let Some(prepared) = CoverageTriangle::new(triangle) else {
        return;
    };
    for j in j0..j1 {
        let row = &mut out[j * width + i0..j * width + i1];
        prepared.for_each_held_in_row(&xs[i0..i1], zs[j], |i, height| {
            let cell = &mut row[i];
            *cell = cell.max(height + 0.0);
        });
    }
}

/// The coverage of one world point, by the same predicate and triangles as [`rasterize`]: the
/// highest covering grounded top triangle's height, `None` uncovered.
pub(super) fn covered_height_at(
    roadbed: &RoadbedSystem,
    x: f64,
    z: f64,
    pieces: &mut Vec<PieceId>,
) -> Option<f32> {
    let mut best = f32::NEG_INFINITY;
    let bounds = [x - PAD_M, z - PAD_M, x + PAD_M, z + PAD_M];
    roadbed.for_each_grounded_top_triangle_in_bounds(bounds, pieces, |triangle| {
        if let Some(height) = RoadbedSystem::top_triangle_height_at(&triangle, x, z) {
            best = best.max(height + 0.0);
        }
    });
    (best > f32::NEG_INFINITY).then_some(best)
}

/// Gathers once the grounded coverage triangles meeting `bounds`, in the same piece, grid-cell
/// and triangle order as the per-point query. The reused buffers allocate nothing once warm.
pub(super) fn gather(
    roadbed: &RoadbedSystem,
    bounds: [f64; 4],
    pieces: &mut GroundedPieceScratch,
    triangles: &mut Vec<[RoadVec3; 3]>,
) {
    triangles.clear();
    roadbed.for_each_grounded_top_triangle_in_bounds_indexed(
        [
            bounds[0] - PAD_M,
            bounds[1] - PAD_M,
            bounds[2] + PAD_M,
            bounds[3] + PAD_M,
        ],
        pieces,
        |triangle| triangles.push(triangle),
    );
}

/// [`covered_height_at`] over triangles gathered once for a window.
pub(super) fn covered_height_in(triangles: &[[RoadVec3; 3]], x: f64, z: f64) -> Option<f32> {
    let mut best = f32::NEG_INFINITY;
    for triangle in triangles {
        if let Some(height) = RoadbedSystem::top_triangle_height_at(triangle, x, z) {
            best = best.max(height + 0.0);
        }
    }
    (best > f32::NEG_INFINITY).then_some(best)
}

/// Marks each cell of the grid with sides `xs` x `zs` (sorted; cell `(i, j)` the closed rectangle
/// `[xs[i], xs[i + 1]] x [zs[j], zs[j + 1]]`, `out` row-major by `z`) that a grounded top
/// triangle meets: some point of the closed cell is held by the roadbed's coverage predicate
/// (`docs/terrain.md` T32 (a)). A triangle without area holds nothing, as in the predicate.
/// `O(T + C)` for the `T` triangles met and the `C` cells in their boxes.
pub(super) fn mark_covered_cells(
    roadbed: &RoadbedSystem,
    xs: &[f64],
    zs: &[f64],
    pieces: &mut Vec<PieceId>,
    out: &mut [bool],
) {
    let (Some(&x_first), Some(&x_last), Some(&z_first), Some(&z_last)) =
        (xs.first(), xs.last(), zs.first(), zs.last())
    else {
        return;
    };
    let bounds = [
        x_first - PAD_M,
        z_first - PAD_M,
        x_last + PAD_M,
        z_last + PAD_M,
    ];
    roadbed.for_each_grounded_top_triangle_in_bounds(bounds, pieces, |[a, b, c]| {
        let ring = [[a.x, a.z], [b.x, b.z], [c.x, c.z]];
        if orient(ring[0], ring[1], ring[2]) != 0.0 {
            mark_met_cells(xs, zs, &ring, out);
        }
    });
}

/// Marks each cell of the grid (as [`mark_covered_cells`]) that the closed polygon `ring` (simple,
/// either orientation) meets, exactly: a corner of the ring in the cell, a side crossing it, or
/// the cell inside the ring. `O(C x corners)` for the `C` cells in the ring's box.
pub(super) fn mark_met_cells(xs: &[f64], zs: &[f64], ring: &[[f64; 2]], out: &mut [bool]) {
    let (columns, rows) = (xs.len().saturating_sub(1), zs.len().saturating_sub(1));
    if ring.is_empty() || columns == 0 || rows == 0 {
        return;
    }
    let [min_x, min_z, max_x, max_z] = ring.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[a, b, c, d], &[x, z]| [a.min(x), b.min(z), c.max(x), d.max(z)],
    );
    // The cells whose closed sides overlap the ring's closed box.
    let (i0, i1) = (
        xs[1..].partition_point(|&x| x < min_x),
        xs[..columns].partition_point(|&x| x <= max_x),
    );
    let (j0, j1) = (
        zs[1..].partition_point(|&z| z < min_z),
        zs[..rows].partition_point(|&z| z <= max_z),
    );
    for j in j0..j1 {
        for i in i0..i1 {
            let k = j * columns + i;
            if !out[k] && ring_meets_rect(ring, [xs[i], zs[j], xs[i + 1], zs[j + 1]]) {
                out[k] = true;
            }
        }
    }
}

// Whether the closed polygon `ring` meets the closed rectangle `[min_x, min_z, max_x, max_z]`.
// If no corner of the ring lies in it and no side meets it, the ring's boundary misses the
// rectangle, which then lies wholly inside the ring or outside it, so one of its corners (on no
// side) decides. Exact: comparisons and exact orientations.
fn ring_meets_rect(ring: &[[f64; 2]], rect: [f64; 4]) -> bool {
    let [min_x, min_z, max_x, max_z] = rect;
    if ring
        .iter()
        .any(|&[x, z]| x >= min_x && x <= max_x && z >= min_z && z <= max_z)
    {
        return true;
    }
    let n = ring.len();
    if (0..n).any(|i| segment_meets_rect(ring[i], ring[(i + 1) % n], rect)) {
        return true;
    }
    // The crossing number of the ray from the corner towards `+x`, half-open in `z`: an upward
    // side crosses right of the point exactly when the point is left of it. The corner lies on
    // no side, so no orientation is zero where a side crosses its row.
    let corner = [min_x, min_z];
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a[1] > corner[1]) != (b[1] > corner[1]) && (b[1] > a[1]) == (orient(a, b, corner) > 0.0)
        {
            inside = !inside;
        }
    }
    inside
}

// Whether the closed segment `a b` meets the closed rectangle: separated by neither axis (the
// boxes overlap) nor the segment's own normal (the four corners strictly on one side), the
// separating axes of two convex sets.
fn segment_meets_rect(a: [f64; 2], b: [f64; 2], [min_x, min_z, max_x, max_z]: [f64; 4]) -> bool {
    if a[0].max(b[0]) < min_x
        || a[0].min(b[0]) > max_x
        || a[1].max(b[1]) < min_z
        || a[1].min(b[1]) > max_z
    {
        return false;
    }
    let sides = [
        [min_x, min_z],
        [max_x, min_z],
        [max_x, max_z],
        [min_x, max_z],
    ]
    .map(|corner| orient(a, b, corner));
    !(sides.iter().all(|&side| side > 0.0) || sides.iter().all(|&side| side < 0.0))
}

// The exact orientation of `p` against `a b` (`robust::orient2d`): positive to the left.
fn orient(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    let coord = |[x, y]: [f64; 2]| robust::Coord { x, y };
    robust::orient2d(coord(a), coord(b), coord(p))
}

/// A triangle's XZ bounds `[min_x, min_z, max_x, max_z]`.
pub(super) fn triangle_box(triangle: &[RoadVec3; 3]) -> [f64; 4] {
    let [a, b, c] = triangle;
    [
        a.x.min(b.x).min(c.x),
        a.z.min(b.z).min(c.z),
        a.x.max(b.x).max(c.x),
        a.z.max(b.z).max(c.z),
    ]
}

/// A triangle's vertex bits, `[ax, ay, az, bx, by, bz, cx, cy, cz]`.
pub(super) fn triangle_bits(triangle: &[RoadVec3; 3]) -> [u64; 9] {
    let [a, b, c] = triangle;
    [a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z].map(f64::to_bits)
}
