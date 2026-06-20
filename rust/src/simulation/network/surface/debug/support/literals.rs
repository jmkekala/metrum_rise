//! Debug literal formatting helpers.

use super::*;

pub(in crate::simulation::network::surface::debug) trait DebugVec2Literal {
    fn x(self) -> f64;
    fn y(self) -> f64;
}

pub(in crate::simulation::network::surface::debug) trait DebugVec3Literal {
    fn x(self) -> f64;
    fn y(self) -> f64;
    fn z(self) -> f64;
}

impl DebugVec2Literal for Vector2 {
    fn x(self) -> f64 {
        f64::from(self.x)
    }

    fn y(self) -> f64 {
        f64::from(self.y)
    }
}

impl DebugVec2Literal for backend::RoadVec2 {
    fn x(self) -> f64 {
        self.x
    }

    fn y(self) -> f64 {
        self.y
    }
}

impl DebugVec3Literal for Vector3 {
    fn x(self) -> f64 {
        f64::from(self.x)
    }

    fn y(self) -> f64 {
        f64::from(self.y)
    }

    fn z(self) -> f64 {
        f64::from(self.z)
    }
}

impl DebugVec3Literal for backend::RoadVec3 {
    fn x(self) -> f64 {
        self.x
    }

    fn y(self) -> f64 {
        self.y
    }

