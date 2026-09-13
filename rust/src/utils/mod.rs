// SPDX-License-Identifier: GPL-2.0-only

//! Planar geometry shared by simulation and rendering.

use godot::prelude::Vector2;

/// Tests ray-crossing parity for a nonempty polygon, without a boundary tolerance halo.
/// Near-horizontal crossings retain the production-area predicate's epsilon exclusion.
#[inline]
pub(crate) fn point_in_polygon(point: Vector2, polygon: &[Vector2]) -> bool {
    let mut inside = false;
    let mut prev = polygon[polygon.len() - 1];
    for &curr in polygon {
        let crosses = (curr.y > point.y) != (prev.y > point.y);
        if crosses {
            let denom = prev.y - curr.y;
            if denom.abs() > f32::EPSILON {
                let intersection_x = (prev.x - curr.x) * (point.y - curr.y) / denom + curr.x;
                if point.x < intersection_x {
                    inside = !inside;
                }
            }
        }
        prev = curr;
    }
    inside
}
