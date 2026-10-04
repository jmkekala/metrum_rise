// SPDX-License-Identifier: GPL-2.0-only

//! XZ geometric predicates, distances, and ray intersections.

use super::*;

// Short loops (node regions, span quads) keep the allocation-free all-pairs test.
const STRICT_CROSSING_ALL_PAIRS_MAX_POINTS: usize = 64;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn polygon_contains_point_xz(
        points_world: &[RoadVec3],
        point: RoadVec2,
    ) -> bool {
        Self::polygon_contains_point_xz_with_boundary(points_world, point, true)
    }

    /// Allows constrained ground regions to exclude the road polygon query's boundary halo.
    pub(super) fn polygon_contains_point_xz_with_boundary(
        points_world: &[RoadVec3],
        point: RoadVec2,
        include_boundary_halo: bool,
    ) -> bool {
        if points_world.len() < 3 {
            return false;
        }
        let mut inside = false;
        for index in 0..points_world.len() {
            let start = points_world[index];
            let end = points_world[(index + 1) % points_world.len()];
            if include_boundary_halo
                && Self::point_segment_distance_squared_xz(point, start, end) <= 0.0001
            {
                return true;
            }
            let start_z = start.z;
            let end_z = end.z;
            if (start_z > point.y) != (end_z > point.y) {
                let edge_x_at_point_z =
                    (end.x - start.x) * (point.y - start_z) / (end_z - start_z) + start.x;
                if point.x < edge_x_at_point_z {
                    inside = !inside;
                }
            }
        }
        inside
    }

    pub(super) fn point_segment_distance_squared_xz(
        point: RoadVec2,
        start: RoadVec3,
        end: RoadVec3,
    ) -> f64 {
        let start_xz = RoadVec2::new(start.x, start.z);
        let end_xz = RoadVec2::new(end.x, end.z);
        let segment = end_xz - start_xz;
        let length_squared = segment.length_squared();
        if length_squared <= f64::from(SAMPLE_EPSILON_M) {
            return point.distance_squared(start_xz);
        }
        let t = ((point - start_xz).dot(segment) / length_squared).clamp(0.0, 1.0);
        point.distance_squared(start_xz + segment * t)
    }

    pub(in crate::simulation::network::surface) fn signed_polygon_area_xz(
        points: &[RoadVec3],
    ) -> f32 {
        if points.len() < 3 {
            return 0.0;
        }
        let origin = points[0];
        let mut double_area = 0.0;
        for index in 0..points.len() {
            let current = points[index] - origin;
            let next = points[(index + 1) % points.len()] - origin;
            double_area += current.x * next.z - next.x * current.z;
        }
        (double_area * 0.5) as f32
    }

    pub(in crate::simulation::network::surface) fn polygon_has_strict_edge_crossing_xz(points: &[RoadVec3]) -> bool {
        if points.len() < 4 {
            return false;
        }
        if points.len() > STRICT_CROSSING_ALL_PAIRS_MAX_POINTS {
            return Self::polygon_has_strict_edge_crossing_xz_swept(points);
        }
        Self::polygon_has_strict_edge_crossing_xz_all_pairs(points)
    }

    pub(super) fn polygon_has_strict_edge_crossing_xz_all_pairs(points: &[RoadVec3]) -> bool {
        for edge_a in 0..points.len() {
            let edge_a_next = (edge_a + 1) % points.len();
            for edge_b in edge_a + 1..points.len() {
                let edge_b_next = (edge_b + 1) % points.len();
                if edge_a == edge_b
                    || edge_a == edge_b_next
                    || edge_a_next == edge_b
                    || edge_a_next == edge_b_next
                {
                    continue;
                }
                if Self::segments_strictly_intersect_xz(
                    points[edge_a],
                    points[edge_a_next],
                    points[edge_b],
                    points[edge_b_next],
                ) {
                    return true;
                }
            }
        }

        false
    }

    // Same answer as the all-pairs loop for long loops such as a span's earthwork outline, whose
    // point count grows with the span's length (`ROAD-52`). A strict crossing needs each segment's
    // endpoints on opposite sides of the other by a margin far above f64 rounding, so crossing
    // segments share their closed XZ bounds; only pairs whose bounds overlap along the loop's
    // longer axis are tested. O(n log n + n·k) for k segments active at once along that axis.
    pub(super) fn polygon_has_strict_edge_crossing_xz_swept(points: &[RoadVec3]) -> bool {
        let count = points.len();
        let (mut min, mut max) = (points[0], points[0]);
        for point in points {
            min = min.min(*point);
            max = max.max(*point);
        }
        let along_x = max.x - min.x >= max.z - min.z;
        // Segment `index` runs from `points[index]` to the next point, with its bounds as
        // (min along, max along, min across, max across).
        let mut segments: Vec<(f64, f64, f64, f64, usize)> = (0..count)
            .map(|index| {
                let (a, b) = (points[index], points[(index + 1) % count]);
                let (a_along, b_along, a_across, b_across) = if along_x {
                    (a.x, b.x, a.z, b.z)
                } else {
                    (a.z, b.z, a.x, b.x)
                };
                (
                    a_along.min(b_along),
                    a_along.max(b_along),
                    a_across.min(b_across),
                    a_across.max(b_across),
                    index,
                )
            })
            .collect();
        segments.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.4.cmp(&b.4)));
        let mut active: Vec<usize> = Vec::new();
        for (position, &(min_along, _, min_across, max_across, edge_b)) in
            segments.iter().enumerate()
        {
            active.retain(|&earlier| segments[earlier].1 >= min_along);
            let edge_b_next = (edge_b + 1) % count;
            for &earlier in &active {
                let (_, _, earlier_min_across, earlier_max_across, edge_a) = segments[earlier];
                let edge_a_next = (edge_a + 1) % count;
                if earlier_max_across < min_across
                    || earlier_min_across > max_across
                    || edge_a == edge_b_next
                    || edge_a_next == edge_b
                {
                    continue;
                }
                if Self::segments_strictly_intersect_xz(
                    points[edge_a],
                    points[edge_a_next],
                    points[edge_b],
                    points[edge_b_next],
                ) {
                    return true;
                }
            }
            active.push(position);
        }
        false
    }

    pub(super) fn segments_strictly_intersect_xz(
        a: RoadVec3,
        b: RoadVec3,
        c: RoadVec3,
        d: RoadVec3,
    ) -> bool {
        let ab_c = Self::cross_points_xz(a, b, c);
        let ab_d = Self::cross_points_xz(a, b, d);
        let cd_a = Self::cross_points_xz(c, d, a);
        let cd_b = Self::cross_points_xz(c, d, b);
        ab_c * ab_d < -SAMPLE_EPSILON_M && cd_a * cd_b < -SAMPLE_EPSILON_M
    }

    pub(super) fn cross_points_xz(a: RoadVec3, b: RoadVec3, c: RoadVec3) -> f32 {
        ((b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)) as f32
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn polygon_has_area_xz(
        points: &[RoadVec3],
    ) -> bool {
        Self::signed_polygon_area_xz(points).abs() > NODE_OVERLAY_MIN_AREA_M2
    }

    pub(in crate::simulation::network) fn top_surface_triangle_is_renderable_xz(
        triangle: [RoadVec3; 3],
    ) -> bool {
        let projected_cross = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        if projected_cross.abs() <= f64::from(SURFACE_MIN_TRIANGLE_DOUBLE_AREA_M2) {
            return false;
        }
        let edge_ab = RoadVec2::new(triangle[1].x - triangle[0].x, triangle[1].z - triangle[0].z);
        let edge_bc = RoadVec2::new(triangle[2].x - triangle[1].x, triangle[2].z - triangle[1].z);
        let edge_ca = RoadVec2::new(triangle[0].x - triangle[2].x, triangle[0].z - triangle[2].z);
        let max_edge_m = edge_ab.length().max(edge_bc.length()).max(edge_ca.length());
        projected_cross.abs() / max_edge_m.max(f64::from(SAMPLE_EPSILON_M))
            >= f64::from(SURFACE_MIN_TRIANGLE_ALTITUDE_M)
    }

    pub(in crate::simulation::network::surface) fn triangle_has_area_xz(
        triangle: [RoadVec3; 3],
    ) -> bool {
        Self::top_surface_triangle_is_renderable_xz(triangle)
    }

    pub(in crate::simulation::network::surface) fn road_triangle_double_area_xz_m2(
        triangle: [RoadVec3; 3],
    ) -> f64 {
        let [a, b, c] = triangle;
        ((b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)).abs()
    }

    pub(in crate::simulation::network::surface) fn triangle_barycentric_weights_xz(
        triangle: [RoadVec3; 3],
        point: RoadVec2,
    ) -> Option<(f32, f32, f32)> {
        let area = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        if area.abs() <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
            return None;
        }

        let w0 = ((triangle[1].x - point.x) * (triangle[2].z - point.y)
            - (triangle[1].z - point.y) * (triangle[2].x - point.x))
            / area;
        let w1 = ((triangle[2].x - point.x) * (triangle[0].z - point.y)
            - (triangle[2].z - point.y) * (triangle[0].x - point.x))
            / area;
        let w2 = 1.0 - w0 - w1;
        let epsilon = 0.001;
        if w0 < -epsilon || w1 < -epsilon || w2 < -epsilon {
            return None;
        }
        Some((w0 as f32, w1 as f32, w2 as f32))
    }
}
