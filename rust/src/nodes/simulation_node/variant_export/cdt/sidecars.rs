// SPDX-License-Identifier: GPL-2.0-only

//! CDT debug sidecar sample export helpers.

use super::super::super::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn terrain_cdt_vertex_to_vector3(
        vertex: TerrainCdtVertex,
    ) -> Vector3 {
        Vector3::new(vertex.x as f32, vertex.height_m, vertex.z as f32)
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_error_label(
        err: &TerrainCdtError,
    ) -> &'static str {
        match err {
            TerrainCdtError::InvalidPatch => "invalid_patch",
            TerrainCdtError::MissingRoadBoundarySource => "missing_road_boundary_source",
            TerrainCdtError::ConflictingRoadBoundaryHeight => "conflicting_road_boundary_height",
            TerrainCdtError::TriangulationFailed => "triangulation_failed",
        }
    }
}
