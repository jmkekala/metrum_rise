// SPDX-License-Identifier: GPL-2.0-only

//! Edit-boundary footprint checks using the existing polygon overlay and site spatial index.

use crate::simulation::agriculture::PolygonFootprint;
use crate::simulation::buildings::allocator::BuildingAllocator;
use godot::prelude::Vector2;
use i_overlay::core::{fill_rule::FillRule, overlay_rule::OverlayRule};
use i_overlay::float::single::SingleFloatOverlay;

pub(super) fn rectangle(
    center: Vector2,
    forward: Vector2,
    width: f32,
    length: f32,
) -> [Vector2; 4] {
    let along = forward.normalized() * (length * 0.5);
    let across = Vector2::new(-forward.y, forward.x) * (width * 0.5);
    [
        center - along - across,
        center + along - across,
        center + along + across,
        center - along + across,
    ]
}

pub(super) fn precise(points: &[Vector2]) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|p| [f64::from(p.x), f64::from(p.y)])
        .collect()
}

// Uncovered area below which coverage holds: abutting paving (a yard against the road surface)
// leaves zero-width float slivers along off-axis seams, while any real gap a wheel could find
// is far larger.
const UNCOVERED_AREA_TOLERANCE_M2: f64 = 0.01;

/// Union coverage through ordered differences, including disconnected paving and holes.
/// O(S vertices + K overlay cost), S support contours, K AABB-overlapping contours. Disjoint
/// contours incur no overlay; no overlapping support means no overlay or contour allocation.
/// Shared by bay, translated sweep and turning-envelope checks at edit/load boundaries.
pub(super) fn covered(points: &[Vector2], surfaces: &[Vec<[f64; 2]>]) -> bool {
    let bounds = bounds(points.iter().map(|p| [f64::from(p.x), f64::from(p.y)]));
    let mut remaining = None;
    for surface in surfaces {
        let other = self::bounds(surface.iter().copied());
        if bounds[0] >= other[2]
            || bounds[2] <= other[0]
            || bounds[1] >= other[3]
            || bounds[3] <= other[1]
        {
            continue;
        }
        let contours = remaining.get_or_insert_with(|| vec![vec![precise(points)]]);
        *contours = contours.overlay(surface, OverlayRule::Difference, FillRule::EvenOdd);
        if contours.is_empty() {
            return true;
        }
    }
    remaining.is_some_and(|shapes| {
        // Even-odd contours: holes wind opposite to their outer contour, so signed areas net out.
        let area: f64 = shapes
            .iter()
            .flatten()
            .map(|contour| {
                contour
                    .iter()
                    .zip(contour.iter().cycle().skip(1))
                    .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
                    .sum::<f64>()
                    * 0.5
            })
            .sum();
        area.abs() < UNCOVERED_AREA_TOLERANCE_M2
    })
}

fn bounds(points: impl Iterator<Item = [f64; 2]>) -> [f64; 4] {
    points.fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

pub(super) fn overlaps(a: &[Vector2], b: &[Vector2]) -> bool {
    PolygonFootprint::new(a).overlaps(&PolygonFootprint::new(b))
}

/// O(local structure vertices + overlapping structures × overlay cost). The existing
/// PolygonFootprint overlap rejects disjoint AABBs before overlay; site cells bound candidates.
pub(super) fn structure_collision(allocator: &BuildingAllocator, points: &[Vector2]) -> bool {
    let footprint = PolygonFootprint::new(points);
    let mut collision = false;
    allocator.site_cells.visit(
        [
            footprint.min.x,
            footprint.min.y,
            footprint.max.x,
            footprint.max.y,
        ],
        |idx| {
            if collision {
                return;
            }
            if let Some(site) = allocator.building_sites.get(idx) {
                collision |= site
                    .structure_world
                    .iter()
                    .any(|structure| footprint.overlaps(&PolygonFootprint::new(structure)));
            }
        },
    );
    collision
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_filter_matches_ordered_difference_for_gaps_holes_and_remote_support() {
        let rect = |x, z, w, l| rectangle(Vector2::new(x, z), Vector2::UP, w, l);
        let tested = rect(0.0, 0.0, 4.0, 4.0);
        let left = precise(&rect(-1.0, 0.0, 2.0, 4.0));
        let right = precise(&rect(1.0, 0.0, 2.0, 4.0));
        let remote = precise(&rect(200.0, 200.0, 4.0, 4.0));
        let gap = precise(&rect(1.5, 0.0, 1.0, 4.0));
        let touching = precise(&rect(4.0, 0.0, 4.0, 4.0));
        // Four strips leave a real interior hole despite full outer-bound coverage.
        let hole: Vec<_> = [
            rect(-1.5, 0.0, 1.0, 4.0),
            rect(1.5, 0.0, 1.0, 4.0),
            rect(0.0, -1.5, 2.0, 1.0),
            rect(0.0, 1.5, 2.0, 1.0),
        ]
        .iter()
        .map(|p| precise(p))
        .collect();
        for (surfaces, expected) in [
            (vec![], false),
            (vec![remote.clone(), touching], false),
            (vec![remote.clone(), left.clone(), right.clone()], true),
            (vec![left.clone(), gap, remote.clone()], false),
            (hole, false),
            (vec![left, remote, right], true),
        ] {
            let mut reference = vec![vec![precise(&tested)]];
            for surface in &surfaces {
                reference = reference.overlay(surface, OverlayRule::Difference, FillRule::EvenOdd);
                if reference.is_empty() {
                    break;
                }
            }
            assert_eq!(reference.is_empty(), expected);
            assert_eq!(covered(&tested, &surfaces), expected);
        }
    }
}
