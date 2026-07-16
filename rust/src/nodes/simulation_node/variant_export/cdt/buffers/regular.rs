//! Regular terrain filler buffers emitted outside refined CDT windows.

use super::super::super::super::*;
use super::super::types::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn append_regular_terrain_mesh_outside_cdt_patch(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_patch: TerrainCdtPatch,
        boundary_step_m: f32,
    ) {
        let windows = Self::terrain_cdt_window_bounds(patch, cdt_patch, boundary_step_m)
            .into_iter()
            .collect::<Vec<_>>();
        Self::append_regular_terrain_mesh_outside_cdt_windows(export, patch, &windows);
    }

    pub(in crate::nodes::simulation_node) fn append_regular_terrain_mesh_outside_cdt_windows(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        windows: &[TerrainCdtWindowBounds],
    ) {
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        if windows.is_empty() {
            Self::append_regular_terrain_grid_region(
                export,
                patch,
                patch_min_x,
                patch_min_z,
                patch_max_x,
                patch_max_z,
                Self::regular_terrain_mesh_step_m(patch),
            );
            return;
        }
        let step_m = Self::regular_terrain_mesh_step_m(patch);

        let mut x_lines = vec![patch_min_x, patch_max_x];
        let mut z_lines = vec![patch_min_z, patch_max_z];
        for window in windows {
            x_lines.extend([window.min_x, window.max_x]);
            z_lines.extend([window.min_z, window.max_z]);
        }
        Self::sort_dedup_axis_lines(&mut x_lines);
        Self::sort_dedup_axis_lines(&mut z_lines);
        for x_pair in x_lines.windows(2) {
            let min_x = x_pair[0];
            let max_x = x_pair[1];
            if max_x <= min_x + 0.001 {
                continue;
            }
            for z_pair in z_lines.windows(2) {
                let min_z = z_pair[0];
                let max_z = z_pair[1];
                if max_z <= min_z + 0.001 {
                    continue;
                }
                let mid_x = (min_x + max_x) * 0.5;
                let mid_z = (min_z + max_z) * 0.5;
                if windows.iter().any(|window| {
                    mid_x >= window.min_x
                        && mid_x <= window.max_x
                        && mid_z >= window.min_z
                        && mid_z <= window.max_z
                }) {
                    continue;
                }
                let mut xs =
                    Self::regular_terrain_axis_samples_aligned(min_x, max_x, step_m, patch_min_x);
                let mut zs =
                    Self::regular_terrain_axis_samples_aligned(min_z, max_z, step_m, patch_min_z);
                Self::refine_regular_terrain_axes_for_cdt_window_sides(
                    &mut xs, &mut zs, min_x, min_z, max_x, max_z, windows,
                );
                Self::append_regular_terrain_grid_region_with_axes(export, patch, &xs, &zs);
            }
        }
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_window_bounds(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_patch: TerrainCdtPatch,
        boundary_step_m: f32,
    ) -> Option<TerrainCdtWindowBounds> {
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        let min_x = (cdt_patch.min_x as f32).clamp(patch_min_x, patch_max_x);
        let min_z = (cdt_patch.min_z as f32).clamp(patch_min_z, patch_max_z);
        let max_x = (cdt_patch.max_x as f32).clamp(patch_min_x, patch_max_x);
        let max_z = (cdt_patch.max_z as f32).clamp(patch_min_z, patch_max_z);
        if max_x <= min_x + 0.001 || max_z <= min_z + 0.001 {
            return None;
        }
        Some(TerrainCdtWindowBounds {
            min_x,
            min_z,
            max_x,
            max_z,
            boundary_step_m: boundary_step_m.max(f32::EPSILON),
        })
    }

    pub(in crate::nodes::simulation_node) fn refine_regular_terrain_axes_for_cdt_window_sides(
        xs: &mut Vec<f32>,
        zs: &mut Vec<f32>,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        windows: &[TerrainCdtWindowBounds],
    ) {
        for window in windows {
            let z_overlap_min = min_z.max(window.min_z);
            let z_overlap_max = max_z.min(window.max_z);
            if z_overlap_max > z_overlap_min + 0.001
                && (Self::axis_lines_touch(max_x, window.min_x)
                    || Self::axis_lines_touch(min_x, window.max_x))
            {
                Self::extend_axis_samples(zs, z_overlap_min, z_overlap_max, window.boundary_step_m);
            }

            let x_overlap_min = min_x.max(window.min_x);
            let x_overlap_max = max_x.min(window.max_x);
            if x_overlap_max > x_overlap_min + 0.001
                && (Self::axis_lines_touch(max_z, window.min_z)
                    || Self::axis_lines_touch(min_z, window.max_z))
            {
                Self::extend_axis_samples(xs, x_overlap_min, x_overlap_max, window.boundary_step_m);
            }
        }
    }

    pub(in crate::nodes::simulation_node) fn axis_lines_touch(left: f32, right: f32) -> bool {
        (left - right).abs() <= 0.001
    }

    pub(in crate::nodes::simulation_node) fn extend_axis_samples(
        samples: &mut Vec<f32>,
        min: f32,
        max: f32,
        step_m: f32,
    ) {
        samples.extend(Self::terrain_cdt_axis_samples(min, max, step_m));
        Self::sort_dedup_axis_lines(samples);
    }

    pub(in crate::nodes::simulation_node) fn sort_dedup_axis_lines(values: &mut Vec<f32>) {
        values.sort_by(|left, right| left.total_cmp(right));
        values.dedup_by(|left, right| (*left - *right).abs() <= 0.001);
    }

    pub(in crate::nodes::simulation_node) fn regular_terrain_mesh_step_m(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
    ) -> f32 {
        let sample_step_x = patch.world_size_x / patch.sample_width.saturating_sub(1).max(1) as f32;
        let sample_step_z =
            patch.world_size_z / patch.sample_height.saturating_sub(1).max(1) as f32;
        sample_step_x
            .max(sample_step_z)
            .max(TERRAIN_CDT_FAR_SAMPLE_MIN_STEP_M)
    }

    pub(in crate::nodes::simulation_node) fn regular_terrain_axis_samples_aligned(
        min: f32,
        max: f32,
        step_m: f32,
        anchor: f32,
    ) -> Vec<f32> {
        let safe_step_m = step_m.max(f32::EPSILON);
        let mut samples = vec![min];
        let first = ((min - anchor) / safe_step_m).ceil() as i64;
        let last = ((max - anchor) / safe_step_m).floor() as i64;
        for index in first..=last {
            let sample = anchor + index as f32 * safe_step_m;
            if sample > min + 0.001 && sample < max - 0.001 {
                samples.push(sample);
            }
        }
        if samples
            .last()
            .is_none_or(|last| (*last - max).abs() > 0.001)
        {
            samples.push(max);
        }
        Self::sort_dedup_axis_lines(&mut samples);
        samples
    }

    pub(in crate::nodes::simulation_node) fn append_regular_terrain_grid_region(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        step_m: f32,
    ) {
        if max_x <= min_x + 0.001 || max_z <= min_z + 0.001 {
            return;
        }

        let xs =
            Self::regular_terrain_axis_samples_aligned(min_x, max_x, step_m, patch.world_origin_x);
        let zs =
            Self::regular_terrain_axis_samples_aligned(min_z, max_z, step_m, patch.world_origin_z);
        Self::append_regular_terrain_grid_region_with_axes(export, patch, &xs, &zs);
    }

    pub(in crate::nodes::simulation_node) fn append_regular_terrain_grid_region_with_axes(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        xs: &[f32],
        zs: &[f32],
    ) {
        if xs.len() < 2 || zs.len() < 2 {
            return;
        }

        let center_x = patch.world_origin_x + patch.world_size_x * 0.5;
        let center_z = patch.world_origin_z + patch.world_size_z * 0.5;
        let base_index = export.vertices.len();
        for &z in zs {
            for &x in xs {
                export.vertices.push(Vector3::new(
                    x - center_x,
                    Self::terrain_patch_height_at_world_m(patch, x, z),
                    z - center_z,
                ));
                export.normals.push(Vector3::ZERO);
                export.uvs.push(Vector2::new(
                    ((x - patch.world_origin_x) / patch.world_size_x.max(0.001)).clamp(0.0, 1.0),
                    ((z - patch.world_origin_z) / patch.world_size_z.max(0.001)).clamp(0.0, 1.0),
                ));
            }
        }

        let width = xs.len();
        for z_index in 0..zs.len() - 1 {
            for x_index in 0..xs.len() - 1 {
                let i00 = base_index + z_index * width + x_index;
                let i10 = i00 + 1;
                let i01 = i00 + width;
                let i11 = i01 + 1;
                Self::append_regular_terrain_triangle(export, [i00, i11, i10]);
                Self::append_regular_terrain_triangle(export, [i00, i01, i11]);
            }
        }

        for normal in &mut export.normals[base_index..] {
            if normal.length_squared() <= 0.000_001 {
                *normal = Vector3::UP;
            } else {
                *normal = normal.normalized();
            }
        }
    }

    pub(in crate::nodes::simulation_node) fn append_regular_terrain_triangle(
        export: &mut TerrainCdtTriangleBufferExport,
        triangle: [usize; 3],
    ) {
        let points = [
            export.vertices[triangle[0]],
            export.vertices[triangle[1]],
            export.vertices[triangle[2]],
        ];
        let normal = (points[1] - points[0]).cross(points[2] - points[0]);
        if normal.length_squared() <= 0.000_001 {
            return;
        }
        let normal = normal.normalized();
        let (face_y_delta_m, face_slope_ratio, face_longest_edge_m) =
            Self::terrain_buffer_triangle_metrics(points);
        export.max_face_y_delta_m = export.max_face_y_delta_m.max(face_y_delta_m);
        export.max_face_slope_ratio = export.max_face_slope_ratio.max(face_slope_ratio);
        export.longest_triangle_edge_m = export.longest_triangle_edge_m.max(face_longest_edge_m);
        for index in triangle {
            export.normals[index] = export.normals[index] + normal;
            export
                .indices
                .push(i32::try_from(index).unwrap_or(i32::MAX));
        }
        export.emitted_faces += 1;
        export.face_sources.push_sources(&[]);
    }

    pub(in crate::nodes::simulation_node) fn terrain_buffer_triangle_metrics(
        points: [Vector3; 3],
    ) -> (f32, f32, f32) {
        let max_y_delta_m = (points[1].y - points[0].y)
            .abs()
            .max((points[2].y - points[1].y).abs())
            .max((points[0].y - points[2].y).abs());
        let longest_edge_m = Self::terrain_buffer_edge_length_xz(points[0], points[1])
            .max(Self::terrain_buffer_edge_length_xz(points[1], points[2]))
            .max(Self::terrain_buffer_edge_length_xz(points[2], points[0]));
        let slope_ratio = Self::terrain_buffer_triangle_plane_slope_ratio(points);
        (max_y_delta_m, slope_ratio, longest_edge_m)
    }

    pub(in crate::nodes::simulation_node) fn terrain_buffer_edge_length_xz(
        a: Vector3,
        b: Vector3,
    ) -> f32 {
        let dx = b.x - a.x;
        let dz = b.z - a.z;
        (dx * dx + dz * dz).sqrt()
    }

    pub(in crate::nodes::simulation_node) fn terrain_buffer_triangle_plane_slope_ratio(
        points: [Vector3; 3],
    ) -> f32 {
        let a = points[1] - points[0];
        let b = points[2] - points[0];
        let normal = a.cross(b);
        let horizontal_normal = (normal.x * normal.x + normal.z * normal.z).sqrt();
        if horizontal_normal <= 0.000_001 {
            return 0.0;
        }
        if normal.y.abs() <= 0.000_001 {
            return 1_000_000.0;
        }
        horizontal_normal / normal.y.abs()
    }
}
