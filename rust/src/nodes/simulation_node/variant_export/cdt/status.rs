//! CDT status, stats, and cached terrain-mesh export helpers.

use super::super::super::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn append_cached_cdt_terrain_mesh(
        dict: &mut VarDictionary,
        cached: &CachedRefinedTerrainPatch,
        include_debug: bool,
    ) {
        Self::append_cdt_contract_metadata(dict);
        if cached.input_road_loops == 0 {
            if let Some(error_label) = cached.clip_error_label {
                Self::append_empty_cdt_failure(dict, error_label, include_debug);
            } else if cached.road_clip_source_count > 0 {
                Self::append_empty_cdt_failure(dict, "missing_road_clip_loops", include_debug);
            } else {
                dict.set("terrain_cdt_status", GString::from("empty"));
                dict.set("terrain_cdt_empty_refined", true);
                dict.set("terrain_cdt_mesh_suppressed", true);
            }
            return;
        }

        let successful_windows = cached
            .windows
            .iter()
            .filter_map(|window| window.mesh_result.as_ref().ok().map(|mesh| (window, mesh)))
            .collect::<Vec<_>>();
        if successful_windows.is_empty() {
            let error_label = cached
                .windows
                .iter()
                .find_map(|window| window.mesh_result.as_ref().err())
                .map(Self::terrain_cdt_error_label)
                .unwrap_or("missing_road_clip_loops");
            Self::append_empty_cdt_failure(dict, error_label, include_debug);
            return;
        }

        let has_conflicts = successful_windows
            .iter()
            .any(|(_, mesh)| Self::terrain_cdt_stats_have_constraint_conflicts(mesh.stats));
        if include_debug {
            Self::append_cdt_diagnostic_metadata(dict, TERRAIN_CDT_BACKEND_SPADE_LABEL);
        }
        let aggregate_stats = Self::aggregate_cdt_window_stats(&successful_windows);
        Self::append_cdt_stats(dict, aggregate_stats);
        let mesh_buffer_summary = Self::append_cdt_window_mesh_buffers(
            dict,
            &cached.patch,
            &successful_windows,
            (cached.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON),
            include_debug,
        );
        let max_face_slope_ratio = mesh_buffer_summary.terrain_max_face_slope_ratio;
        let longest_triangle_edge_m = mesh_buffer_summary.terrain_longest_triangle_edge_m;
        let pathological_output =
            Self::terrain_cdt_output_is_pathological(max_face_slope_ratio, longest_triangle_edge_m);
        dict.set(
            "terrain_cdt_status",
            GString::from(Self::terrain_cdt_output_status(
                has_conflicts,
                max_face_slope_ratio,
                longest_triangle_edge_m,
            )),
        );
        dict.set("terrain_cdt_pathological_output", pathological_output);
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_output_status(
        has_conflicts: bool,
        max_face_slope_ratio: f32,
        longest_triangle_edge_m: f32,
    ) -> &'static str {
        if has_conflicts {
            "conflicted"
        } else if Self::terrain_cdt_output_is_pathological(
            max_face_slope_ratio,
            longest_triangle_edge_m,
        ) {
            "pathological"
        } else {
            "ok"
        }
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_stats_have_constraint_conflicts(
        stats: TerrainCdtStats,
    ) -> bool {
        stats.invalid_constraint_edges > 0
            || stats.preserved_road_constraint_edges < stats.road_constraint_edges
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_output_is_pathological(
        max_face_slope_ratio: f32,
        longest_triangle_edge_m: f32,
    ) -> bool {
        (max_face_slope_ratio.is_finite()
            && max_face_slope_ratio > TERRAIN_CDT_PATHOLOGICAL_FACE_SLOPE_RATIO)
            || (longest_triangle_edge_m.is_finite()
                && longest_triangle_edge_m > TERRAIN_CDT_PATHOLOGICAL_TRIANGLE_EDGE_M)
    }

    pub(in crate::nodes::simulation_node) fn debug_log_pathological_terrain_cdt_output(
        label: &str,
        patch_x: usize,
        patch_z: usize,
        status: &'static str,
        max_face_slope_ratio: f32,
        longest_triangle_edge_m: f32,
        road_loops: usize,
        source_samples: usize,
    ) {
        if Self::terrain_cdt_output_is_pathological(max_face_slope_ratio, longest_triangle_edge_m) {
            debug_log!(
                "road",
                "WARNING {} key=({},{}) status={} max_face_slope={:.3} slope_threshold={:.3} longest_triangle_edge_m={:.3} longest_edge_threshold_m={:.3} road_loops={} source_samples={}",
                label,
                patch_x,
                patch_z,
                status,
                max_face_slope_ratio,
                TERRAIN_CDT_PATHOLOGICAL_FACE_SLOPE_RATIO,
                longest_triangle_edge_m,
                TERRAIN_CDT_PATHOLOGICAL_TRIANGLE_EDGE_M,
                road_loops,
                source_samples
            );
        }
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_stats(
        dict: &mut VarDictionary,
        stats: TerrainCdtStats,
    ) {
        Self::append_cdt_contract_metadata(dict);
        dict.set(
            "terrain_cdt_input_vertices",
            i64::try_from(stats.input_vertices).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_constraint_edges",
            i64::try_from(stats.constraint_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_road_constraint_edges",
            i64::try_from(stats.road_constraint_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_accepted_faces",
            i64::try_from(stats.accepted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_rejected_road_faces",
            i64::try_from(stats.rejected_road_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_preserved_road_constraint_edges",
            i64::try_from(stats.preserved_road_constraint_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_spade_missing_road_constraint_edges",
            i64::try_from(stats.spade_missing_road_constraint_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_rejected_road_constraint_edges",
            i64::try_from(stats.rejected_road_constraint_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_internal_road_constraint_edges",
            i64::try_from(stats.internal_road_constraint_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_invalid_constraints",
            i64::try_from(stats.invalid_constraint_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_max_face_y_delta_m",
            f64::from(stats.max_face_y_delta_m),
        );
        dict.set(
            "terrain_cdt_max_face_slope_ratio",
            f64::from(stats.max_face_slope_ratio),
        );
        dict.set(
            "terrain_cdt_longest_triangle_edge_m",
            f64::from(stats.longest_triangle_edge_m),
        );
        dict.set(
            "terrain_cdt_road_seam_faces",
            i64::try_from(stats.road_seam_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_road_seam_max_y_delta_m",
            f64::from(stats.road_seam_max_y_delta_m),
        );
        dict.set(
            "terrain_cdt_road_seam_max_slope_ratio",
            f64::from(stats.road_seam_max_slope_ratio),
        );
        dict.set(
            "terrain_cdt_retaining_wall_faces",
            i64::try_from(stats.retaining_wall_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_retaining_wall_max_y_delta_m",
            f64::from(stats.retaining_wall_max_y_delta_m),
        );
        dict.set(
            "terrain_cdt_retaining_wall_max_slope_ratio",
            f64::from(stats.retaining_wall_max_slope_ratio),
        );
        dict.set(
            "terrain_cdt_accepted_seam_edges",
            i64::try_from(stats.accepted_seam_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_merged_subbudget_seam_edges",
            i64::try_from(stats.merged_subbudget_seam_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_omitted_near_seam_source_samples",
            i64::try_from(stats.tie_in_widened_source_samples).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_retaining_wall_required_seam_edges",
            i64::try_from(stats.retaining_wall_required_seam_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_retaining_wall_required_seam_faces",
            i64::try_from(stats.retaining_wall_required_seam_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_blocking_degenerate_seam_edges",
            i64::try_from(stats.blocking_degenerate_seam_edges).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_tie_in_widened_source_samples",
            i64::try_from(stats.tie_in_widened_source_samples).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_tie_in_widened_max_y_delta_m",
            f64::from(stats.tie_in_widened_max_y_delta_m),
        );
        dict.set(
            "terrain_cdt_tie_in_widened_max_slope_ratio",
            f64::from(stats.tie_in_widened_max_slope_ratio),
        );
    }

    pub(in crate::nodes::simulation_node) fn aggregate_cdt_window_stats(
        windows: &[(&CachedRefinedTerrainCdtWindow, &TerrainCdtMesh)],
    ) -> TerrainCdtStats {
        let mut aggregate = TerrainCdtStats {
            input_vertices: 0,
            constraint_edges: 0,
            road_constraint_edges: 0,
            accepted_faces: 0,
            rejected_road_faces: 0,
            preserved_road_constraint_edges: 0,
            spade_missing_road_constraint_edges: 0,
            rejected_road_constraint_edges: 0,
            internal_road_constraint_edges: 0,
            invalid_constraint_edges: 0,
            max_face_y_delta_m: 0.0,
            max_face_slope_ratio: 0.0,
            longest_triangle_edge_m: 0.0,
            road_seam_faces: 0,
            road_seam_max_y_delta_m: 0.0,
            road_seam_max_slope_ratio: 0.0,
            retaining_wall_faces: 0,
            retaining_wall_max_y_delta_m: 0.0,
            retaining_wall_max_slope_ratio: 0.0,
            accepted_seam_edges: 0,
            merged_subbudget_seam_edges: 0,
            retaining_wall_required_seam_edges: 0,
            retaining_wall_required_seam_faces: 0,
            blocking_degenerate_seam_edges: 0,
            tie_in_widened_source_samples: 0,
            tie_in_widened_max_y_delta_m: 0.0,
            tie_in_widened_max_slope_ratio: 0.0,
        };
        for (_, mesh) in windows {
            let stats = mesh.stats;
            aggregate.input_vertices += stats.input_vertices;
            aggregate.constraint_edges += stats.constraint_edges;
            aggregate.road_constraint_edges += stats.road_constraint_edges;
            aggregate.accepted_faces += stats.accepted_faces;
            aggregate.rejected_road_faces += stats.rejected_road_faces;
            aggregate.preserved_road_constraint_edges += stats.preserved_road_constraint_edges;
            aggregate.spade_missing_road_constraint_edges +=
                stats.spade_missing_road_constraint_edges;
            aggregate.rejected_road_constraint_edges += stats.rejected_road_constraint_edges;
            aggregate.internal_road_constraint_edges += stats.internal_road_constraint_edges;
            aggregate.invalid_constraint_edges += stats.invalid_constraint_edges;
            aggregate.max_face_y_delta_m =
                aggregate.max_face_y_delta_m.max(stats.max_face_y_delta_m);
            aggregate.max_face_slope_ratio = aggregate
                .max_face_slope_ratio
                .max(stats.max_face_slope_ratio);
            aggregate.longest_triangle_edge_m = aggregate
                .longest_triangle_edge_m
                .max(stats.longest_triangle_edge_m);
            aggregate.road_seam_faces += stats.road_seam_faces;
            aggregate.road_seam_max_y_delta_m = aggregate
                .road_seam_max_y_delta_m
                .max(stats.road_seam_max_y_delta_m);
            aggregate.road_seam_max_slope_ratio = aggregate
                .road_seam_max_slope_ratio
                .max(stats.road_seam_max_slope_ratio);
            aggregate.retaining_wall_faces += stats.retaining_wall_faces;
            aggregate.retaining_wall_max_y_delta_m = aggregate
                .retaining_wall_max_y_delta_m
                .max(stats.retaining_wall_max_y_delta_m);
            aggregate.retaining_wall_max_slope_ratio = aggregate
                .retaining_wall_max_slope_ratio
                .max(stats.retaining_wall_max_slope_ratio);
            aggregate.accepted_seam_edges += stats.accepted_seam_edges;
            aggregate.merged_subbudget_seam_edges += stats.merged_subbudget_seam_edges;
            aggregate.retaining_wall_required_seam_edges +=
                stats.retaining_wall_required_seam_edges;
            aggregate.retaining_wall_required_seam_faces +=
                stats.retaining_wall_required_seam_faces;
            aggregate.blocking_degenerate_seam_edges += stats.blocking_degenerate_seam_edges;
            aggregate.tie_in_widened_source_samples += stats.tie_in_widened_source_samples;
            aggregate.tie_in_widened_max_y_delta_m = aggregate
                .tie_in_widened_max_y_delta_m
                .max(stats.tie_in_widened_max_y_delta_m);
            aggregate.tie_in_widened_max_slope_ratio = aggregate
                .tie_in_widened_max_slope_ratio
                .max(stats.tie_in_widened_max_slope_ratio);
        }
        aggregate
    }
}
