//! Shared debug support geometry, matching, and literal writers.

use super::*;

impl RoadSurfaceSystem {
    #[cfg(test)]
    pub(super) fn debug_top_vertices(piece: &RoadSurfaceVisualNodePiece) -> Vec<DebugTopVertex> {
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

    pub(super) fn debug_overlay_contours_from_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<NodeOverlayContour> {
        polygons
            .iter()
            .filter_map(|polygon| {
                Self::debug_overlay_contour_from_world_points(&polygon.points_world)
            })
            .collect()
    }

    pub(super) fn debug_overlay_contours_from_top_polygons<'a>(
        polygons: impl IntoIterator<Item = &'a RoadSurfaceVisualPolygon>,
    ) -> Vec<NodeOverlayContour> {
        let mut contours = Vec::new();
        for polygon in polygons {
            if polygon.triangles_world.is_empty() {
                if let Some(contour) =
                    Self::debug_overlay_contour_from_world_points(&polygon.points_world)
                {
                    contours.push(contour);
                }
                continue;
            }
            for triangle in &polygon.triangles_world {
                if let Some(contour) = Self::debug_overlay_contour_from_world_points(triangle) {
                    contours.push(contour);
                }
            }
        }
        contours
    }

    pub(super) fn debug_overlay_contour_from_world_points(
        points: &[Vector3],
    ) -> Option<NodeOverlayContour> {
        let mut contour = Vec::with_capacity(points.len());
        for point in points {
            let point = backend::road_vec2_to_overlay_point(backend::godot_vec3_xz_to_road(*point));
            if contour.last().is_none_or(|last| *last != point) {
                contour.push(point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        (contour.len() >= 3).then_some(contour)
    }

    pub(super) fn debug_overlay_area_m2(shapes: &NodeOverlayShapes) -> f32 {
        shapes.iter().map(Self::overlay_shape_area_m2).sum()
    }

    pub(super) fn append_overlay_shape_samples(dump: &mut String, shapes: &[NodeOverlayShape]) {
        for (index, shape) in shapes.iter().take(DEBUG_MAX_PROBLEM_SAMPLES).enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let area_m2 = Self::overlay_shape_area_m2(shape);
            let (centroid_x, centroid_z) = Self::overlay_shape_centroid_xz(shape);
            let (min_x, min_z, max_x, max_z) = Self::overlay_shape_bounds_xz(shape);
            let _ = write!(
                dump,
                "{{\"area_m2\":{:.6},\"centroid\":[{:.3}, {:.3}],\"bounds\":[[{:.3}, {:.3}], [{:.3}, {:.3}]]}}",
                area_m2, centroid_x, centroid_z, min_x, min_z, max_x, max_z
            );
        }
    }

    pub(super) fn overlay_shape_centroid_xz(shape: &NodeOverlayShape) -> (f64, f64) {
        let mut weighted_x = 0.0;
        let mut weighted_z = 0.0;
        let mut total_weight = 0.0;
        for contour in shape {
            let area = Self::overlay_contour_area_f64(contour);
            let weight = area.abs();
            let (x, z) = Self::overlay_contour_average_xz(contour);
            weighted_x += x * weight;
            weighted_z += z * weight;
            total_weight += weight;
        }
        if total_weight <= f64::EPSILON {
            return Self::overlay_shape_average_xz(shape);
        }
        (weighted_x / total_weight, weighted_z / total_weight)
    }

    pub(super) fn overlay_shape_average_xz(shape: &NodeOverlayShape) -> (f64, f64) {
        let mut x = 0.0;
        let mut z = 0.0;
        let mut count = 0usize;
        for contour in shape {
            for point in contour {
                x += point[0];
                z += point[1];
                count += 1;
            }
        }
        if count == 0 {
            return (0.0, 0.0);
        }
        (x / count as f64, z / count as f64)
    }

    pub(super) fn overlay_contour_average_xz(contour: &[NodeOverlayPoint]) -> (f64, f64) {
        if contour.is_empty() {
            return (0.0, 0.0);
        }
        let mut x = 0.0;
        let mut z = 0.0;
        for point in contour {
            x += point[0];
            z += point[1];
        }
        (x / contour.len() as f64, z / contour.len() as f64)
    }

    pub(super) fn overlay_shape_bounds_xz(shape: &NodeOverlayShape) -> (f64, f64, f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut min_z = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_z = f64::NEG_INFINITY;
        for point in shape.iter().flat_map(|contour| contour.iter()) {
            min_x = min_x.min(point[0]);
            min_z = min_z.min(point[1]);
            max_x = max_x.max(point[0]);
            max_z = max_z.max(point[1]);
        }
        if !min_x.is_finite() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        (min_x, min_z, max_x, max_z)
    }

    #[cfg(test)]
    pub(super) fn closest_debug_top_vertex(
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

    pub(super) fn closest_debug_top_support_for_material(
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

    pub(super) fn update_closest_debug_top_support(
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

    pub(super) fn update_closest_debug_top_segment_support(
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

    pub(super) fn retain_closest_debug_top_support(
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

    pub(super) fn update_debug_match_stats(
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

    pub(super) fn debug_match_is_problem(closest: DebugClosestTopVertex) -> bool {
        closest.xz_error_m > DEBUG_VERTEX_NEAR_TOLERANCE_M
            || (closest.xz_error_m <= DEBUG_VERTEX_NEAR_TOLERANCE_M
                && closest.y_delta_m.abs() > DEBUG_VERTEX_MATCH_TOLERANCE_M)
    }

    pub(super) fn append_match_stats_fields(dump: &mut String, stats: &DebugMatchStats) {
        let _ = write!(
            dump,
            "\"tested_vertices\":{},\"problem_count\":{},\"max_xz_error_m\":{:.4},\"max_y_error_m\":{:.4}",
            stats.total, stats.problem_count, stats.max_xz_error_m, stats.max_y_error_m
        );
    }

    pub(super) fn append_raw_json_samples(dump: &mut String, samples: &[String]) {
        for (index, sample) in samples.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            dump.push_str(sample);
        }
    }

    pub(super) fn append_surface_sample_literal(
        dump: &mut String,
        terrain: &TerrainSystem,
        point: Vector3,
    ) {
        let source_y_m = terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
        let visual_y_m =
            terrain.sample_visual_height_world(point.x, point.z) * config::HEIGHT_SCALE;
        dump.push('{');
        dump.push_str("\"world\":");
        Self::append_vector3_literal(dump, point);
        let _ = write!(
            dump,
            ",\"source_terrain_y_m\":{:.3},\"visual_terrain_y_m\":{:.3}",
            source_y_m, visual_y_m
        );
        dump.push('}');
    }

    pub(super) fn append_vector3_list_literal(dump: &mut String, points: &[Vector3]) {
        dump.push('[');
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_vector3_literal(dump, *point);
        }
        dump.push(']');
    }

    pub(super) fn append_vector3_precise_list_literal(dump: &mut String, points: &[Vector3]) {
        dump.push('[');
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_vector3_precise_literal(dump, *point);
        }
        dump.push(']');
    }

    pub(super) fn append_vector3_pair_precise_literal(
        dump: &mut String,
        start: Vector3,
        end: Vector3,
    ) {
        dump.push('[');
        Self::append_vector3_precise_literal(dump, start);
        dump.push_str(", ");
        Self::append_vector3_precise_literal(dump, end);
        dump.push(']');
    }

    pub(super) fn append_usize_list_literal(dump: &mut String, values: &[usize]) {
        dump.push('[');
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "{value}");
        }
        dump.push(']');
    }

    pub(super) fn append_debug_render_edge_key_literal(dump: &mut String, key: DebugRenderEdgeKey) {
        dump.push('{');
        dump.push_str("\"start\":");
        Self::append_debug_render_vertex_key_literal(dump, key.start);
        dump.push_str(",\"end\":");
        Self::append_debug_render_vertex_key_literal(dump, key.end);
        dump.push('}');
    }

    pub(super) fn append_optional_debug_render_edge_key_literal(
        dump: &mut String,
        key: Option<DebugRenderEdgeKey>,
    ) {
        if let Some(key) = key {
            Self::append_debug_render_edge_key_literal(dump, key);
        } else {
            dump.push_str("null");
        }
    }

    pub(super) fn append_debug_render_vertex_key_literal(
        dump: &mut String,
        key: DebugRenderVertexKey,
    ) {
        let _ = write!(
            dump,
            "{{\"x_key\":{},\"y_mm\":{},\"z_key\":{}}}",
            key.x_key, key.y_mm, key.z_key
        );
    }

    pub(super) fn append_debug_render_xz_edge_key_literal(
        dump: &mut String,
        key: DebugRenderXzEdgeKey,
    ) {
        dump.push('{');
        dump.push_str("\"start\":");
        Self::append_debug_render_xz_vertex_key_literal(dump, key.start);
        dump.push_str(",\"end\":");
        Self::append_debug_render_xz_vertex_key_literal(dump, key.end);
        dump.push('}');
    }

    pub(super) fn append_debug_render_xz_vertex_key_literal(
        dump: &mut String,
        key: DebugRenderXzVertexKey,
    ) {
        let _ = write!(dump, "{{\"x_key\":{},\"z_key\":{}}}", key.x_key, key.z_key);
    }

    pub(super) fn append_chunk_key_list_literal(dump: &mut String, chunks: &[SurfaceChunkKey]) {
        dump.push('[');
        for (index, chunk) in chunks.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "[{}, {}]", chunk.0, chunk.1);
        }
        dump.push(']');
    }

    pub(super) fn append_vector3_literal(dump: &mut String, point: Vector3) {
        let _ = write!(dump, "[{:.3}, {:.3}, {:.3}]", point.x, point.y, point.z);
    }

    pub(super) fn append_vector3_precise_literal(dump: &mut String, point: Vector3) {
        let _ = write!(dump, "[{:.6}, {:.6}, {:.6}]", point.x, point.y, point.z);
    }

    pub(super) fn append_optional_vector3_precise_literal(
        dump: &mut String,
        point: Option<Vector3>,
    ) {
        if let Some(point) = point {
            Self::append_vector3_precise_literal(dump, point);
        } else {
            dump.push_str("null");
        }
    }

    pub(super) fn append_vector2_literal(dump: &mut String, point: Vector2) {
        let _ = write!(dump, "[{:.3}, {:.3}]", point.x, point.y);
    }

    pub(super) fn append_optional_f32_literal(dump: &mut String, value: Option<f32>) {
        if let Some(value) = value {
            let _ = write!(dump, "{value:.3}");
        } else {
            dump.push_str("null");
        }
    }

    pub(super) fn append_optional_f32_precise_literal(dump: &mut String, value: Option<f32>) {
        if let Some(value) = value {
            let _ = write!(dump, "{value:.6}");
        } else {
            dump.push_str("null");
        }
    }
}
