// SPDX-License-Identifier: GPL-2.0-only

//! The behind-the-road tile flag (`TERRAIN-07-G`, `docs/terrain.md` T49 (c)): whether the drawn
//! terrain of a tile rises above the road top along a road seam, where the outer-band overlap
//! (T24 (a)) discards it and the behind-the-road variant (T24 (a2)) keeps it behind the road.
//!
//! The cells draw each lattice square as two triangles split along its `(i, j)`-`(i + 1, j + 1)`
//! diagonal, the terrain linear on each. Along a seam the road top is linear too, so the
//! terrain minus the road top is linear on the part of the seam inside a triangle and largest at
//! that part's ends. A tile is flagged where that difference is positive at such an end for any
//! seam the mask may take in the square (its corners' named rows and its extra candidates).

use super::tile::TilePayload;
use rayon::prelude::*;

/// Whether `payload`'s drawn terrain rises above the road top along a candidate seam of one of
/// its squares, with `height(i, j)` the drawn terrain at lattice point `(i, j)` (world metres),
/// `xs` and `zs` the lattice's world coordinates and `origin` the tile's. Squares as
/// [`super::extras::square_extras`] lists them. `O(squares in the band x their candidates)`.
pub(super) fn rises_above_a_seam(
    payload: &TilePayload,
    (xs, zs): (&[f64], &[f64]),
    origin: [f64; 2],
    height: impl Fn(usize, usize) -> f64 + Sync,
) -> bool {
    let width = xs.len();
    if width < 4 || zs.len() < 4 {
        return false;
    }
    // Rows of squares in parallel on the bake's pool; the answer is one bool, whatever the order.
    (1..zs.len() - 2).into_par_iter().any(|j| {
        let row_start = (j * width) as u32;
        let first_listed = payload
            .square_extras
            .partition_point(|&(at, _)| at < row_start);
        let mut squares = payload.square_extras[first_listed..].iter().peekable();
        (1..width - 2).any(|i| {
            let point = (j * width + i) as u32;
            let corners = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)];
            let named = corners.map(|(i, j)| payload.texels[j * width + i][2]);
            let (first, count) = match squares.peek() {
                Some(&&(at, first)) if at == point => {
                    squares.next();
                    (first as usize, payload.extras[first as usize][0] as usize)
                }
                _ => (0, 0),
            };
            // A square without candidates is outside every seam's band.
            if count == 0 && named.iter().all(|&row| row < 0.0) {
                return false;
            }
            let extras = (1..=count).map(|k| payload.extras[first + k / 4][k % 4]);
            let mut rows = named.into_iter().filter(|&row| row >= 0.0).chain(extras);
            let vertices =
                corners.map(|(i, j)| [xs[i] - origin[0], zs[j] - origin[1], height(i, j)]);
            let triangles = [
                [vertices[0], vertices[1], vertices[3]],
                [vertices[0], vertices[3], vertices[2]],
            ];
            rows.any(|row| {
                let row = row as usize;
                let [ax, az, bx, bz] = payload.seams[row].map(f64::from);
                let [ha, hb] = payload.seam_heights[row].map(f64::from);
                triangles
                    .iter()
                    .any(|triangle| rises_in(triangle, [ax, az], [bx, bz], [ha, hb]))
            })
        })
    })
}

// Whether the terrain of `triangle` (tile-local `[x, z, height]` vertices) lies above the road
// top along the part of the seam `a`-`b` (heights `h`) inside the triangle, at either end of that
// part (Cyrus-Beck clipping of the segment by the triangle's three edges).
fn rises_in(triangle: &[[f64; 3]; 3], a: [f64; 2], b: [f64; 2], h: [f64; 2]) -> bool {
    let d = [b[0] - a[0], b[1] - a[1]];
    // Orientation of the triangle, so each edge's inner side is known.
    let area = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let corner = |k: usize| [triangle[k][0], triangle[k][1]];
    let total = area(corner(0), corner(1), corner(2));
    if total == 0.0 {
        return false;
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for k in 0..3 {
        let (p, q) = (corner(k), corner((k + 1) % 3));
        // Inside where `sign(total) * area(p, q, x) >= 0`, linear in the segment's parameter.
        let at = |x: [f64; 2]| total.signum() * area(p, q, x);
        let start = at(a);
        let slope = at(b) - start;
        if slope == 0.0 {
            if start < 0.0 {
                return false;
            }
            continue;
        }
        let t = -start / slope;
        if slope > 0.0 {
            lo = lo.max(t);
        } else {
            hi = hi.min(t);
        }
        if lo > hi {
            return false;
        }
    }
    [lo, hi].into_iter().any(|t| {
        let x = [a[0] + d[0] * t, a[1] + d[1] * t];
        // Barycentric weights of `x` in the triangle.
        let w1 = area(corner(2), corner(0), x) / total;
        let w2 = area(corner(0), corner(1), x) / total;
        let w0 = 1.0 - w1 - w2;
        let terrain = w0 * triangle[0][2] + w1 * triangle[1][2] + w2 * triangle[2][2];
        terrain > h[0] + (h[1] - h[0]) * t
    })
}
