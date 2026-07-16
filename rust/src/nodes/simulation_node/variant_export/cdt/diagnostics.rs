//! CDT build diagnostics, failure payloads, and source sidecar exports.

use super::super::super::*;
use super::types::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn append_cdt_terrain_mesh(
        dict: &mut VarDictionary,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_input: TerrainCdtInput,
        render_step_m: f32,
        has_grounded_road_contributors: bool,
        requires_road_clip: bool,
        clip_error_label: Option<&'static str>,
        include_debug: bool,
    ) {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let input_road_loops = cdt_input.road_loops.len();
        let input_source_samples = cdt_input.source_samples.len();
        let cdt_patch = cdt_input.patch;
        if cdt_input.road_loops.is_empty() {
            if let Some(error_label) = clip_error_label {
                Self::append_empty_cdt_failure(dict, error_label, include_debug);
            } else if has_grounded_road_contributors || requires_road_clip {
                Self::append_empty_cdt_failure(dict, "missing_road_clip_loops", include_debug);
            }
            if road_debug {
                debug_log!(
                    "road",
                    "terrain_cdt key=({},{}) include_debug={} status=empty road_loops={} source_samples={} total_ms={:.3}",
                    patch.patch_x,
                    patch.patch_z,
                    include_debug,
                    input_road_loops,
                    input_source_samples,
                    total_start
                        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                        .unwrap_or(0.0)
                );
            }
            return;
        }

        let cdt_start = road_debug.then(Instant::now);
        match build_road_touched_terrain_patch(cdt_input) {
            Ok(mesh) => {
                let cdt_ms = cdt_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                let has_conflicts = Self::terrain_cdt_stats_have_constraint_conflicts(mesh.stats);
                let metadata_start = road_debug.then(Instant::now);
                if include_debug {
                    Self::append_cdt_diagnostic_metadata(dict, TERRAIN_CDT_BACKEND_SPADE_LABEL);
                }
                Self::append_cdt_stats(dict, mesh.stats);
                let metadata_ms = metadata_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                let debug_sidecars_start = road_debug.then(Instant::now);
                if include_debug {
                    Self::append_cdt_road_seam_face_samples(dict, &mesh);
                    Self::append_cdt_retaining_wall_face_samples(dict, &mesh);
                    Self::append_cdt_tie_in_widened_samples(dict, &mesh);
                    Self::append_cdt_seam_quality_samples(dict, &mesh);
                    Self::append_cdt_invalid_constraint_samples(dict, &mesh);
                }
                let debug_sidecars_ms = debug_sidecars_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                let mesh_export_start = road_debug.then(Instant::now);
                let mesh_buffer_summary = Self::append_cdt_mesh_buffers(
                    dict,
                    patch,
                    cdt_patch,
                    &mesh,
                    render_step_m,
                    include_debug,
                );
                let mesh_export_ms = mesh_export_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                let max_face_slope_ratio = mesh_buffer_summary.terrain_max_face_slope_ratio;
                let longest_triangle_edge_m = mesh_buffer_summary.terrain_longest_triangle_edge_m;
                let cdt_status = Self::terrain_cdt_output_status(
                    has_conflicts,
                    max_face_slope_ratio,
                    longest_triangle_edge_m,
                );
                let pathological_output = Self::terrain_cdt_output_is_pathological(
                    max_face_slope_ratio,
                    longest_triangle_edge_m,
                );
                dict.set("terrain_cdt_status", GString::from(cdt_status));
                dict.set("terrain_cdt_pathological_output", pathological_output);
                if road_debug {
                    Self::debug_log_pathological_terrain_cdt_output(
                        "terrain_cdt_pathological_output",
                        patch.patch_x,
                        patch.patch_z,
                        cdt_status,
                        max_face_slope_ratio,
                        longest_triangle_edge_m,
                        input_road_loops,
                        input_source_samples,
                    );
                    debug_log!(
                        "road",
                        "terrain_cdt key=({},{}) include_debug={} status={} input_vertices={} road_loops={} source_samples={} constraints={} accepted_faces={} max_face_y_delta_m={:.3} max_face_slope={:.3} tie_in_widened_samples={} retaining_wall_faces={} longest_triangle_edge_m={:.3} cdt_ms={:.3} metadata_ms={:.3} debug_sidecars_ms={:.3} mesh_export_ms={:.3} total_ms={:.3}",
                        patch.patch_x,
                        patch.patch_z,
                        include_debug,
                        cdt_status,
                        mesh.stats.input_vertices,
                        input_road_loops,
                        input_source_samples,
                        mesh.stats.constraint_edges,
                        mesh.stats.accepted_faces,
                        mesh_buffer_summary.max_face_y_delta_m,
                        mesh_buffer_summary.max_face_slope_ratio,
                        mesh.stats.tie_in_widened_source_samples,
                        mesh.stats.retaining_wall_faces,
                        mesh_buffer_summary.longest_triangle_edge_m,
                        cdt_ms,
                        metadata_ms,
                        debug_sidecars_ms,
                        mesh_export_ms,
                        total_start
                            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                            .unwrap_or(0.0)
                    );
                }
            }
            Err(err) => {
                let cdt_ms = cdt_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                Self::append_empty_cdt_failure(
                    dict,
                    Self::terrain_cdt_error_label(&err),
                    include_debug,
                );
                if road_debug {
                    debug_log!(
                        "road",
                        "terrain_cdt key=({},{}) include_debug={} status=failed error={} road_loops={} source_samples={} cdt_ms={:.3} total_ms={:.3}",
                        patch.patch_x,
                        patch.patch_z,
                        include_debug,
                        Self::terrain_cdt_error_label(&err),
                        input_road_loops,
                        input_source_samples,
                        cdt_ms,
                        total_start
                            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                            .unwrap_or(0.0)
                    );
                }
            }
        }
    }

    pub(in crate::nodes::simulation_node) fn append_empty_cdt_failure(
        dict: &mut VarDictionary,
        error_label: &'static str,
        include_debug: bool,
    ) {
        Self::append_cdt_contract_metadata(dict);
        dict.set("terrain_cdt_status", GString::from("failed"));
        dict.set("terrain_cdt_pathological_output", false);
        dict.set("terrain_cdt_mesh_suppressed", false);
        dict.set("terrain_cdt_render_fallback", GString::new());
        dict.set("terrain_cdt_pathological_faces_omitted", 0i64);
        dict.set("terrain_cdt_error", GString::from(error_label));
        let backend_label = if error_label == "triangulation_failed" {
            TERRAIN_CDT_BACKEND_SPADE_LABEL
        } else {
            TERRAIN_CDT_BACKEND_NONE_LABEL
        };
        if include_debug {
            Self::append_cdt_diagnostic_metadata(dict, backend_label);
        }
        dict.set("terrain_cdt_input_vertices", 0i64);
        dict.set("terrain_cdt_constraint_edges", 0i64);
        dict.set("terrain_cdt_road_constraint_edges", 0i64);
        dict.set("terrain_cdt_accepted_faces", 0i64);
        dict.set("terrain_cdt_rejected_road_faces", 0i64);
        dict.set("terrain_cdt_preserved_road_constraint_edges", 0i64);
        dict.set("terrain_cdt_spade_missing_road_constraint_edges", 0i64);
        dict.set("terrain_cdt_rejected_road_constraint_edges", 0i64);
        dict.set("terrain_cdt_internal_road_constraint_edges", 0i64);
        dict.set("terrain_cdt_invalid_constraints", 1i64);
        dict.set("terrain_cdt_emitted_faces", 0i64);
        dict.set("terrain_cdt_retaining_wall_emitted_faces", 0i64);
        dict.set("terrain_cdt_max_face_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_max_face_slope_ratio", 0.0f64);
        dict.set("terrain_cdt_longest_triangle_edge_m", 0.0f64);
        dict.set("terrain_cdt_ordinary_max_face_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_ordinary_max_face_slope_ratio", 0.0f64);
        dict.set("terrain_cdt_ordinary_longest_triangle_edge_m", 0.0f64);
        dict.set(
            "terrain_cdt_retaining_wall_final_max_face_y_delta_m",
            0.0f64,
        );
        dict.set(
            "terrain_cdt_retaining_wall_final_max_face_slope_ratio",
            0.0f64,
        );
        dict.set(
            "terrain_cdt_retaining_wall_final_longest_triangle_edge_m",
            0.0f64,
        );
        dict.set("terrain_cdt_road_seam_faces", 0i64);
        dict.set("terrain_cdt_road_seam_max_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_road_seam_max_slope_ratio", 0.0f64);
        dict.set("terrain_cdt_retaining_wall_faces", 0i64);
        dict.set("terrain_cdt_retaining_wall_max_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_retaining_wall_max_slope_ratio", 0.0f64);
        dict.set("terrain_cdt_accepted_seam_edges", 0i64);
        dict.set("terrain_cdt_merged_subbudget_seam_edges", 0i64);
        dict.set("terrain_cdt_omitted_near_seam_source_samples", 0i64);
        dict.set("terrain_cdt_retaining_wall_required_seam_edges", 0i64);
        dict.set("terrain_cdt_retaining_wall_required_seam_faces", 0i64);
        dict.set("terrain_cdt_blocking_degenerate_seam_edges", 0i64);
        dict.set("terrain_cdt_tie_in_widened_source_samples", 0i64);
        dict.set("terrain_cdt_tie_in_widened_max_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_tie_in_widened_max_slope_ratio", 0.0f64);
        dict.set("terrain_mesh_vertices", PackedVector3Array::new());
        dict.set("terrain_mesh_normals", PackedVector3Array::new());
        dict.set("terrain_mesh_uvs", PackedVector2Array::new());
        dict.set("terrain_mesh_indices", PackedInt32Array::new());
        dict.set(
            "terrain_retaining_wall_mesh_vertices",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_retaining_wall_mesh_normals",
            PackedVector3Array::new(),
        );
        dict.set("terrain_retaining_wall_mesh_uvs", PackedVector2Array::new());
        dict.set(
            "terrain_retaining_wall_mesh_indices",
            PackedInt32Array::new(),
        );
        if !include_debug {
            return;
        }
        dict.set(
            "terrain_cdt_road_seam_sample_centroids",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_bounds",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_metrics",
            PackedFloat32Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_vertices",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_kinds",
            PackedInt32Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_road_seam");
        dict.set(
            "terrain_cdt_retaining_wall_sample_centroids",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_bounds",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_metrics",
            PackedFloat32Array::new(),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_vertices",
            PackedVector3Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_retaining_wall");
        dict.set(
            "terrain_cdt_tie_in_widened_sample_points",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_tie_in_widened_sample_metrics",
            PackedFloat32Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_tie_in_widened");
        dict.set(
            "terrain_cdt_seam_quality_sample_edges",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_seam_quality_sample_metrics",
            PackedFloat32Array::new(),
        );
        dict.set(
            "terrain_cdt_seam_quality_sample_kinds",
            PackedInt32Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_seam_quality");
        dict.set(
            "terrain_cdt_invalid_constraint_sample_edges",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_invalid_constraint_sample_metadata",
            PackedInt32Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_invalid_constraint");
        Self::append_empty_cdt_face_source_export(dict, "terrain_mesh");
        Self::append_empty_cdt_face_source_export(dict, "terrain_retaining_wall_mesh");
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_contract_metadata(
        dict: &mut VarDictionary,
    ) {
        dict.set(
            "terrain_cdt_contract_revision",
            TERRAIN_CDT_CONTRACT_REVISION,
        );
    }

    pub(in crate::nodes::simulation_node) fn refined_terrain_patch_cache_entry_is_current(
        cached: &CachedRefinedTerrainPatch,
        surface_generation: u64,
    ) -> bool {
        cached.contract_revision == TERRAIN_CDT_CONTRACT_REVISION
            && cached.surface_generation == surface_generation
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_diagnostic_metadata(
        dict: &mut VarDictionary,
        backend_label: &str,
    ) {
        let backend_code = if backend_label == TERRAIN_CDT_BACKEND_SPADE_LABEL {
            TERRAIN_CDT_BACKEND_SPADE_CODE
        } else {
            TERRAIN_CDT_BACKEND_NONE_CODE
        };
        dict.set(
            "terrain_cdt_diagnostic_stage",
            GString::from(TERRAIN_CDT_DIAGNOSTIC_STAGE_LABEL),
        );
        dict.set(
            "terrain_cdt_diagnostic_stage_code",
            TERRAIN_CDT_DIAGNOSTIC_STAGE_CODE,
        );
        dict.set(
            "terrain_cdt_diagnostic_backend",
            GString::from(backend_label),
        );
        dict.set("terrain_cdt_diagnostic_backend_code", backend_code);
    }

    pub(in crate::nodes::simulation_node) fn append_empty_cdt_face_source_export(
        dict: &mut VarDictionary,
        prefix: &str,
    ) {
        Self::append_cdt_face_source_export(dict, prefix, &TerrainCdtSourceExport::default());
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_face_source_export(
        dict: &mut VarDictionary,
        prefix: &str,
        export: &TerrainCdtSourceExport,
    ) {
        let field_prefix = format!("{prefix}_face_source");
        let label_key = format!("{prefix}_face_sources");
        Self::append_cdt_source_export(dict, &field_prefix, &label_key, export);
    }

    pub(in crate::nodes::simulation_node) fn append_empty_cdt_sample_source_export(
        dict: &mut VarDictionary,
        prefix: &str,
    ) {
        Self::append_cdt_sample_source_export(dict, prefix, &TerrainCdtSourceExport::default());
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_sample_source_export(
        dict: &mut VarDictionary,
        prefix: &str,
        export: &TerrainCdtSourceExport,
    ) {
        let field_prefix = format!("{prefix}_sample_source");
        let label_key = format!("{prefix}_sample_sources");
        Self::append_cdt_source_export(dict, &field_prefix, &label_key, export);
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_source_export(
        dict: &mut VarDictionary,
        field_prefix: &str,
        label_key: &str,
        export: &TerrainCdtSourceExport,
    ) {
        Self::set_cdt_source_i32(dict, field_prefix, "counts", &export.counts);
        dict.set(
            label_key,
            PackedStringArray::from_iter(
                export
                    .labels
                    .iter()
                    .map(|label| GString::from(label.as_str())),
            ),
        );
        Self::set_cdt_source_i32(dict, field_prefix, "kind_codes", &export.kind_codes);
        Self::set_cdt_source_i32(dict, field_prefix, "primary_ids", &export.primary_ids);
        Self::set_cdt_source_i32(
            dict,
            field_prefix,
            "node_kind_codes",
            &export.node_kind_codes,
        );
        Self::set_cdt_source_i32(
            dict,
            field_prefix,
            "edge_class_codes",
            &export.edge_class_codes,
        );
        Self::set_cdt_source_i32(dict, field_prefix, "owner_kinds", &export.owner_kinds);
        Self::set_cdt_source_i32(dict, field_prefix, "owner_indices", &export.owner_indices);
        Self::set_cdt_source_i32(
            dict,
            field_prefix,
            "support_policies",
            &export.support_policies,
        );
        Self::set_cdt_source_i32(dict, field_prefix, "roles", &export.roles);
        Self::set_cdt_source_i32(dict, field_prefix, "section_ranges", &export.section_ranges);
        Self::set_cdt_source_f32(dict, field_prefix, "s_ranges", &export.s_ranges);
    }

    pub(in crate::nodes::simulation_node) fn set_cdt_source_i32(
        dict: &mut VarDictionary,
        field_prefix: &str,
        suffix: &str,
        values: &[i32],
    ) {
        let key = format!("{field_prefix}_{suffix}");
        dict.set(
            key.as_str(),
            PackedInt32Array::from_iter(values.iter().copied()),
        );
    }

    pub(in crate::nodes::simulation_node) fn set_cdt_source_f32(
        dict: &mut VarDictionary,
        field_prefix: &str,
        suffix: &str,
        values: &[f32],
    ) {
        let key = format!("{field_prefix}_{suffix}");
        dict.set(
            key.as_str(),
            PackedFloat32Array::from_iter(values.iter().copied()),
        );
    }
}
