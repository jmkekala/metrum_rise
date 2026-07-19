//! CDT and regular-terrain mesh buffer export helpers.

use super::super::super::*;
use super::types::*;

mod regular;

#[derive(Clone, Copy)]
struct TerrainCdtMeshBufferExportMetrics {
    terrain_vertices: usize,
    terrain_indices: usize,
    retaining_vertices: usize,
    retaining_indices: usize,
    terrain_emitted_faces: usize,
    retaining_emitted_faces: usize,
    omitted_pathological_terrain_faces: usize,
    final_max_face_y_delta_m: f32,
    final_max_face_slope_ratio: f32,
    final_longest_triangle_edge_m: f32,
    terrain_max_face_slope_ratio: f32,
    terrain_longest_triangle_edge_m: f32,
}

impl TerrainCdtMeshBufferExportMetrics {
    fn summary(self) -> TerrainCdtMeshBufferSummary {
        TerrainCdtMeshBufferSummary {
            max_face_y_delta_m: self.final_max_face_y_delta_m,
            max_face_slope_ratio: self.final_max_face_slope_ratio,
            longest_triangle_edge_m: self.final_longest_triangle_edge_m,
            terrain_max_face_slope_ratio: self.terrain_max_face_slope_ratio,
            terrain_longest_triangle_edge_m: self.terrain_longest_triangle_edge_m,
        }
    }
}

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn cached_refined_terrain_mesh_buffer_summary(
        buffers: &CachedRefinedTerrainMeshBuffers,
    ) -> TerrainCdtMeshBufferSummary {
        TerrainCdtMeshBufferSummary {
            max_face_y_delta_m: buffers
                .terrain_max_face_y_delta_m
                .max(buffers.retaining_max_face_y_delta_m),
            max_face_slope_ratio: buffers
                .terrain_max_face_slope_ratio
                .max(buffers.retaining_max_face_slope_ratio),
            longest_triangle_edge_m: buffers
                .terrain_longest_triangle_edge_m
                .max(buffers.retaining_longest_triangle_edge_m),
            terrain_max_face_slope_ratio: buffers.terrain_max_face_slope_ratio,
            terrain_longest_triangle_edge_m: buffers.terrain_longest_triangle_edge_m,
        }
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_window_mesh_buffers(
        dict: &mut VarDictionary,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        windows: &[(
            &CachedRefinedTerrainCdtWindow,
            &crate::simulation::terrain::cdt::TerrainCdtMesh,
        )],
        boundary_step_m: f32,
        include_debug: bool,
    ) -> TerrainCdtMeshBufferSummary {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let terrain_buffer_start = road_debug.then(Instant::now);
        let mut terrain_buffers = TerrainCdtTriangleBufferExport::empty();
        let mut retaining_buffers = TerrainCdtTriangleBufferExport::empty();
        let mut cdt_windows = Vec::with_capacity(windows.len());
        for (window, mesh) in windows {
            let window_terrain_buffers = Self::terrain_cdt_triangle_buffers(
                patch,
                &mesh.vertices,
                &mesh.triangles,
                &mesh.terrain_triangle_sources,
                include_debug,
                true,
            );
            Self::append_triangle_buffer_export(&mut terrain_buffers, window_terrain_buffers);
            let window_retaining_buffers = Self::terrain_cdt_triangle_buffers(
                patch,
                &mesh.vertices,
                &mesh.retaining_wall_triangles,
                &mesh.retaining_wall_triangle_sources,
                include_debug,
                false,
            );
            Self::append_triangle_buffer_export(&mut retaining_buffers, window_retaining_buffers);
            if let Some(mut bounds) =
                Self::terrain_cdt_window_bounds(patch, window.cdt_patch, boundary_step_m)
            {
                Self::append_terrain_cdt_mesh_side_samples(&mut bounds, &mesh.vertices);
                cdt_windows.push(bounds);
            }
        }
        Self::append_regular_terrain_mesh_outside_cdt_windows(
            &mut terrain_buffers,
            patch,
            &cdt_windows,
        );
        Self::reconcile_terrain_mesh_duplicate_normals(&mut terrain_buffers);
        let terrain_buffer_ms = terrain_buffer_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let dict_start = road_debug.then(Instant::now);
        let metrics = Self::append_cdt_mesh_buffer_export(
            dict,
            terrain_buffers,
            retaining_buffers,
            include_debug,
        );
        let dict_ms = dict_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "terrain_cdt_window_mesh_buffers key=({},{}) include_debug={} windows={} terrain_vertices={} terrain_indices={} terrain_faces={} retaining_vertices={} retaining_indices={} retaining_faces={} omitted_pathological_terrain_faces={} final_max_face_y_delta_m={:.3} final_max_face_slope={:.3} final_longest_triangle_edge_m={:.3} terrain_buffer_ms={:.3} dict_ms={:.3} total_ms={:.3}",
                patch.patch_x,
                patch.patch_z,
                include_debug,
                windows.len(),
                metrics.terrain_vertices,
                metrics.terrain_indices,
                metrics.terrain_emitted_faces,
                metrics.retaining_vertices,
                metrics.retaining_indices,
                metrics.retaining_emitted_faces,
                metrics.omitted_pathological_terrain_faces,
                metrics.final_max_face_y_delta_m,
                metrics.final_max_face_slope_ratio,
                metrics.final_longest_triangle_edge_m,
                terrain_buffer_ms,
                dict_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
        metrics.summary()
    }

    pub(in crate::nodes::simulation_node) fn prepare_cached_refined_terrain_mesh_buffers(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        windows: &[(
            &CachedRefinedTerrainCdtWindow,
            &crate::simulation::terrain::cdt::TerrainCdtMesh,
        )],
        boundary_step_m: f32,
    ) -> CachedRefinedTerrainMeshBuffers {
        let mut terrain_buffers = TerrainCdtTriangleBufferExport::empty();
        let mut retaining_buffers = TerrainCdtTriangleBufferExport::empty();
        Self::reserve_cached_refined_window_mesh_buffers(
            &mut terrain_buffers,
            &mut retaining_buffers,
            windows,
        );
        let mut cdt_windows = Vec::with_capacity(windows.len());
        for (window, mesh) in windows {
            if let Some(buffers) = window.mesh_buffers.as_deref() {
                Self::append_cached_refined_window_mesh_buffers(
                    &mut terrain_buffers,
                    &mut retaining_buffers,
                    buffers,
                );
            } else {
                let buffers = Self::prepare_cached_refined_terrain_window_mesh_buffers(
                    patch,
                    window.cdt_patch,
                    boundary_step_m,
                    mesh,
                );
                Self::append_cached_refined_window_mesh_buffers(
                    &mut terrain_buffers,
                    &mut retaining_buffers,
                    &buffers,
                );
            }
            if let Some(mut bounds) =
                Self::terrain_cdt_window_bounds(patch, window.cdt_patch, boundary_step_m)
            {
                if let Some(buffers) = window.mesh_buffers.as_deref() {
                    bounds
                        .min_x_side_zs
                        .clone_from(&buffers.window_min_x_side_zs);
                    bounds
                        .max_x_side_zs
                        .clone_from(&buffers.window_max_x_side_zs);
                    bounds
                        .min_z_side_xs
                        .clone_from(&buffers.window_min_z_side_xs);
                    bounds
                        .max_z_side_xs
                        .clone_from(&buffers.window_max_z_side_xs);
                } else {
                    Self::append_terrain_cdt_mesh_side_samples(&mut bounds, &mesh.vertices);
                }
                cdt_windows.push(bounds);
            }
        }
        Self::append_regular_terrain_mesh_outside_cdt_windows(
            &mut terrain_buffers,
            patch,
            &cdt_windows,
        );
        Self::reconcile_terrain_mesh_duplicate_normals(&mut terrain_buffers);
        CachedRefinedTerrainMeshBuffers {
            terrain_vertices: terrain_buffers.vertices,
            terrain_normals: terrain_buffers.normals,
            terrain_normal_sum_lengths: Vec::new(),
            window_min_x_side_zs: Vec::new(),
            window_max_x_side_zs: Vec::new(),
            window_min_z_side_xs: Vec::new(),
            window_max_z_side_xs: Vec::new(),
            terrain_uvs: terrain_buffers.uvs,
            terrain_indices: terrain_buffers.indices,
            retaining_vertices: retaining_buffers.vertices,
            retaining_normals: retaining_buffers.normals,
            retaining_uvs: retaining_buffers.uvs,
            retaining_indices: retaining_buffers.indices,
            terrain_emitted_faces: terrain_buffers.emitted_faces,
            retaining_emitted_faces: retaining_buffers.emitted_faces,
            omitted_pathological_terrain_faces: terrain_buffers.omitted_pathological_faces,
            terrain_max_face_y_delta_m: terrain_buffers.max_face_y_delta_m,
            terrain_max_face_slope_ratio: terrain_buffers.max_face_slope_ratio,
            terrain_longest_triangle_edge_m: terrain_buffers.longest_triangle_edge_m,
            retaining_max_face_y_delta_m: retaining_buffers.max_face_y_delta_m,
            retaining_max_face_slope_ratio: retaining_buffers.max_face_slope_ratio,
            retaining_longest_triangle_edge_m: retaining_buffers.longest_triangle_edge_m,
        }
    }

    fn reserve_cached_refined_window_mesh_buffers(
        terrain_target: &mut TerrainCdtTriangleBufferExport,
        retaining_target: &mut TerrainCdtTriangleBufferExport,
        windows: &[(
            &CachedRefinedTerrainCdtWindow,
            &crate::simulation::terrain::cdt::TerrainCdtMesh,
        )],
    ) {
        let mut terrain_vertices = 0usize;
        let mut terrain_indices = 0usize;
        let mut retaining_vertices = 0usize;
        let mut retaining_indices = 0usize;
        for (window, mesh) in windows {
            if let Some(buffers) = window.mesh_buffers.as_deref() {
                terrain_vertices = terrain_vertices.saturating_add(buffers.terrain_vertices.len());
                terrain_indices = terrain_indices.saturating_add(buffers.terrain_indices.len());
                retaining_vertices =
                    retaining_vertices.saturating_add(buffers.retaining_vertices.len());
                retaining_indices =
                    retaining_indices.saturating_add(buffers.retaining_indices.len());
            } else {
                terrain_vertices = terrain_vertices.saturating_add(mesh.vertices.len());
                terrain_indices =
                    terrain_indices.saturating_add(mesh.triangles.len().saturating_mul(3));
                retaining_vertices = retaining_vertices.saturating_add(mesh.vertices.len());
                retaining_indices = retaining_indices
                    .saturating_add(mesh.retaining_wall_triangles.len().saturating_mul(3));
            }
        }
        terrain_target.vertices.reserve(terrain_vertices);
        terrain_target.normals.reserve(terrain_vertices);
        terrain_target.normal_sum_lengths.reserve(terrain_vertices);
        terrain_target.uvs.reserve(terrain_vertices);
        terrain_target.indices.reserve(terrain_indices);
        retaining_target.vertices.reserve(retaining_vertices);
        retaining_target.normals.reserve(retaining_vertices);
        retaining_target.uvs.reserve(retaining_vertices);
        retaining_target.indices.reserve(retaining_indices);
    }

    /// Converts one successful fixed CDT window into immutable patch-local render arrays.
    pub(in crate::nodes::simulation_node) fn prepare_cached_refined_terrain_window_mesh_buffers(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_patch: TerrainCdtPatch,
        boundary_step_m: f32,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) -> CachedRefinedTerrainMeshBuffers {
        let terrain_buffers = Self::terrain_cdt_triangle_buffers(
            patch,
            &mesh.vertices,
            &mesh.triangles,
            &mesh.terrain_triangle_sources,
            false,
            true,
        );
        let retaining_buffers = Self::terrain_cdt_triangle_buffers(
            patch,
            &mesh.vertices,
            &mesh.retaining_wall_triangles,
            &mesh.retaining_wall_triangle_sources,
            false,
            false,
        );
        let mut window_bounds = Self::terrain_cdt_window_bounds(patch, cdt_patch, boundary_step_m);
        if let Some(bounds) = window_bounds.as_mut() {
            Self::append_terrain_cdt_mesh_side_samples(bounds, &mesh.vertices);
        }
        CachedRefinedTerrainMeshBuffers {
            terrain_vertices: terrain_buffers.vertices,
            terrain_normals: terrain_buffers.normals,
            terrain_normal_sum_lengths: terrain_buffers.normal_sum_lengths,
            window_min_x_side_zs: window_bounds
                .as_ref()
                .map_or_else(Vec::new, |bounds| bounds.min_x_side_zs.clone()),
            window_max_x_side_zs: window_bounds
                .as_ref()
                .map_or_else(Vec::new, |bounds| bounds.max_x_side_zs.clone()),
            window_min_z_side_xs: window_bounds
                .as_ref()
                .map_or_else(Vec::new, |bounds| bounds.min_z_side_xs.clone()),
            window_max_z_side_xs: window_bounds
                .as_ref()
                .map_or_else(Vec::new, |bounds| bounds.max_z_side_xs.clone()),
            terrain_uvs: terrain_buffers.uvs,
            terrain_indices: terrain_buffers.indices,
            retaining_vertices: retaining_buffers.vertices,
            retaining_normals: retaining_buffers.normals,
            retaining_uvs: retaining_buffers.uvs,
            retaining_indices: retaining_buffers.indices,
            terrain_emitted_faces: terrain_buffers.emitted_faces,
            retaining_emitted_faces: retaining_buffers.emitted_faces,
            omitted_pathological_terrain_faces: terrain_buffers.omitted_pathological_faces,
            terrain_max_face_y_delta_m: terrain_buffers.max_face_y_delta_m,
            terrain_max_face_slope_ratio: terrain_buffers.max_face_slope_ratio,
            terrain_longest_triangle_edge_m: terrain_buffers.longest_triangle_edge_m,
            retaining_max_face_y_delta_m: retaining_buffers.max_face_y_delta_m,
            retaining_max_face_slope_ratio: retaining_buffers.max_face_slope_ratio,
            retaining_longest_triangle_edge_m: retaining_buffers.longest_triangle_edge_m,
        }
    }

    fn append_cached_refined_window_mesh_buffers(
        terrain_target: &mut TerrainCdtTriangleBufferExport,
        retaining_target: &mut TerrainCdtTriangleBufferExport,
        buffers: &CachedRefinedTerrainMeshBuffers,
    ) {
        Self::append_cached_triangle_buffers(
            terrain_target,
            &buffers.terrain_vertices,
            &buffers.terrain_normals,
            &buffers.terrain_normal_sum_lengths,
            &buffers.terrain_uvs,
            &buffers.terrain_indices,
            buffers.terrain_emitted_faces,
            buffers.omitted_pathological_terrain_faces,
            buffers.terrain_max_face_y_delta_m,
            buffers.terrain_max_face_slope_ratio,
            buffers.terrain_longest_triangle_edge_m,
        );
        Self::append_cached_triangle_buffers(
            retaining_target,
            &buffers.retaining_vertices,
            &buffers.retaining_normals,
            &[],
            &buffers.retaining_uvs,
            &buffers.retaining_indices,
            buffers.retaining_emitted_faces,
            0,
            buffers.retaining_max_face_y_delta_m,
            buffers.retaining_max_face_slope_ratio,
            buffers.retaining_longest_triangle_edge_m,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn append_cached_triangle_buffers(
        target: &mut TerrainCdtTriangleBufferExport,
        vertices: &[Vector3],
        normals: &[Vector3],
        normal_sum_lengths: &[f32],
        uvs: &[Vector2],
        indices: &[i32],
        emitted_faces: usize,
        omitted_pathological_faces: usize,
        max_face_y_delta_m: f32,
        max_face_slope_ratio: f32,
        longest_triangle_edge_m: f32,
    ) {
        let vertex_offset = i32::try_from(target.vertices.len()).unwrap_or(i32::MAX);
        target.vertices.extend_from_slice(vertices);
        target.normals.extend_from_slice(normals);
        target
            .normal_sum_lengths
            .extend_from_slice(normal_sum_lengths);
        target.uvs.extend_from_slice(uvs);
        target.indices.extend(
            indices
                .iter()
                .copied()
                .map(|index| index.saturating_add(vertex_offset)),
        );
        target.emitted_faces += emitted_faces;
        target.omitted_pathological_faces += omitted_pathological_faces;
        target.max_face_y_delta_m = target.max_face_y_delta_m.max(max_face_y_delta_m);
        target.max_face_slope_ratio = target.max_face_slope_ratio.max(max_face_slope_ratio);
        target.longest_triangle_edge_m =
            target.longest_triangle_edge_m.max(longest_triangle_edge_m);
    }

    pub(in crate::nodes::simulation_node) fn append_cached_refined_terrain_mesh_buffers(
        dict: &mut VarDictionary,
        buffers: &CachedRefinedTerrainMeshBuffers,
    ) -> TerrainCdtMeshBufferSummary {
        let summary = Self::cached_refined_terrain_mesh_buffer_summary(buffers);
        dict.set(
            "terrain_cdt_emitted_faces",
            i64::try_from(buffers.terrain_emitted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_retaining_wall_emitted_faces",
            i64::try_from(buffers.retaining_emitted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_max_face_y_delta_m",
            f64::from(summary.max_face_y_delta_m),
        );
        dict.set(
            "terrain_cdt_max_face_slope_ratio",
            f64::from(summary.max_face_slope_ratio),
        );
        dict.set(
            "terrain_cdt_longest_triangle_edge_m",
            f64::from(summary.longest_triangle_edge_m),
        );
        dict.set(
            "terrain_cdt_ordinary_max_face_y_delta_m",
            f64::from(buffers.terrain_max_face_y_delta_m),
        );
        dict.set(
            "terrain_cdt_ordinary_max_face_slope_ratio",
            f64::from(buffers.terrain_max_face_slope_ratio),
        );
        dict.set(
            "terrain_cdt_ordinary_longest_triangle_edge_m",
            f64::from(buffers.terrain_longest_triangle_edge_m),
        );
        dict.set(
            "terrain_cdt_retaining_wall_final_max_face_y_delta_m",
            f64::from(buffers.retaining_max_face_y_delta_m),
        );
        dict.set(
            "terrain_cdt_retaining_wall_final_max_face_slope_ratio",
            f64::from(buffers.retaining_max_face_slope_ratio),
        );
        dict.set(
            "terrain_cdt_retaining_wall_final_longest_triangle_edge_m",
            f64::from(buffers.retaining_longest_triangle_edge_m),
        );
        dict.set("terrain_cdt_mesh_suppressed", false);
        dict.set(
            "terrain_cdt_pathological_faces_omitted",
            i64::try_from(buffers.omitted_pathological_terrain_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_mesh_vertices",
            PackedVector3Array::from_iter(buffers.terrain_vertices.iter().copied()),
        );
        dict.set(
            "terrain_mesh_normals",
            PackedVector3Array::from_iter(buffers.terrain_normals.iter().copied()),
        );
        dict.set(
            "terrain_mesh_uvs",
            PackedVector2Array::from_iter(buffers.terrain_uvs.iter().copied()),
        );
        dict.set(
            "terrain_mesh_indices",
            PackedInt32Array::from_iter(buffers.terrain_indices.iter().copied()),
        );
        dict.set(
            "terrain_retaining_wall_mesh_vertices",
            PackedVector3Array::from_iter(buffers.retaining_vertices.iter().copied()),
        );
        dict.set(
            "terrain_retaining_wall_mesh_normals",
            PackedVector3Array::from_iter(buffers.retaining_normals.iter().copied()),
        );
        dict.set(
            "terrain_retaining_wall_mesh_uvs",
            PackedVector2Array::from_iter(buffers.retaining_uvs.iter().copied()),
        );
        dict.set(
            "terrain_retaining_wall_mesh_indices",
            PackedInt32Array::from_iter(buffers.retaining_indices.iter().copied()),
        );
        summary
    }

    fn append_cdt_mesh_buffer_export(
        dict: &mut VarDictionary,
        terrain_buffers: TerrainCdtTriangleBufferExport,
        retaining_buffers: TerrainCdtTriangleBufferExport,
        include_debug: bool,
    ) -> TerrainCdtMeshBufferExportMetrics {
        let metrics = TerrainCdtMeshBufferExportMetrics {
            terrain_vertices: terrain_buffers.vertices.len(),
            terrain_indices: terrain_buffers.indices.len(),
            retaining_vertices: retaining_buffers.vertices.len(),
            retaining_indices: retaining_buffers.indices.len(),
            terrain_emitted_faces: terrain_buffers.emitted_faces,
            retaining_emitted_faces: retaining_buffers.emitted_faces,
            omitted_pathological_terrain_faces: terrain_buffers.omitted_pathological_faces,
            final_max_face_y_delta_m: terrain_buffers
                .max_face_y_delta_m
                .max(retaining_buffers.max_face_y_delta_m),
            final_max_face_slope_ratio: terrain_buffers
                .max_face_slope_ratio
                .max(retaining_buffers.max_face_slope_ratio),
            final_longest_triangle_edge_m: terrain_buffers
                .longest_triangle_edge_m
                .max(retaining_buffers.longest_triangle_edge_m),
            terrain_max_face_slope_ratio: terrain_buffers.max_face_slope_ratio,
            terrain_longest_triangle_edge_m: terrain_buffers.longest_triangle_edge_m,
        };

        dict.set(
            "terrain_cdt_emitted_faces",
            i64::try_from(metrics.terrain_emitted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_retaining_wall_emitted_faces",
            i64::try_from(metrics.retaining_emitted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_max_face_y_delta_m",
            f64::from(metrics.final_max_face_y_delta_m),
        );
        dict.set(
            "terrain_cdt_max_face_slope_ratio",
            f64::from(metrics.final_max_face_slope_ratio),
        );
        dict.set(
            "terrain_cdt_longest_triangle_edge_m",
            f64::from(metrics.final_longest_triangle_edge_m),
        );
        dict.set(
            "terrain_cdt_ordinary_max_face_y_delta_m",
            f64::from(terrain_buffers.max_face_y_delta_m),
        );
        dict.set(
            "terrain_cdt_ordinary_max_face_slope_ratio",
            f64::from(terrain_buffers.max_face_slope_ratio),
        );
        dict.set(
            "terrain_cdt_ordinary_longest_triangle_edge_m",
            f64::from(terrain_buffers.longest_triangle_edge_m),
        );
        dict.set(
            "terrain_cdt_retaining_wall_final_max_face_y_delta_m",
            f64::from(retaining_buffers.max_face_y_delta_m),
        );
        dict.set(
            "terrain_cdt_retaining_wall_final_max_face_slope_ratio",
            f64::from(retaining_buffers.max_face_slope_ratio),
        );
        dict.set(
            "terrain_cdt_retaining_wall_final_longest_triangle_edge_m",
            f64::from(retaining_buffers.longest_triangle_edge_m),
        );
        dict.set("terrain_cdt_mesh_suppressed", false);
        dict.set(
            "terrain_cdt_pathological_faces_omitted",
            i64::try_from(metrics.omitted_pathological_terrain_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_mesh_vertices",
            PackedVector3Array::from_iter(terrain_buffers.vertices),
        );
        dict.set(
            "terrain_mesh_normals",
            PackedVector3Array::from_iter(terrain_buffers.normals),
        );
        dict.set(
            "terrain_mesh_uvs",
            PackedVector2Array::from_iter(terrain_buffers.uvs),
        );
        dict.set(
            "terrain_mesh_indices",
            PackedInt32Array::from_iter(terrain_buffers.indices),
        );
        if include_debug {
            Self::append_cdt_face_source_export(
                dict,
                "terrain_mesh",
                &terrain_buffers.face_sources,
            );
        }
        dict.set(
            "terrain_retaining_wall_mesh_vertices",
            PackedVector3Array::from_iter(retaining_buffers.vertices),
        );
        dict.set(
            "terrain_retaining_wall_mesh_normals",
            PackedVector3Array::from_iter(retaining_buffers.normals),
        );
        dict.set(
            "terrain_retaining_wall_mesh_uvs",
            PackedVector2Array::from_iter(retaining_buffers.uvs),
        );
        dict.set(
            "terrain_retaining_wall_mesh_indices",
            PackedInt32Array::from_iter(retaining_buffers.indices),
        );
        if include_debug {
            Self::append_cdt_face_source_export(
                dict,
                "terrain_retaining_wall_mesh",
                &retaining_buffers.face_sources,
            );
        }

        metrics
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_triangle_buffers(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        vertices_source: &[TerrainCdtVertex],
        triangles: &[[usize; 3]],
        triangle_sources: &[Vec<TerrainCdtRoadBoundarySource>],
        include_debug: bool,
        omit_pathological_terrain_faces: bool,
    ) -> TerrainCdtTriangleBufferExport {
        debug_assert_eq!(
            triangles.len(),
            triangle_sources.len(),
            "CDT triangle source sidecars must match the emitted triangle bucket"
        );
        let center_x = patch.world_origin_x + patch.world_size_x * 0.5;
        let center_z = patch.world_origin_z + patch.world_size_z * 0.5;
        let mut vertices = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::with_capacity(triangles.len() * 3);
        let mut vertex_remap = vec![usize::MAX; vertices_source.len()];
        let mut source_export = TerrainCdtSourceExport::with_sample_capacity(triangles.len());
        let mut emitted_faces = 0usize;
        let mut omitted_pathological_faces = 0usize;
        let mut max_face_y_delta_m = 0.0_f32;
        let mut max_face_slope_ratio = 0.0_f32;
        let mut longest_triangle_edge_m = 0.0_f32;

        for (triangle_index, triangle) in triangles.iter().enumerate() {
            let mut source_indices = *triangle;
            let mut points = [
                Self::terrain_cdt_vertex_to_vector3(vertices_source[triangle[0]]),
                Self::terrain_cdt_vertex_to_vector3(vertices_source[triangle[1]]),
                Self::terrain_cdt_vertex_to_vector3(vertices_source[triangle[2]]),
            ];
            let mut raw_normal = (points[1] - points[0]).cross(points[2] - points[0]);
            if raw_normal.length_squared() <= 0.000_001 {
                continue;
            }
            if raw_normal.y < 0.0 {
                source_indices.swap(1, 2);
                points.swap(1, 2);
                raw_normal = (points[1] - points[0]).cross(points[2] - points[0]);
            }
            let normal = raw_normal.normalized();
            let (face_y_delta_m, face_slope_ratio, face_longest_edge_m) =
                Self::terrain_buffer_triangle_metrics(points);
            if omit_pathological_terrain_faces
                && Self::terrain_cdt_output_is_pathological(face_slope_ratio, face_longest_edge_m)
            {
                omitted_pathological_faces += 1;
                continue;
            }
            max_face_y_delta_m = max_face_y_delta_m.max(face_y_delta_m);
            max_face_slope_ratio = max_face_slope_ratio.max(face_slope_ratio);
            longest_triangle_edge_m = longest_triangle_edge_m.max(face_longest_edge_m);
            emitted_faces += 1;
            if include_debug {
                let triangle_face_sources = triangle_sources
                    .get(triangle_index)
                    .map_or(&[][..], Vec::as_slice);
                source_export.push_sources(triangle_face_sources);
            }
            for source_index in source_indices {
                let mut export_index = vertex_remap[source_index];
                if export_index == usize::MAX {
                    let point = Self::terrain_cdt_vertex_to_vector3(vertices_source[source_index]);
                    export_index = vertices.len();
                    vertex_remap[source_index] = export_index;
                    vertices.push(Vector3::new(
                        point.x - center_x,
                        point.y,
                        point.z - center_z,
                    ));
                    normals.push(Vector3::new(0.0, 0.0, 0.0));
                    uvs.push(Vector2::new(
                        ((point.x - patch.world_origin_x) / patch.world_size_x.max(0.001))
                            .clamp(0.0, 1.0),
                        ((point.z - patch.world_origin_z) / patch.world_size_z.max(0.001))
                            .clamp(0.0, 1.0),
                    ));
                }
                normals[export_index] = normals[export_index] + normal;
                indices.push(i32::try_from(export_index).unwrap_or(i32::MAX));
            }
        }

        let normal_sum_lengths = normals
            .iter()
            .map(|normal| normal.length())
            .collect::<Vec<_>>();
        for normal in &mut normals {
            if normal.length_squared() <= 0.000_001 {
                *normal = Vector3::new(0.0, 1.0, 0.0);
            } else {
                *normal = normal.normalized();
            }
        }

        TerrainCdtTriangleBufferExport {
            vertices,
            normals,
            normal_sum_lengths,
            uvs,
            indices,
            face_sources: source_export,
            emitted_faces,
            omitted_pathological_faces,
            max_face_y_delta_m,
            max_face_slope_ratio,
            longest_triangle_edge_m,
        }
    }

    pub(in crate::nodes::simulation_node) fn append_triangle_buffer_export(
        target: &mut TerrainCdtTriangleBufferExport,
        source: TerrainCdtTriangleBufferExport,
    ) {
        let vertex_offset = i32::try_from(target.vertices.len()).unwrap_or(i32::MAX);
        target.vertices.extend(source.vertices);
        target.normals.extend(source.normals);
        target.normal_sum_lengths.extend(source.normal_sum_lengths);
        target.uvs.extend(source.uvs);
        target.indices.extend(
            source
                .indices
                .into_iter()
                .map(|index| index.saturating_add(vertex_offset)),
        );
        target
            .face_sources
            .counts
            .extend(source.face_sources.counts);
        target
            .face_sources
            .labels
            .extend(source.face_sources.labels);
        target
            .face_sources
            .kind_codes
            .extend(source.face_sources.kind_codes);
        target
            .face_sources
            .primary_ids
            .extend(source.face_sources.primary_ids);
        target
            .face_sources
            .node_kind_codes
            .extend(source.face_sources.node_kind_codes);
        target
            .face_sources
            .edge_class_codes
            .extend(source.face_sources.edge_class_codes);
        target
            .face_sources
            .owner_kinds
            .extend(source.face_sources.owner_kinds);
        target
            .face_sources
            .owner_indices
            .extend(source.face_sources.owner_indices);
        target
            .face_sources
            .support_policies
            .extend(source.face_sources.support_policies);
        target.face_sources.roles.extend(source.face_sources.roles);
        target
            .face_sources
            .section_ranges
            .extend(source.face_sources.section_ranges);
        target
            .face_sources
            .s_ranges
            .extend(source.face_sources.s_ranges);
        target.emitted_faces += source.emitted_faces;
        target.omitted_pathological_faces += source.omitted_pathological_faces;
        target.max_face_y_delta_m = target.max_face_y_delta_m.max(source.max_face_y_delta_m);
        target.max_face_slope_ratio = target.max_face_slope_ratio.max(source.max_face_slope_ratio);
        target.longest_triangle_edge_m = target
            .longest_triangle_edge_m
            .max(source.longest_triangle_edge_m);
    }

    /// Adds actual CDT side vertices to the canonical filler seam manifests.
    pub(in crate::nodes::simulation_node) fn append_terrain_cdt_mesh_side_samples(
        bounds: &mut TerrainCdtWindowBounds,
        vertices: &[TerrainCdtVertex],
    ) {
        for vertex in vertices {
            let x = vertex.x as f32;
            let z = vertex.z as f32;
            if Self::axis_lines_touch(x, bounds.min_x) {
                bounds.min_x_side_zs.push(z);
            }
            if Self::axis_lines_touch(x, bounds.max_x) {
                bounds.max_x_side_zs.push(z);
            }
            if Self::axis_lines_touch(z, bounds.min_z) {
                bounds.min_z_side_xs.push(x);
            }
            if Self::axis_lines_touch(z, bounds.max_z) {
                bounds.max_z_side_xs.push(x);
            }
        }
        Self::sort_dedup_axis_lines(&mut bounds.min_x_side_zs);
        Self::sort_dedup_axis_lines(&mut bounds.max_x_side_zs);
        Self::sort_dedup_axis_lines(&mut bounds.min_z_side_xs);
        Self::sort_dedup_axis_lines(&mut bounds.max_z_side_xs);
    }

    /// Assigns one accumulated normal to every quantized duplicate terrain vertex.
    pub(in crate::nodes::simulation_node) fn reconcile_terrain_mesh_duplicate_normals(
        export: &mut TerrainCdtTriangleBufferExport,
    ) {
        if export.vertices.is_empty() {
            return;
        }
        let vertex_key = |vertex: Vector3| {
            (
                (f64::from(vertex.x) * TERRAIN_CDT_SAMPLE_KEY_SCALE).round() as i64,
                (f64::from(vertex.y) * TERRAIN_CDT_SAMPLE_KEY_SCALE).round() as i64,
                (f64::from(vertex.z) * TERRAIN_CDT_SAMPLE_KEY_SCALE).round() as i64,
            )
        };
        let mut normal_sums =
            HashMap::<(i64, i64, i64), Vector3>::with_capacity(export.vertices.len());
        // A normalized vector plus its pre-normalization magnitude reconstructs the exact local
        // contribution unless the local sum hit the near-zero fallback. That rare case retains
        // the triangle walk so opposing faces cannot lose their direction.
        let cached_normal_sums_are_recoverable = export.normal_sum_lengths.len()
            == export.vertices.len()
            && export
                .normal_sum_lengths
                .iter()
                .all(|length| *length == 0.0 || length * length > 0.000_001);
        if cached_normal_sums_are_recoverable {
            for ((vertex, normal), sum_length) in export
                .vertices
                .iter()
                .zip(&export.normals)
                .zip(&export.normal_sum_lengths)
            {
                *normal_sums
                    .entry(vertex_key(*vertex))
                    .or_insert(Vector3::ZERO) += *normal * *sum_length;
            }
        } else {
            for triangle in export.indices.chunks_exact(3) {
                let Ok(i0) = usize::try_from(triangle[0]) else {
                    continue;
                };
                let Ok(i1) = usize::try_from(triangle[1]) else {
                    continue;
                };
                let Ok(i2) = usize::try_from(triangle[2]) else {
                    continue;
                };
                let Some(&p0) = export.vertices.get(i0) else {
                    continue;
                };
                let Some(&p1) = export.vertices.get(i1) else {
                    continue;
                };
                let Some(&p2) = export.vertices.get(i2) else {
                    continue;
                };
                let raw_normal = (p1 - p0).cross(p2 - p0);
                if raw_normal.length_squared() <= 0.000_001 {
                    continue;
                }
                let normal = raw_normal.normalized();
                for point in [p0, p1, p2] {
                    *normal_sums
                        .entry(vertex_key(point))
                        .or_insert(Vector3::ZERO) += normal;
                }
            }
        }
        for (vertex, normal) in export.vertices.iter().zip(&mut export.normals) {
            let sum = normal_sums
                .get(&vertex_key(*vertex))
                .copied()
                .unwrap_or(Vector3::UP);
            *normal = if sum.length_squared() <= 0.000_001 {
                Vector3::UP
            } else {
                sum.normalized()
            };
        }
    }
}
