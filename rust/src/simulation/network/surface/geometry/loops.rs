//! Visual loop canonicalization and polygon construction.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn canonicalize_loop_points(
        mut points_world: Vec<RoadVec3>,
    ) -> Vec<RoadVec3> {
        points_world.dedup_by(|a, b| {
            (*a - *b).length_squared() <= f64::from(WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2)
        });
        if points_world.len() >= 2
            && (points_world.first().copied().unwrap() - points_world.last().copied().unwrap())
                .length_squared()
                <= f64::from(WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2)
        {
            points_world.pop();
        }
        points_world
    }

    pub(in crate::simulation::network::surface) fn make_visual_polygon(
        mut points_world: Vec<RoadVec3>,
    ) -> Option<RoadSurfaceVisualPolygon> {
        points_world = Self::canonicalize_loop_points(points_world);
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

    pub(in crate::simulation::network::surface) fn make_boundary_loop_polygon(
        points_world: Vec<RoadVec3>,
    ) -> Option<RoadSurfaceVisualPolygon> {
        Self::make_boundary_loop_polygon_with_winding(points_world, false)
    }

    pub(in crate::simulation::network::surface) fn make_boundary_loop_polygon_preserving_winding(
        points_world: Vec<RoadVec3>,
    ) -> Option<RoadSurfaceVisualPolygon> {
        Self::make_boundary_loop_polygon_with_winding(points_world, true)
    }

    pub(super) fn make_boundary_loop_polygon_with_winding(
        mut points_world: Vec<RoadVec3>,
        preserve_winding: bool,
    ) -> Option<RoadSurfaceVisualPolygon> {
        points_world = Self::canonicalize_loop_points(points_world);
        if points_world.len() < 3 {
            return None;
        }
        if Self::polygon_has_strict_edge_crossing_xz(&points_world) {
            return None;
        }
        let signed_area = Self::signed_polygon_area_xz(&points_world);
        if signed_area.abs() <= NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        if !preserve_winding && signed_area < 0.0 {
            points_world.reverse();
        }
        let (start_index, _) = points_world.iter().enumerate().min_by(|(_, a), (_, b)| {
            a.x.total_cmp(&b.x)
                .then(a.z.total_cmp(&b.z))
                .then(a.y.total_cmp(&b.y))
        })?;
        points_world.rotate_left(start_index);
        Some(RoadSurfaceVisualPolygon {
            points_world,
            triangles_world: Vec::new(),
        })
    }

    pub(in crate::simulation::network::surface) fn make_visual_strip_polygon(
        mut points_world: Vec<RoadVec3>,
    ) -> Option<RoadSurfaceVisualPolygon> {
        points_world.dedup_by(|a, b| {
            (*a - *b).length_squared() <= f64::from(WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2)
        });
        if points_world.len() >= 2
            && (points_world.first().copied()? - points_world.last().copied()?).length_squared()
                <= f64::from(WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2)
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

    pub(in crate::simulation::network::surface) fn make_vertical_quad_polygon(
        points_world: [RoadVec3; 4],
    ) -> Option<RoadSurfaceVisualPolygon> {
        let front = [
            [points_world[0], points_world[1], points_world[2]],
            [points_world[0], points_world[2], points_world[3]],
        ];
        if front.iter().all(|triangle| {
            (triangle[1] - triangle[0])
                .cross(triangle[2] - triangle[0])
                .length()
                <= f64::from(SURFACE_MIN_TRIANGLE_DOUBLE_AREA_M2)
        }) {
            return None;
        }

        let triangles_world = front
            .into_iter()
            .filter(|triangle| {
                (triangle[1] - triangle[0])
                    .cross(triangle[2] - triangle[0])
                    .length()
                    * 0.5
                    > f64::from(NODE_OVERLAY_MIN_AREA_M2)
            })
            .collect::<Vec<_>>();
        if triangles_world.is_empty() {
            return None;
        }
        Some(RoadSurfaceVisualPolygon {
            points_world: points_world.into_iter().collect(),
            triangles_world,
        })
    }
}
