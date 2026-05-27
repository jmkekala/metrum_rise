//! Local XZ polygon triangulation helpers.

use super::*;

impl RoadSurfaceSystem {
    pub(super) fn triangulate_fan_polygon_xz(
        points_world: &[RoadVec3],
    ) -> Option<Vec<[RoadVec3; 3]>> {
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

    pub(super) fn triangulate_constrained_polygon_xz(
        points_world: &[RoadVec3],
    ) -> Option<Vec<[RoadVec3; 3]>> {
        if points_world.len() < 3 {
            return None;
        }
        if points_world.len() == 3 {
            let triangle = [points_world[0], points_world[1], points_world[2]];
            return Self::triangle_has_area_xz(triangle).then_some(vec![triangle]);
        }

        let vertices = points_world
            .iter()
            .map(|point| Point2::new(point.x, point.z))
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
            let centroid = RoadVec2::new(
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
}
