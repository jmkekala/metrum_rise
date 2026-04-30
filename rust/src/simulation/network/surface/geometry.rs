//! Low-level polygon, triangle, and section-boundary geometry helpers.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualPolygon,
    SAMPLE_EPSILON_M, SurfaceCdt, WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2,
};
use godot::prelude::{Vector2, Vector3};
use spade::{Point2, Triangulation};

// Reject triangles that are area-positive but too skinny for stable height interpolation.
const SURFACE_MIN_TRIANGLE_DOUBLE_AREA_M2: f32 = 1.0e-8;
const SURFACE_MIN_TRIANGLE_ALTITUDE_M: f32 = 0.01;

impl RoadSurfaceSystem {
    pub(super) fn make_visual_polygon(
        mut points_world: Vec<Vector3>,
    ) -> Option<RoadSurfaceVisualPolygon> {
        points_world
            .dedup_by(|a, b| (*a - *b).length_squared() <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2);
        if points_world.len() >= 2
            && (points_world.first().copied()? - points_world.last().copied()?).length_squared()
                <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
        {
            points_world.pop();
        }
        if Self::polygon_has_strict_edge_crossing_xz(&points_world) {
            return None;
        }
        let signed_area = Self::signed_polygon_area_xz(&points_world);
        if signed_area.abs() <= NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        if signed_area < 0.0 {
            points_world.reverse();
        }
        let Some((start_index, _)) = points_world.iter().enumerate().min_by(|(_, a), (_, b)| {
            a.x.total_cmp(&b.x)
                .then(a.z.total_cmp(&b.z))
                .then(a.y.total_cmp(&b.y))
        }) else {
            return None;
        };
        points_world.rotate_left(start_index);
        let triangles_world = Self::triangulate_constrained_polygon_xz(&points_world)?;
        Some(RoadSurfaceVisualPolygon {
            points_world,
            triangles_world,
        })
    }

    pub(super) fn make_visual_strip_polygon(
        mut points_world: Vec<Vector3>,
    ) -> Option<RoadSurfaceVisualPolygon> {
        points_world
            .dedup_by(|a, b| (*a - *b).length_squared() <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2);
        if points_world.len() >= 2
            && (points_world.first().copied()? - points_world.last().copied()?).length_squared()
                <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
        {
            points_world.pop();
        }
        if points_world.len() < 3 {
            return None;
        }
        if Self::polygon_has_strict_edge_crossing_xz(&points_world) {
            return None;
        }
        let triangles_world = Self::triangulate_fan_polygon_xz(&points_world)?;
        Some(RoadSurfaceVisualPolygon {
            points_world,
            triangles_world,
        })
    }

    fn triangulate_fan_polygon_xz(points_world: &[Vector3]) -> Option<Vec<[Vector3; 3]>> {
        if points_world.len() < 3 {
            return None;
        }
        let anchor = points_world[0];
        let mut triangles = Vec::with_capacity(points_world.len().saturating_sub(2));
        for index in 1..points_world.len() - 1 {
            let triangle = [anchor, points_world[index], points_world[index + 1]];
            if Self::triangle_has_area_xz(triangle) {
                triangles.push(triangle);
            }
        }
        (!triangles.is_empty()).then_some(triangles)
    }

