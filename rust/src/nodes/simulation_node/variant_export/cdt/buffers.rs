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
            if let Some(bounds) =
                Self::terrain_cdt_window_bounds(patch, window.cdt_patch, boundary_step_m)
            {
                cdt_windows.push(bounds);
            }
        }
        Self::append_regular_terrain_mesh_outside_cdt_windows(
            &mut terrain_buffers,
            patch,
            &cdt_windows,
        );
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

    pub(in crate::nodes::simulation_node) fn terrain_cdt_window_final_buffer_stats(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        windows: &[(
            &CachedRefinedTerrainCdtWindow,
            &crate::simulation::terrain::cdt::TerrainCdtMesh,
        )],
        boundary_step_m: f32,
    ) -> TerrainCdtMeshBufferSummary {
        let mut terrain_buffers = TerrainCdtTriangleBufferExport::empty();
        let mut retaining_buffers = TerrainCdtTriangleBufferExport::empty();
        let mut cdt_windows = Vec::with_capacity(windows.len());
        for (window, mesh) in windows {
            let window_terrain_buffers = Self::terrain_cdt_triangle_buffers(
                patch,
                &mesh.vertices,
                &mesh.triangles,
                &mesh.terrain_triangle_sources,
                false,
                true,
            );
            Self::append_triangle_buffer_export(&mut terrain_buffers, window_terrain_buffers);
            let window_retaining_buffers = Self::terrain_cdt_triangle_buffers(
                patch,
                &mesh.vertices,
                &mesh.retaining_wall_triangles,
                &mesh.retaining_wall_triangle_sources,
                false,
                false,
            );
            Self::append_triangle_buffer_export(&mut retaining_buffers, window_retaining_buffers);
            if let Some(bounds) =
                Self::terrain_cdt_window_bounds(patch, window.cdt_patch, boundary_step_m)
            {
                cdt_windows.push(bounds);
            }
        }
        Self::append_regular_terrain_mesh_outside_cdt_windows(
            &mut terrain_buffers,
            patch,
            &cdt_windows,
        );
        let max_face_y_delta_m = terrain_buffers
            .max_face_y_delta_m
            .max(retaining_buffers.max_face_y_delta_m);
        let max_face_slope_ratio = terrain_buffers
            .max_face_slope_ratio
            .max(retaining_buffers.max_face_slope_ratio);
        let longest_triangle_edge_m = terrain_buffers
            .longest_triangle_edge_m
            .max(retaining_buffers.longest_triangle_edge_m);
        TerrainCdtMeshBufferSummary {
            max_face_y_delta_m,
            max_face_slope_ratio,
            longest_triangle_edge_m,
            terrain_max_face_slope_ratio: terrain_buffers.max_face_slope_ratio,
            terrain_longest_triangle_edge_m: terrain_buffers.longest_triangle_edge_m,
        }
    }

    pub(in crate::nodes::simulation_node) fn append_cdt_mesh_buffers(
        dict: &mut VarDictionary,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_patch: TerrainCdtPatch,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
        boundary_step_m: f32,
        include_debug: bool,
    ) -> TerrainCdtMeshBufferSummary {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let terrain_buffer_start = road_debug.then(Instant::now);
        let terrain_buffers = Self::terrain_cdt_triangle_buffers(
            patch,
            &mesh.vertices,
            &mesh.triangles,
            &mesh.terrain_triangle_sources,
            include_debug,
            true,
        );
        let mut terrain_buffers = terrain_buffers;
        Self::append_regular_terrain_mesh_outside_cdt_patch(
            &mut terrain_buffers,
            patch,
            cdt_patch,
            boundary_step_m,
        );
        let terrain_buffer_ms = terrain_buffer_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let retaining_buffer_start = road_debug.then(Instant::now);
        let retaining_buffers = Self::terrain_cdt_triangle_buffers(
            patch,
            &mesh.vertices,
            &mesh.retaining_wall_triangles,
            &mesh.retaining_wall_triangle_sources,
            include_debug,
            false,
        );
        let retaining_buffer_ms = retaining_buffer_start
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
                "terrain_cdt_mesh_buffers key=({},{}) include_debug={} terrain_vertices={} terrain_indices={} terrain_faces={} retaining_vertices={} retaining_indices={} retaining_faces={} omitted_pathological_terrain_faces={} final_max_face_y_delta_m={:.3} final_max_face_slope={:.3} final_longest_triangle_edge_m={:.3} terrain_buffer_ms={:.3} retaining_buffer_ms={:.3} dict_ms={:.3} total_ms={:.3}",
                patch.patch_x,
                patch.patch_z,
                include_debug,
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
                retaining_buffer_ms,
                dict_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
        metrics.summary()
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
}
