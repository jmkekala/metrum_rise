// SPDX-License-Identifier: GPL-2.0-only

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

        let constraints = (0..points_world.len())
            .map(|index| [index, (index + 1) % points_world.len()])
            .collect();
        Self::triangulate_constrained_region_xz(
            points_world,
            constraints,
            Self::triangle_has_area_xz,
            |point| Self::polygon_contains_point_xz(points_world, point),
        )
    }

    /// Triangulates an outer boundary and holes using the same CDT as road polygons.
    pub(in crate::simulation::network::surface) fn triangulate_constrained_rings_xz(
        rings: &[impl AsRef<[RoadVec3]>],
    ) -> Option<Vec<[RoadVec3; 3]>> {
        if rings.is_empty() || rings.iter().any(|ring| ring.as_ref().len() < 3) {
            return None;
        }
        let points_world: Vec<_> = rings
            .iter()
            .flat_map(|ring| ring.as_ref().iter().copied())
            .collect();

        let mut constraints = Vec::with_capacity(points_world.len());
        let mut offset = 0;
        for ring in rings {
            let len = ring.as_ref().len();
            constraints.extend((0..len).map(|i| [offset + i, offset + (i + 1) % len]));
            offset += len;
        }
        // Ground must close even sub-centimetre residuals: the road renderer's skinny-triangle
        // rejection is inappropriate for a terrain cutout. CDT constraints make the interior
        // centroid test exact; no boundary-distance halo is used for hole inclusion.
        Self::triangulate_constrained_region_xz(
            &points_world,
            constraints,
            |triangle| Self::road_triangle_double_area_xz_m2(triangle) > 0.0,
            |point| {
                Self::polygon_contains_point_xz_with_boundary(rings[0].as_ref(), point, false)
                    && !rings[1..].iter().any(|ring| {
                        Self::polygon_contains_point_xz_with_boundary(ring.as_ref(), point, false)
                    })
            },
        )
    }

    fn triangulate_constrained_region_xz(
        points_world: &[RoadVec3],
        constraints: Vec<[usize; 2]>,
        accepts: impl Fn([RoadVec3; 3]) -> bool,
        contains: impl Fn(RoadVec2) -> bool,
    ) -> Option<Vec<[RoadVec3; 3]>> {
        let vertices = points_world
            .iter()
            .map(|point| Point2::new(point.x, point.z))
            .collect();
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
            if accepts(triangle) && contains(centroid) {
                triangles.push(triangle);
            }
        }

        (!triangles.is_empty()).then_some(triangles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_infill_preserves_holes_and_subcentimetre_residuals() {
        let ring = |min: f64, max: f64| {
            vec![
                RoadVec3::new(min, 0.0, min),
                RoadVec3::new(max, 0.0, min),
                RoadVec3::new(max, 0.0, max),
                RoadVec3::new(min, 0.0, max),
            ]
        };
        for (rings, expected_area) in [
            (vec![ring(0.0, 4.0), ring(1.0, 3.0)], 12.0),
            (
                vec![vec![
                    RoadVec3::new(0.0, 0.0, 0.0),
                    RoadVec3::new(0.005, 0.0, 0.0),
                    RoadVec3::new(0.005, 0.0, 10.0),
                    RoadVec3::new(0.0, 0.0, 10.0),
                ]],
                0.05,
            ),
        ] {
            let triangles = RoadSurfaceSystem::triangulate_constrained_rings_xz(&rings).unwrap();
            let area: f64 = triangles
                .iter()
                .map(|triangle| RoadSurfaceSystem::road_triangle_double_area_xz_m2(*triangle) * 0.5)
                .sum();
            assert!(
                (area - expected_area).abs() < 1.0e-12,
                "infill area {area} != {expected_area}"
            );
        }
    }
}