    fn z(self) -> f64 {
        self.z
    }
}

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn append_match_stats_fields(
        dump: &mut String,
        stats: &DebugMatchStats,
    ) {
        let _ = write!(
            dump,
            "\"tested_vertices\":{},\"problem_count\":{},\"max_xz_error_m\":{:.4},\"max_y_error_m\":{:.4}",
            stats.total, stats.problem_count, stats.max_xz_error_m, stats.max_y_error_m
        );
    }

    pub(in crate::simulation::network::surface::debug) fn append_raw_json_samples(
        dump: &mut String,
        samples: &[String],
    ) {
        for (index, sample) in samples.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            dump.push_str(sample);
        }
    }

    pub(in crate::simulation::network::surface::debug) fn append_surface_sample_literal(
        dump: &mut String,
        terrain: &TerrainSystem,
        point: backend::RoadVec3,
    ) {
        let source_y_m =
            terrain.sample_height_world(point.x as f32, point.z as f32) * config::HEIGHT_SCALE;
        let visual_y_m = terrain.sample_visual_height_world(point.x as f32, point.z as f32)
            * config::HEIGHT_SCALE;
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

    pub(in crate::simulation::network::surface::debug) fn append_vector3_list_literal<T>(
        dump: &mut String,
        points: &[T],
    ) where
        T: Copy + DebugVec3Literal,
    {
        dump.push('[');
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_vector3_literal(dump, *point);
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_vector3_precise_list_literal<T>(
        dump: &mut String,
        points: &[T],
    ) where
        T: Copy + DebugVec3Literal,
    {
        dump.push('[');
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_vector3_precise_literal(dump, *point);
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_vector3_pair_precise_literal<T>(
        dump: &mut String,
        start: T,
        end: T,
    ) where
        T: Copy + DebugVec3Literal,
    {
        dump.push('[');
        Self::append_vector3_precise_literal(dump, start);
        dump.push_str(", ");
        Self::append_vector3_precise_literal(dump, end);
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_vector3_triangle_precise_literal<
        T,
    >(
        dump: &mut String,
        triangle: [T; 3],
    ) where
        T: Copy + DebugVec3Literal,
    {
        dump.push('[');
        Self::append_vector3_precise_literal(dump, triangle[0]);
        dump.push_str(", ");
        Self::append_vector3_precise_literal(dump, triangle[1]);
        dump.push_str(", ");
        Self::append_vector3_precise_literal(dump, triangle[2]);
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_vector3_triangle_list_precise_literal<
        T,
    >(
        dump: &mut String,
        triangles: &[[T; 3]],
    ) where
        T: Copy + DebugVec3Literal,
    {
        dump.push('[');
        for (index, triangle) in triangles.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_vector3_triangle_precise_literal(dump, *triangle);
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_usize_list_literal(
        dump: &mut String,
        values: &[usize],
    ) {
        dump.push('[');
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "{value}");
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_i64_list_literal(
        dump: &mut String,
        values: &[i64],
    ) {
        dump.push('[');
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "{value}");
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_optional_usize_literal(
        dump: &mut String,
        value: Option<usize>,
    ) {
        if let Some(value) = value {
            let _ = write!(dump, "{value}");
        } else {
            dump.push_str("null");
        }
    }

    pub(in crate::simulation::network::surface::debug) fn append_debug_render_edge_key_literal(
        dump: &mut String,
        key: DebugRenderEdgeKey,
    ) {
        dump.push('{');
        dump.push_str("\"start\":");
        Self::append_debug_render_vertex_key_literal(dump, key.start);
        dump.push_str(",\"end\":");
        Self::append_debug_render_vertex_key_literal(dump, key.end);
        dump.push('}');
    }

    pub(in crate::simulation::network::surface::debug) fn append_optional_debug_render_edge_key_literal(
        dump: &mut String,
        key: Option<DebugRenderEdgeKey>,
    ) {
        if let Some(key) = key {
            Self::append_debug_render_edge_key_literal(dump, key);
        } else {
            dump.push_str("null");
        }
    }

    pub(in crate::simulation::network::surface::debug) fn append_debug_render_vertex_key_literal(
        dump: &mut String,
        key: DebugRenderVertexKey,
    ) {
        let _ = write!(
            dump,
            "{{\"x_key\":{},\"y_mm\":{},\"z_key\":{}}}",
            key.x_key, key.y_mm, key.z_key
        );
    }

    pub(in crate::simulation::network::surface::debug) fn append_debug_render_xz_edge_key_literal(
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

    pub(in crate::simulation::network::surface::debug) fn append_debug_render_xz_vertex_key_literal(
        dump: &mut String,
        key: DebugRenderXzVertexKey,
    ) {
        let _ = write!(dump, "{{\"x_key\":{},\"z_key\":{}}}", key.x_key, key.z_key);
    }

    pub(in crate::simulation::network::surface::debug) fn append_chunk_key_list_literal(
        dump: &mut String,
        chunks: &[SurfaceChunkKey],
    ) {
        dump.push('[');
        for (index, chunk) in chunks.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "[{}, {}]", chunk.0, chunk.1);
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_vector3_literal<T>(
        dump: &mut String,
        point: T,
    ) where
        T: Copy + DebugVec3Literal,
    {
        let _ = write!(
            dump,
            "[{:.3}, {:.3}, {:.3}]",
            point.x(),
            point.y(),
            point.z()
        );
    }

    pub(in crate::simulation::network::surface::debug) fn append_vector3_precise_literal<T>(
        dump: &mut String,
        point: T,
    ) where
        T: Copy + DebugVec3Literal,
    {
        let _ = write!(
            dump,
            "[{:.6}, {:.6}, {:.6}]",
            point.x(),
            point.y(),
            point.z()
        );
    }

    pub(in crate::simulation::network::surface::debug) fn append_optional_vector3_precise_literal<
        T,
    >(
        dump: &mut String,
        point: Option<T>,
    ) where
        T: Copy + DebugVec3Literal,
    {
        if let Some(point) = point {
            Self::append_vector3_precise_literal(dump, point);
        } else {
            dump.push_str("null");
        }
    }

    pub(in crate::simulation::network::surface::debug) fn append_vector2_literal<T>(
        dump: &mut String,
        point: T,
    ) where
        T: Copy + DebugVec2Literal,
    {
        let _ = write!(dump, "[{:.3}, {:.3}]", point.x(), point.y());
    }

    pub(in crate::simulation::network::surface::debug) fn append_vector2_precise_literal<T>(
        dump: &mut String,
        point: T,
    ) where
        T: Copy + DebugVec2Literal,
    {
        let _ = write!(dump, "[{:.6}, {:.6}]", point.x(), point.y());
    }

    pub(in crate::simulation::network::surface::debug) fn append_vector2_precise_list_literal<T>(
        dump: &mut String,
        points: &[T],
    ) where
        T: Copy + DebugVec2Literal,
    {
        dump.push('[');
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_vector2_precise_literal(dump, *point);
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_optional_f32_literal(
        dump: &mut String,
        value: Option<f32>,
    ) {
        if let Some(value) = value {
            let _ = write!(dump, "{value:.3}");
        } else {
            dump.push_str("null");
        }
    }

    pub(in crate::simulation::network::surface::debug) fn append_optional_f32_precise_literal(
        dump: &mut String,
        value: Option<f32>,
    ) {
        if let Some(value) = value {
            let _ = write!(dump, "{value:.6}");
        } else {
            dump.push_str("null");
        }
    }
}
