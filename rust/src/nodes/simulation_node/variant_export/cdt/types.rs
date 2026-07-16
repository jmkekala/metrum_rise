//! Shared terrain CDT export buffer types.

use super::super::super::*;

#[derive(Default)]
pub(in crate::nodes::simulation_node) struct TerrainCdtSourceExport {
    pub(in crate::nodes::simulation_node) counts: Vec<i32>,
    pub(in crate::nodes::simulation_node) labels: Vec<String>,
    pub(in crate::nodes::simulation_node) kind_codes: Vec<i32>,
    pub(in crate::nodes::simulation_node) primary_ids: Vec<i32>,
    pub(in crate::nodes::simulation_node) node_kind_codes: Vec<i32>,
    pub(in crate::nodes::simulation_node) edge_class_codes: Vec<i32>,
    pub(in crate::nodes::simulation_node) owner_kinds: Vec<i32>,
    pub(in crate::nodes::simulation_node) owner_indices: Vec<i32>,
    pub(in crate::nodes::simulation_node) support_policies: Vec<i32>,
    pub(in crate::nodes::simulation_node) roles: Vec<i32>,
    pub(in crate::nodes::simulation_node) section_ranges: Vec<i32>,
    pub(in crate::nodes::simulation_node) s_ranges: Vec<f32>,
}

impl TerrainCdtSourceExport {
    pub(in crate::nodes::simulation_node) fn with_sample_capacity(sample_count: usize) -> Self {
        Self {
            counts: Vec::with_capacity(sample_count),
            labels: Vec::new(),
            kind_codes: Vec::new(),
            primary_ids: Vec::new(),
            node_kind_codes: Vec::new(),
            edge_class_codes: Vec::new(),
            owner_kinds: Vec::new(),
            owner_indices: Vec::new(),
            support_policies: Vec::new(),
            roles: Vec::new(),
            section_ranges: Vec::new(),
            s_ranges: Vec::new(),
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
            self.node_kind_codes.push(source.node_kind_code());
            self.edge_class_codes.push(source.edge_class_code());
            self.owner_kinds.push(source.owner_kind_code());
            self.owner_indices.push(source.owner_index_code());
            self.support_policies.push(source.support_policy_code());
            self.roles.push(source.role_code());
            self.section_ranges.extend(source.section_range_codes());
            self.s_ranges.extend(source.s_range_values());
        }
    }
}

pub(in crate::nodes::simulation_node) struct TerrainCdtTriangleBufferExport {
    pub(in crate::nodes::simulation_node) vertices: Vec<Vector3>,
    pub(in crate::nodes::simulation_node) normals: Vec<Vector3>,
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

#[derive(Clone, Copy)]
pub(in crate::nodes::simulation_node) struct TerrainCdtWindowBounds {
    pub(in crate::nodes::simulation_node) min_x: f32,
    pub(in crate::nodes::simulation_node) min_z: f32,
    pub(in crate::nodes::simulation_node) max_x: f32,
    pub(in crate::nodes::simulation_node) max_z: f32,
    pub(in crate::nodes::simulation_node) boundary_step_m: f32,
}