    fn triangulate_constrained_polygon_xz(points_world: &[Vector3]) -> Option<Vec<[Vector3; 3]>> {
        if points_world.len() < 3 {
            return None;
        }
        if points_world.len() == 3 {
            let triangle = [points_world[0], points_world[1], points_world[2]];
            return Self::triangle_has_area_xz(triangle).then_some(vec![triangle]);
        }

        let vertices = points_world
            .iter()
            .map(|point| Point2::new(f64::from(point.x), f64::from(point.z)))
            .collect::<Vec<_>>();
        let constraints = (0..points_world.len())
            .map(|index| [index, (index + 1) % points_world.len()])
            .collect::<Vec<_>>();
        let mut invalid_constraints = 0usize;
        let cdt = SurfaceCdt::try_bulk_load_cdt(vertices, constraints, |_| {
            invalid_constraints += 1;
        })
        .ok()?;
        if invalid_constraints > 0 {
            return None;
        }

        let mut triangles = Vec::new();
        for face in cdt.inner_faces() {
            let [a, b, c] = face.vertices();
            let triangle = [
                points_world[a.fix().index()],
                points_world[b.fix().index()],
                points_world[c.fix().index()],
            ];
            let centroid = Vector2::new(
                (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
                (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
            );
            if Self::triangle_has_area_xz(triangle)
                && Self::polygon_contains_point_xz(points_world, centroid)
            {
                triangles.push(triangle);
            }
        }

        (!triangles.is_empty()).then_some(triangles)
    }

    pub(super) fn polygon_contains_point_xz(points_world: &[Vector3], point: Vector2) -> bool {
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

    fn point_segment_distance_squared_xz(point: Vector2, start: Vector3, end: Vector3) -> f32 {
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

    pub(super) fn signed_polygon_area_xz(points: &[Vector3]) -> f32 {
        if points.len() < 3 {
            return 0.0;
        }
        let mut signed_area = 0.0;
        for index in 0..points.len() {
            let current = points[index];
            let next = points[(index + 1) % points.len()];
            signed_area += current.x * next.z - next.x * current.z;
        }
        signed_area * 0.5
    }

    fn polygon_has_strict_edge_crossing_xz(points: &[Vector3]) -> bool {
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

    fn segments_strictly_intersect_xz(a: Vector3, b: Vector3, c: Vector3, d: Vector3) -> bool {
        let ab_c = Self::cross_points_xz(a, b, c);
        let ab_d = Self::cross_points_xz(a, b, d);
        let cd_a = Self::cross_points_xz(c, d, a);
        let cd_b = Self::cross_points_xz(c, d, b);
        ab_c * ab_d < -SAMPLE_EPSILON_M && cd_a * cd_b < -SAMPLE_EPSILON_M
    }

    fn cross_points_xz(a: Vector3, b: Vector3, c: Vector3) -> f32 {
        (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)
    }

    #[cfg(test)]
    pub(super) fn polygon_has_area_xz(points: &[Vector3]) -> bool {
        Self::signed_polygon_area_xz(points).abs() > NODE_OVERLAY_MIN_AREA_M2
    }

    pub(super) fn triangle_has_area_xz(triangle: [Vector3; 3]) -> bool {
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

    pub(super) fn triangle_barycentric_weights_xz(
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

    pub(super) fn point_is_inside_or_near_triangle_xz(
        triangle: [Vector3; 3],
        point: Vector2,
        margin_m: f32,
    ) -> bool {
        if Self::triangle_barycentric_weights_xz(triangle, point).is_some() {
            return true;
        }
        Self::distance_point_to_triangle_xz(triangle, point) <= margin_m
    }

    pub(super) fn closest_point_on_triangle_xz(triangle: [Vector3; 3], point: Vector2) -> Vector2 {
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

    fn distance_point_to_triangle_xz(triangle: [Vector3; 3], point: Vector2) -> f32 {
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

    fn distance_point_to_segment_xz(point: Vector2, start: Vector2, end: Vector2) -> f32 {
        point.distance_to(Self::closest_point_on_segment_xz(point, start, end))
    }

    fn closest_point_on_segment_xz(point: Vector2, start: Vector2, end: Vector2) -> Vector2 {
        let segment = end - start;
        let length_squared = segment.length_squared();
        if length_squared <= SAMPLE_EPSILON_M {
            return start;
        }
        let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
        start + segment * t
    }

    pub(super) fn ray_triangle_intersection_t(
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

    pub(super) fn section_boundary_world_point(
        &self,
        section: &RoadSurfaceSection,
        lateral_offset_m: f32,
        height_m: f32,
    ) -> Vector3 {
        Self::section_boundary_world_point_static(section, lateral_offset_m, height_m)
    }

    pub(super) fn section_boundary_world_point_static(
        section: &RoadSurfaceSection,
        lateral_offset_m: f32,
        height_m: f32,
    ) -> Vector3 {
        Vector3::new(
            section.center_xz.x + section.lateral_xz.x * lateral_offset_m,
            height_m,
            section.center_xz.y + section.lateral_xz.y * lateral_offset_m,
        )
    }
}
