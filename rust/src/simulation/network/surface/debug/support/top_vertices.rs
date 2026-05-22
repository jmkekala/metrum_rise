//! Debug top-vertex matching helpers.

use super::*;

impl RoadSurfaceSystem {
    #[cfg(test)]
    pub(in crate::simulation::network::surface::debug) fn debug_top_vertices(
        piece: &RoadSurfaceVisualNodePiece,
    ) -> Vec<DebugTopVertex> {
        let mut vertices = Vec::new();
        for polygon in &piece.road_surface_polygons {
            vertices.extend(
                polygon
                    .points_world
                    .iter()
                    .copied()
                    .map(|point| DebugTopVertex {
                        material: "road",
                        point,
                    }),
            );
            vertices.extend(polygon.triangles_world.iter().flat_map(|triangle| {
                triangle.iter().copied().map(|point| DebugTopVertex {
                    material: "road",
                    point,
                })
            }));
        }
        for polygon in &piece.curb_surface_polygons {
            vertices.extend(
                polygon
                    .points_world
                    .iter()
                    .copied()
                    .map(|point| DebugTopVertex {
                        material: "curb",
                        point,
                    }),
            );
            vertices.extend(polygon.triangles_world.iter().flat_map(|triangle| {
                triangle.iter().copied().map(|point| DebugTopVertex {
                    material: "curb",
                    point,
                })
            }));
        }
        for polygon in &piece.sidewalk_surface_polygons {
            vertices.extend(
                polygon
                    .points_world
                    .iter()
                    .copied()
                    .map(|point| DebugTopVertex {
                        material: "sidewalk",
                        point,
                    }),
            );
            vertices.extend(polygon.triangles_world.iter().flat_map(|triangle| {
                triangle.iter().copied().map(|point| DebugTopVertex {
                    material: "sidewalk",
                    point,
                })
            }));
        }
        vertices
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface::debug) fn closest_debug_top_vertex(
        point: Vector3,
        top_vertices: &[DebugTopVertex],
    ) -> Option<DebugClosestTopVertex> {
        top_vertices
            .iter()
            .map(|vertex| {
                let xz_error_m =
                    Vector2::new(vertex.point.x - point.x, vertex.point.z - point.z).length();
                DebugClosestTopVertex {
                    material: vertex.material,
                    point: vertex.point,
                    xz_error_m,
                    y_delta_m: point.y - vertex.point.y,
                }
            })
            .min_by(|a, b| {
                a.xz_error_m
                    .total_cmp(&b.xz_error_m)
                    .then(a.y_delta_m.abs().total_cmp(&b.y_delta_m.abs()))
            })
    }

    pub(in crate::simulation::network::surface::debug) fn closest_debug_top_support_for_material(
        point: Vector3,
        material: &'static str,
        piece: &RoadSurfaceVisualNodePiece,
    ) -> Option<DebugClosestTopVertex> {
        let polygons = match material {
            "road" => &piece.road_surface_polygons,
            "curb" => &piece.curb_surface_polygons,
            _ => &piece.sidewalk_surface_polygons,
        };
        let mut best = None;
        for polygon in polygons {
            for &candidate in &polygon.points_world {
                Self::update_closest_debug_top_support(&mut best, point, material, candidate);
            }
            for index in 0..polygon.points_world.len() {
                let start = polygon.points_world[index];
                let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
                Self::update_closest_debug_top_segment_support(
                    &mut best, point, material, start, end,
                );
            }
            for triangle in &polygon.triangles_world {
                for &candidate in triangle {
                    Self::update_closest_debug_top_support(&mut best, point, material, candidate);
                }
                for index in 0..3 {
                    Self::update_closest_debug_top_segment_support(
                        &mut best,
                        point,
                        material,
                        triangle[index],
                        triangle[(index + 1) % 3],
                    );
                }
            }
        }
        best
    }

    pub(in crate::simulation::network::surface::debug) fn update_closest_debug_top_support(
        best: &mut Option<DebugClosestTopVertex>,
        point: Vector3,
        material: &'static str,
        candidate: Vector3,
    ) {
        let xz_error_m = Vector2::new(candidate.x - point.x, candidate.z - point.z).length();
        let candidate = DebugClosestTopVertex {
            material,
            point: candidate,
            xz_error_m,
            y_delta_m: point.y - candidate.y,
        };
        Self::retain_closest_debug_top_support(best, candidate);
    }

    pub(in crate::simulation::network::surface::debug) fn update_closest_debug_top_segment_support(
        best: &mut Option<DebugClosestTopVertex>,
        point: Vector3,
        material: &'static str,
        start: Vector3,
        end: Vector3,
    ) {
        let segment_xz = Vector2::new(end.x - start.x, end.z - start.z);
        let len_squared = segment_xz.length_squared();
        if len_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return;
        }
        let to_point_xz = Vector2::new(point.x - start.x, point.z - start.z);
        let t = (to_point_xz.dot(segment_xz) / len_squared).clamp(0.0, 1.0);
        let candidate = start.lerp(end, t);
        Self::update_closest_debug_top_support(best, point, material, candidate);
    }

    pub(in crate::simulation::network::surface::debug) fn retain_closest_debug_top_support(
        best: &mut Option<DebugClosestTopVertex>,
        candidate: DebugClosestTopVertex,
    ) {
        let replace = best.is_none_or(|current| {
            candidate
                .xz_error_m
                .total_cmp(&current.xz_error_m)
                .then(
                    candidate
                        .y_delta_m
                        .abs()
                        .total_cmp(&current.y_delta_m.abs()),
                )
                .is_lt()
        });
        if replace {
            *best = Some(candidate);
        }
    }

    pub(in crate::simulation::network::surface::debug) fn update_debug_match_stats(
        stats: &mut DebugMatchStats,
        closest: DebugClosestTopVertex,
    ) {
        stats.total += 1;
        stats.max_xz_error_m = stats.max_xz_error_m.max(closest.xz_error_m);
        if closest.xz_error_m <= DEBUG_VERTEX_NEAR_TOLERANCE_M {
            stats.max_y_error_m = stats.max_y_error_m.max(closest.y_delta_m.abs());
        }
        if Self::debug_match_is_problem(closest) {
            stats.problem_count += 1;
        }
    }

    pub(in crate::simulation::network::surface::debug) fn debug_match_is_problem(
        closest: DebugClosestTopVertex,
    ) -> bool {
        closest.xz_error_m > DEBUG_VERTEX_NEAR_TOLERANCE_M
            || (closest.xz_error_m <= DEBUG_VERTEX_NEAR_TOLERANCE_M
                && closest.y_delta_m.abs() > DEBUG_VERTEX_MATCH_TOLERANCE_M)
    }
}
