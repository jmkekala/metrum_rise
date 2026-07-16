//! CDT debug sidecar sample export helpers.

use super::super::super::*;
use super::types::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn append_cdt_road_seam_face_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut centroids = Vec::with_capacity(mesh.road_seam_face_samples.len());
        let mut bounds = Vec::with_capacity(mesh.road_seam_face_samples.len() * 2);
        let mut metrics = Vec::with_capacity(mesh.road_seam_face_samples.len() * 2);
        let mut vertices = Vec::with_capacity(mesh.road_seam_face_samples.len() * 3);
        let mut kinds = Vec::with_capacity(mesh.road_seam_face_samples.len());
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.road_seam_face_samples.len());
        for sample in &mesh.road_seam_face_samples {
            centroids.push(Self::terrain_cdt_vertex_to_vector3(sample.centroid));
            bounds.push(Vector3::new(
                sample.min_x as f32,
                sample.min_y_m,
                sample.min_z as f32,
            ));
            bounds.push(Vector3::new(
                sample.max_x as f32,
                sample.max_y_m,
                sample.max_z as f32,
            ));
            metrics.push(sample.max_y_delta_m);
            metrics.push(sample.max_slope_ratio);
            kinds.push(sample.kind.debug_code());
            source_export.push_sources(&sample.sources);
            vertices.extend(
                sample
                    .vertices
                    .into_iter()
                    .map(Self::terrain_cdt_vertex_to_vector3),
            );
        }
        dict.set(
            "terrain_cdt_road_seam_sample_centroids",
            PackedVector3Array::from_iter(centroids),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_bounds",
            PackedVector3Array::from_iter(bounds),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_metrics",
            PackedFloat32Array::from_iter(metrics),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_vertices",
            PackedVector3Array::from_iter(vertices),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_kinds",
            PackedInt32Array::from_iter(kinds),
        );
        Self::append_cdt_sample_source_export(dict, "terrain_cdt_road_seam", &source_export);
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_retaining_wall_face_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut centroids = Vec::with_capacity(mesh.retaining_wall_face_samples.len());
        let mut bounds = Vec::with_capacity(mesh.retaining_wall_face_samples.len() * 2);
        let mut metrics = Vec::with_capacity(mesh.retaining_wall_face_samples.len() * 2);
        let mut vertices = Vec::with_capacity(mesh.retaining_wall_face_samples.len() * 3);
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.retaining_wall_face_samples.len());
        for sample in &mesh.retaining_wall_face_samples {
            centroids.push(Self::terrain_cdt_vertex_to_vector3(sample.centroid));
            bounds.push(Vector3::new(
                sample.min_x as f32,
                sample.min_y_m,
                sample.min_z as f32,
            ));
            bounds.push(Vector3::new(
                sample.max_x as f32,
                sample.max_y_m,
                sample.max_z as f32,
            ));
            metrics.push(sample.max_y_delta_m);
            metrics.push(sample.max_slope_ratio);
            source_export.push_sources(&sample.sources);
            vertices.extend(
                sample
                    .vertices
                    .into_iter()
                    .map(Self::terrain_cdt_vertex_to_vector3),
            );
        }
        dict.set(
            "terrain_cdt_retaining_wall_sample_centroids",
            PackedVector3Array::from_iter(centroids),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_bounds",
            PackedVector3Array::from_iter(bounds),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_metrics",
            PackedFloat32Array::from_iter(metrics),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_vertices",
            PackedVector3Array::from_iter(vertices),
        );
        Self::append_cdt_sample_source_export(dict, "terrain_cdt_retaining_wall", &source_export);
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_tie_in_widened_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut points = Vec::with_capacity(mesh.tie_in_widened_samples.len() * 2);
        let mut metrics = Vec::with_capacity(mesh.tie_in_widened_samples.len() * 4);
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.tie_in_widened_samples.len());
        for sample in &mesh.tie_in_widened_samples {
            points.push(Self::terrain_cdt_vertex_to_vector3(sample.source_sample));
            points.push(Self::terrain_cdt_vertex_to_vector3(sample.seam_point));
            source_export.push_sources(&[sample.seam_source]);
            metrics.push(sample.distance_m);
            metrics.push(sample.required_distance_m);
            metrics.push(sample.height_delta_m);
            metrics.push(sample.slope_ratio);
        }
        dict.set(
            "terrain_cdt_tie_in_widened_sample_points",
            PackedVector3Array::from_iter(points),
        );
        dict.set(
            "terrain_cdt_tie_in_widened_sample_metrics",
            PackedFloat32Array::from_iter(metrics),
        );
        Self::append_cdt_sample_source_export(dict, "terrain_cdt_tie_in_widened", &source_export);
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_invalid_constraint_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut edges = Vec::with_capacity(mesh.invalid_constraint_samples.len() * 2);
        let mut metadata = Vec::with_capacity(mesh.invalid_constraint_samples.len() * 4);
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.invalid_constraint_samples.len());
        for sample in &mesh.invalid_constraint_samples {
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.start));
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.end));
            metadata.push(if sample.road_owned { 1 } else { 0 });
            metadata.push(i32::try_from(sample.stable_piece_id).unwrap_or(i32::MAX));
            metadata.push(i32::try_from(sample.local_loop_index).unwrap_or(i32::MAX));
            metadata.push(i32::try_from(sample.local_edge_index).unwrap_or(i32::MAX));
            if let Some(source) = sample.source {
                source_export.push_sources(&[source]);
            } else {
                source_export.push_sources(&[]);
            }
        }
        dict.set(
            "terrain_cdt_invalid_constraint_sample_edges",
            PackedVector3Array::from_iter(edges),
        );
        dict.set(
            "terrain_cdt_invalid_constraint_sample_metadata",
            PackedInt32Array::from_iter(metadata),
        );
        Self::append_cdt_sample_source_export(
            dict,
            "terrain_cdt_invalid_constraint",
            &source_export,
        );
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_seam_quality_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut edges = Vec::with_capacity(mesh.seam_quality_samples.len() * 2);
        let mut metrics = Vec::with_capacity(mesh.seam_quality_samples.len() * 2);
        let mut kinds = Vec::with_capacity(mesh.seam_quality_samples.len());
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.seam_quality_samples.len());
        for sample in &mesh.seam_quality_samples {
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.start));
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.end));
            metrics.push(sample.length_m);
            metrics.push(sample.height_delta_m);
            kinds.push(sample.kind.debug_code());
            source_export.push_sources(&[sample.source]);
        }
        dict.set(
            "terrain_cdt_seam_quality_sample_edges",
            PackedVector3Array::from_iter(edges),
        );
        dict.set(
            "terrain_cdt_seam_quality_sample_metrics",
            PackedFloat32Array::from_iter(metrics),
        );
        dict.set(
            "terrain_cdt_seam_quality_sample_kinds",
            PackedInt32Array::from_iter(kinds),
        );
        Self::append_cdt_sample_source_export(dict, "terrain_cdt_seam_quality", &source_export);
    }

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
