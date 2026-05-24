//! XZ geometric predicates, distances, and ray intersections.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn polygon_contains_point_xz(
        points_world: &[Vector3],
        point: Vector2,
    ) -> bool {
        if points_world.len() < 3 {
            return false;
        }
        let mut inside = false;
        for index in 0..points_world.len() {
            let start = points_world[index];
            let end = points_world[(index + 1) % points_world.len()];
            if Self::point_segment_distance_squared_xz(point, start, end) <= 0.0001 {
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
        point: Vector2,
        start: Vector3,
        end: Vector3,
    ) -> f32 {
        let start_xz = Vector2::new(start.x, start.z);
        let end_xz = Vector2::new(end.x, end.z);
        let segment = end_xz - start_xz;
        let length_squared = segment.length_squared();
        if length_squared <= SAMPLE_EPSILON_M {
            return point.distance_squared_to(start_xz);
        }
        let t = ((point - start_xz).dot(segment) / length_squared).clamp(0.0, 1.0);
        point.distance_squared_to(start_xz + segment * t)
    }

    pub(in crate::simulation::network::surface) fn signed_polygon_area_xz(
        points: &[Vector3],
    ) -> f32 {
        if points.len() < 3 {
            return 0.0;
        }
        let origin = points[0];
        let mut double_area = 0.0;
        for index in 0..points.len() {
            let current = points[index] - origin;
            let next = points[(index + 1) % points.len()] - origin;
            double_area +=
                f64::from(current.x) * f64::from(next.z) - f64::from(next.x) * f64::from(current.z);
        }
        (double_area * 0.5) as f32
    }

    pub(super) fn polygon_has_strict_edge_crossing_xz(points: &[Vector3]) -> bool {
        if points.len() < 4 {
            return false;
        }

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

    pub(super) fn segments_strictly_intersect_xz(
        a: Vector3,
        b: Vector3,
        c: Vector3,
        d: Vector3,
    ) -> bool {
        let ab_c = Self::cross_points_xz(a, b, c);
        let ab_d = Self::cross_points_xz(a, b, d);
        let cd_a = Self::cross_points_xz(c, d, a);
        let cd_b = Self::cross_points_xz(c, d, b);
        ab_c * ab_d < -SAMPLE_EPSILON_M && cd_a * cd_b < -SAMPLE_EPSILON_M
    }

    pub(super) fn cross_points_xz(a: Vector3, b: Vector3, c: Vector3) -> f32 {
        (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn polygon_has_area_xz(points: &[Vector3]) -> bool {
        Self::signed_polygon_area_xz(points).abs() > NODE_OVERLAY_MIN_AREA_M2
    }

    pub(in crate::simulation::network::surface) fn triangle_has_area_xz(
        triangle: [Vector3; 3],
    ) -> bool {
        let projected_cross = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        if projected_cross.abs() <= SURFACE_MIN_TRIANGLE_DOUBLE_AREA_M2 {
            return false;
        }
        let edge_ab = Vector2::new(triangle[1].x - triangle[0].x, triangle[1].z - triangle[0].z);
        let edge_bc = Vector2::new(triangle[2].x - triangle[1].x, triangle[2].z - triangle[1].z);
        let edge_ca = Vector2::new(triangle[0].x - triangle[2].x, triangle[0].z - triangle[2].z);
        let max_edge_m = edge_ab.length().max(edge_bc.length()).max(edge_ca.length());
        projected_cross.abs() / max_edge_m.max(SAMPLE_EPSILON_M) >= SURFACE_MIN_TRIANGLE_ALTITUDE_M
    }

    pub(in crate::simulation::network::surface) fn road_triangle_double_area_xz_m2(
        triangle: [RoadVec3; 3],
    ) -> f64 {
        let [a, b, c] = triangle;
        ((b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)).abs()
    }

    pub(in crate::simulation::network::surface) fn triangle_barycentric_weights_xz(
        triangle: [Vector3; 3],
        point: Vector2,
    ) -> Option<(f32, f32, f32)> {
        let area = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        if area.abs() <= SAMPLE_EPSILON_M {
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
        Some((w0, w1, w2))
    }

    pub(in crate::simulation::network::surface) fn point_is_inside_or_near_triangle_xz(
        triangle: [Vector3; 3],
        point: Vector2,
        margin_m: f32,
    ) -> bool {
        if Self::triangle_barycentric_weights_xz(triangle, point).is_some() {
            return true;
        }
        Self::distance_point_to_triangle_xz(triangle, point) <= margin_m
    }

    pub(in crate::simulation::network::surface) fn closest_point_on_triangle_xz(
        triangle: [Vector3; 3],
        point: Vector2,
    ) -> Vector2 {
        if Self::triangle_barycentric_weights_xz(triangle, point).is_some() {
            return point;
        }

        let triangle_points = [
            Vector2::new(triangle[0].x, triangle[0].z),
            Vector2::new(triangle[1].x, triangle[1].z),
            Vector2::new(triangle[2].x, triangle[2].z),
        ];
        let mut best = triangle_points[0];
        let mut best_distance_squared = point.distance_squared_to(best);

        for &(start, end) in &[
            (triangle_points[0], triangle_points[1]),
            (triangle_points[1], triangle_points[2]),
            (triangle_points[2], triangle_points[0]),
        ] {
            let candidate = Self::closest_point_on_segment_xz(point, start, end);
            let distance_squared = point.distance_squared_to(candidate);
            if distance_squared < best_distance_squared {
                best = candidate;
                best_distance_squared = distance_squared;
            }
        }

        best
    }

    pub(super) fn distance_point_to_triangle_xz(triangle: [Vector3; 3], point: Vector2) -> f32 {
        Self::distance_point_to_segment_xz(
            point,
            Vector2::new(triangle[0].x, triangle[0].z),
            Vector2::new(triangle[1].x, triangle[1].z),
        )
        .min(Self::distance_point_to_segment_xz(
            point,
            Vector2::new(triangle[1].x, triangle[1].z),
            Vector2::new(triangle[2].x, triangle[2].z),
        ))
        .min(Self::distance_point_to_segment_xz(
            point,
            Vector2::new(triangle[2].x, triangle[2].z),
            Vector2::new(triangle[0].x, triangle[0].z),
        ))
    }

    pub(super) fn distance_point_to_segment_xz(
        point: Vector2,
        start: Vector2,
        end: Vector2,
    ) -> f32 {
        point.distance_to(Self::closest_point_on_segment_xz(point, start, end))
    }

    pub(super) fn closest_point_on_segment_xz(
        point: Vector2,
        start: Vector2,
        end: Vector2,
    ) -> Vector2 {
        let segment = end - start;
        let length_squared = segment.length_squared();
        if length_squared <= SAMPLE_EPSILON_M {
            return start;
        }
        let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
        start + segment * t
    }

    pub(in crate::simulation::network::surface) fn ray_triangle_intersection_t(
        triangle: [Vector3; 3],
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<f32> {
        let edge_ab = triangle[1] - triangle[0];
        let edge_ac = triangle[2] - triangle[0];
        let pvec = ray_dir.cross(edge_ac);
        let det = edge_ab.dot(pvec);
        if det.abs() <= SAMPLE_EPSILON_M {
            return None;
        }

        let inv_det = 1.0 / det;
        let tvec = ray_origin - triangle[0];
        let u = tvec.dot(pvec) * inv_det;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }

        let qvec = tvec.cross(edge_ab);
        let v = ray_dir.dot(qvec) * inv_det;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }

        let t = edge_ac.dot(qvec) * inv_det;
        (t >= 0.0).then_some(t)
    }
}
