// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared site-polygon checks for manifest validation and interactive authoring.
//! Area is O(V); intersection checks are O(V²), allocation-free and asset-local; the shared
//! area of two polygons goes through the polygon overlay, once per asset check.

use i_overlay::core::{fill_rule::FillRule, overlay_rule::OverlayRule};
use i_overlay::float::single::SingleFloatOverlay;

pub(crate) fn clamp_translation(
    min: [f32; 2],
    max: [f32; 2],
    half_lot: [f32; 2],
    delta: [f32; 2],
) -> [f32; 2] {
    std::array::from_fn(|axis| {
        clamp_interval(
            delta[axis],
            -half_lot[axis] - min[axis],
            half_lot[axis] - max[axis],
        )
    })
}

pub(crate) fn clamp_interval(value: f32, min: f32, max: f32) -> f32 {
    if min <= max {
        value.clamp(min, max)
    } else {
        (min + max) * 0.5
    }
}

/// Whether a plan rectangle `[min, max]` lies inside the lot `±half_lot` grown by `tolerance`:
/// the editor's mesh part fit (`_mesh_part_fits_in_lot`, `CODE-36`). Constant work.
pub(crate) fn fits_lot(plan: [[f32; 2]; 2], half_lot: [f32; 2], tolerance: f32) -> bool {
    (0..2).all(|axis| {
        plan[0][axis] >= -half_lot[axis] - tolerance && plan[1][axis] <= half_lot[axis] + tolerance
    })
}

/// The area two simple polygons share, through the crate's polygon overlay (`i_overlay`, as the
/// parking edit checks use it). Polygons that only touch, along an edge or at a point, share
/// none. O((A + B) log(A + B)), asset-local.
pub(crate) fn intersection_area(a: &[[f32; 2]], b: &[[f32; 2]]) -> f64 {
    if a.len() < 3 || b.len() < 3 {
        return 0.0;
    }
    let precise = |polygon: &[[f32; 2]]| -> Vec<[f64; 2]> {
        polygon.iter().map(|p| p.map(f64::from)).collect()
    };
    let shapes = precise(a).overlay(&precise(b), OverlayRule::Intersect, FillRule::EvenOdd);
    // Even-odd contours: holes wind opposite to their outer contour, so signed areas net out.
    let twice: f64 = shapes
        .iter()
        .flatten()
        .map(|contour| {
            contour
                .iter()
                .zip(contour.iter().cycle().skip(1))
                .map(|(p, q)| p[0] * q[1] - q[0] * p[1])
                .sum::<f64>()
        })
        .sum();
    (twice * 0.5).abs()
}

pub(crate) fn is_valid(vertices: &[[f32; 2]]) -> bool {
    vertices.len() >= 3
        && vertices.iter().flatten().all(|value| value.is_finite())
        && signed_area(vertices).abs() > 0.001
        && !self_intersects(vertices)
}

pub(crate) fn signed_area(vertices: &[[f32; 2]]) -> f32 {
    let mut twice_area = 0.0;
    for i in 0..vertices.len() {
        let [ax, az] = vertices[i];
        let [bx, bz] = vertices[(i + 1) % vertices.len()];
        twice_area += ax * bz - bx * az;
    }
    twice_area * 0.5
}

pub(crate) fn self_intersects(vertices: &[[f32; 2]]) -> bool {
    for a in 0..vertices.len() {
        let b = (a + 1) % vertices.len();
        for c in (a + 1)..vertices.len() {
            let d = (c + 1) % vertices.len();
            if a == c || a == d || b == c || b == d {
                continue;
            }
            if segments_intersect(vertices[a], vertices[b], vertices[c], vertices[d]) {
                return true;
            }
        }
    }
    false
}

fn segments_intersect(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> bool {
    const EPS: f32 = 0.0001;
    let ab_c = orientation(a, b, c);
    let ab_d = orientation(a, b, d);
    let cd_a = orientation(c, d, a);
    let cd_b = orientation(c, d, b);

    if ab_c.abs() <= EPS && point_on_segment(a, b, c) {
        return true;
    }
    if ab_d.abs() <= EPS && point_on_segment(a, b, d) {
        return true;
    }
    if cd_a.abs() <= EPS && point_on_segment(c, d, a) {
        return true;
    }
    if cd_b.abs() <= EPS && point_on_segment(c, d, b) {
        return true;
    }

    (ab_c > EPS) != (ab_d > EPS) && (cd_a > EPS) != (cd_b > EPS)
}

fn orientation(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_on_segment(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> bool {
    const EPS: f32 = 0.0001;
    p[0] >= a[0].min(b[0]) - EPS
        && p[0] <= a[0].max(b[0]) + EPS
        && p[1] >= a[1].min(b[1]) - EPS
        && p[1] <= a[1].max(b[1]) + EPS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_clamps_each_axis_and_centers_oversized_footprints() {
        assert_eq!(
            clamp_translation([-2.0, -3.0], [2.0, 3.0], [10.0, 10.0], [20.0, -20.0]),
            [8.0, -7.0]
        );
        assert_eq!(
            clamp_translation([-12.0, 1.0], [16.0, 2.0], [10.0, 10.0], [50.0, 0.0]),
            [-2.0, 0.0]
        );
    }

    #[test]
    fn winding_concavity_and_invalid_geometry() {
        let mut concave = [
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 1.0],
            [1.0, 1.0],
            [1.0, 4.0],
            [0.0, 4.0],
        ];
        assert!(is_valid(&concave));
        assert_eq!(signed_area(&concave), 7.0);
        concave.reverse();
        assert!(is_valid(&concave));
        assert_eq!(signed_area(&concave), -7.0);
        for invalid in [
            vec![],
            vec![[0.0, 0.0], [1.0, 1.0]],
            vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]],
            vec![[0.0, 0.0], [4.0, 4.0], [0.0, 4.0], [4.0, 0.0]],
            vec![[0.0, 0.0], [1.0, 0.0], [f32::NAN, 1.0]],
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, f32::INFINITY]],
        ] {
            assert!(!is_valid(&invalid));
        }
    }
}
