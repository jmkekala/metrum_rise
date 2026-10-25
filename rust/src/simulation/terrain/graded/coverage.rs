// SPDX-License-Identifier: GPL-2.0-only

//! Road coverage (`TERRAIN-07`): which lattice points a grounded road top triangle covers, by
//! the roadbed's own point-in-triangle predicate, so it equals `height_at(p).is_some()` wherever
//! no structure span lies over the point. No loops are joined and no side is inferred, so an
//! open chain flips no region.

use crate::simulation::network::roadbed::{PieceId, RoadVec3, RoadbedSystem};

// Bounds padding: wider than the roadbed's sampling tolerance and its grid padding, so every
// triangle whose predicate can accept a point of the rectangle is visited.
const PAD_M: f64 = 0.01;

/// Rasterizes the grounded top triangles over the lattice `xs` x `zs` (sorted, row-major by
/// `z`): `road[k]` is the highest covering triangle's height at lattice point `k`, `-inf`
/// where none covers it. Heights are `+ 0.0` so a negative zero never decides a maximum. With
/// `triangles`, the bits of every triangle visited, sorted and unique, are the coverage
/// inputs. `O(T + K)` for the `T` triangles met and the `K` lattice points in their boxes.
pub(super) fn rasterize(
    roadbed: &RoadbedSystem,
    xs: &[f64],
    zs: &[f64],
    pieces: &mut Vec<PieceId>,
    road: &mut Vec<f32>,
    mut triangles: Option<&mut Vec<[u64; 9]>>,
) {
    road.clear();
    road.resize(xs.len() * zs.len(), f32::NEG_INFINITY);
    if let Some(triangles) = triangles.as_deref_mut() {
        triangles.clear();
    }
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
    let width = xs.len();
    roadbed.for_each_grounded_top_triangle_in_bounds(bounds, pieces, |triangle| {
        if let Some(triangles) = triangles.as_deref_mut() {
            triangles.push(triangle_bits(&triangle));
        }
        let [min_x, min_z, max_x, max_z] = triangle_box(&triangle);
        let (i0, i1) = (
            xs.partition_point(|&x| x < min_x - PAD_M),
            xs.partition_point(|&x| x <= max_x + PAD_M),
        );
        let (j0, j1) = (
            zs.partition_point(|&z| z < min_z - PAD_M),
            zs.partition_point(|&z| z <= max_z + PAD_M),
        );
        for j in j0..j1 {
            for i in i0..i1 {
                if let Some(height) = RoadbedSystem::top_triangle_height_at(&triangle, xs[i], zs[j])
                {
                    let cell = &mut road[j * width + i];
                    *cell = cell.max(height + 0.0);
                }
            }
        }
    });
    if let Some(triangles) = triangles {
        triangles.sort_unstable();
        triangles.dedup();
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

fn triangle_box(triangle: &[RoadVec3; 3]) -> [f64; 4] {
    let [a, b, c] = triangle;
    [
        a.x.min(b.x).min(c.x),
        a.z.min(b.z).min(c.z),
        a.x.max(b.x).max(c.x),
        a.z.max(b.z).max(c.z),
    ]
}

fn triangle_bits(triangle: &[RoadVec3; 3]) -> [u64; 9] {
    let [a, b, c] = triangle;
    [a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z].map(f64::to_bits)
}
