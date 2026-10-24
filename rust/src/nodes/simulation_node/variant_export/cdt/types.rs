// SPDX-License-Identifier: GPL-2.0-only

//! Shared terrain CDT export buffer types.

use super::super::super::*;

/// Per-sample or per-face CDT sources for the debug display: each source's Rust label, kind
/// code (0 road, 1 building site) and primary id (edge, node or building).
#[derive(Default)]
pub(in crate::nodes::simulation_node) struct TerrainCdtSourceExport {
    pub(in crate::nodes::simulation_node) counts: Vec<i32>,
    pub(in crate::nodes::simulation_node) labels: Vec<String>,
    pub(in crate::nodes::simulation_node) kind_codes: Vec<i32>,
    pub(in crate::nodes::simulation_node) primary_ids: Vec<i32>,
}

impl TerrainCdtSourceExport {
    pub(in crate::nodes::simulation_node) fn with_sample_capacity(sample_count: usize) -> Self {
        Self {
            counts: Vec::with_capacity(sample_count),
            ..Self::default()
        }
    }

    pub(in crate::nodes::simulation_node) fn push_sources(
        &mut self,
        sources: &[TerrainCdtRoadBoundarySource],
    ) {
        self.counts
            .push(i32::try_from(sources.len()).unwrap_or(i32::MAX));
        for source in sources.iter().copied() {
            self.labels.push(source.debug_label());
            self.kind_codes.push(source.source_kind_code());
            self.primary_ids.push(source.primary_id_code());
        }
    }
}

pub(in crate::nodes::simulation_node) struct TerrainCdtTriangleBufferExport {
    pub(in crate::nodes::simulation_node) vertices: Vec<Vector3>,
    pub(in crate::nodes::simulation_node) normals: Vec<Vector3>,
    pub(in crate::nodes::simulation_node) normal_sum_lengths: Vec<f32>,
    pub(in crate::nodes::simulation_node) uvs: Vec<Vector2>,
    pub(in crate::nodes::simulation_node) indices: Vec<i32>,
    pub(in crate::nodes::simulation_node) face_sources: TerrainCdtSourceExport,
    pub(in crate::nodes::simulation_node) emitted_faces: usize,
    pub(in crate::nodes::simulation_node) omitted_pathological_faces: usize,
    pub(in crate::nodes::simulation_node) max_face_y_delta_m: f32,
    pub(in crate::nodes::simulation_node) max_face_slope_ratio: f32,
    pub(in crate::nodes::simulation_node) longest_triangle_edge_m: f32,
}

impl TerrainCdtTriangleBufferExport {
    pub(in crate::nodes::simulation_node) fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            normals: Vec::new(),
            normal_sum_lengths: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            face_sources: TerrainCdtSourceExport::default(),
            emitted_faces: 0,
            omitted_pathological_faces: 0,
            max_face_y_delta_m: 0.0,
            max_face_slope_ratio: 0.0,
            longest_triangle_edge_m: 0.0,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::nodes::simulation_node) struct TerrainCdtMeshBufferSummary {
    pub(in crate::nodes::simulation_node) max_face_y_delta_m: f32,
    pub(in crate::nodes::simulation_node) max_face_slope_ratio: f32,
    pub(in crate::nodes::simulation_node) longest_triangle_edge_m: f32,
    pub(in crate::nodes::simulation_node) terrain_max_face_slope_ratio: f32,
    pub(in crate::nodes::simulation_node) terrain_longest_triangle_edge_m: f32,
}

#[derive(Clone)]
pub(in crate::nodes::simulation_node) struct TerrainCdtWindowBounds {
    /// Parent-patch-clipped minimum world X.
    pub(in crate::nodes::simulation_node) min_x: f32,
    /// Parent-patch-clipped minimum world Z.
    pub(in crate::nodes::simulation_node) min_z: f32,
    /// Parent-patch-clipped maximum world X.
    pub(in crate::nodes::simulation_node) max_x: f32,
    /// Parent-patch-clipped maximum world Z.
    pub(in crate::nodes::simulation_node) max_z: f32,
    /// Canonical fixed-window boundary sample cadence in quantized millimetres.
    pub(in crate::nodes::simulation_node) boundary_step_mm: i64,
    /// Canonical Z samples emitted on the minimum-X side.
    pub(in crate::nodes::simulation_node) min_x_side_zs: Vec<f32>,
    /// Canonical Z samples emitted on the maximum-X side.
    pub(in crate::nodes::simulation_node) max_x_side_zs: Vec<f32>,
    /// Canonical X samples emitted on the minimum-Z side.
    pub(in crate::nodes::simulation_node) min_z_side_xs: Vec<f32>,
    /// Canonical X samples emitted on the maximum-Z side.
    pub(in crate::nodes::simulation_node) max_z_side_xs: Vec<f32>,
}
