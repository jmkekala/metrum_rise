// SPDX-License-Identifier: GPL-2.0-only

//! Road coverage (`TERRAIN-07`): which lattice points a grounded road top triangle covers, by
//! the roadbed's own point-in-triangle predicate, so it equals `height_at(p).is_some()` wherever
//! no structure span lies over the point. No loops are joined and no side is inferred, so an
//! open chain flips no region.

use crate::simulation::network::roadbed::{CoverageTriangle, PieceId, RoadVec3, RoadbedSystem};

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
