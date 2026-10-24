// SPDX-License-Identifier: GPL-2.0-only

//! Regular terrain filler buffers emitted outside refined CDT windows.
//!
//! The filler conforms to everything around it (`ROAD-68-D`): a cell side shared with a CDT
//! window takes that window's exported side vertices bitwise, and every other side (another
//! cell, the patch side) takes the render-step boundary lattice CDT windows put on their sides,
//! at the patch height function the windows' side samples use. Inside a cell a regular grid
//! follows the terrain samples, and one strip per side joins the side to the grid's outer row.

use super::super::super::super::*;
use super::super::types::*;

// Patch-local f32 of a world coordinate, rounded once from f64 exactly as the CDT export does.
fn patch_local(world: f32, centre: f32) -> f32 {
    crate::simulation::terrain::render_frame_local(f64::from(world), centre)
}

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn append_regular_terrain_mesh_outside_cdt_windows(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        windows: &[TerrainCdtWindowBounds],
        boundary_step_m: f32,
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
        // Cells are bounded by the patch and window lines only, so every cell side lies wholly
        // on one window side or on none.
        let mut x_lines = vec![patch_min_x, patch_max_x];
        let mut z_lines = vec![patch_min_z, patch_max_z];
        for window in windows {
            x_lines.extend([window.min_x, window.max_x]);
            z_lines.extend([window.min_z, window.max_z]);
        }
        for lines in [&mut x_lines, &mut z_lines] {
            lines.sort_by(f32::total_cmp);
            lines.dedup();
        }
        for x_pair in x_lines.windows(2) {
            for z_pair in z_lines.windows(2) {
                let cell = [x_pair[0], z_pair[0], x_pair[1], z_pair[1]];
                let mid_x = (cell[0] + cell[2]) * 0.5;
                let mid_z = (cell[1] + cell[3]) * 0.5;
                if windows.iter().any(|window| {
                    mid_x >= window.min_x
                        && mid_x <= window.max_x
                        && mid_z >= window.min_z
                        && mid_z <= window.max_z
                }) {
                    continue;
                }
                Self::append_regular_terrain_filler_cell(
                    export,
                    patch,
                    windows,
                    cell,
                    boundary_step_m,
                );
            }
        }
    }

    // One filler cell `[min_x, min_z, max_x, max_z]`: the regular grid strictly inside (or the
    // cell's midline where it is narrower than two steps) and four strips joining it to the
    // sides. O(grid vertices + side vertices).
    fn append_regular_terrain_filler_cell(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        windows: &[TerrainCdtWindowBounds],
        [min_x, min_z, max_x, max_z]: [f32; 4],
        boundary_step_m: f32,
    ) {
        let step_m = Self::regular_terrain_mesh_step_m(patch);
        let interior = |min: f32, max: f32, anchor: f32| {
            let samples = Self::regular_terrain_axis_samples_aligned(min, max, step_m, anchor);
            match &samples[1..samples.len() - 1] {
                [] => vec![(min + max) * 0.5],
                lines => lines.to_vec(),
            }
        };
        let xs = interior(min_x, max_x, patch.world_origin_x);
        let zs = interior(min_z, max_z, patch.world_origin_z);
        let base_index = export.vertices.len();
        // One vertex per XZ position in the cell; window vertices go first, so a corner a window
        // shares keeps the window's bits.
        let mut indices = HashMap::<(u32, u32), usize>::new();
        // Bottom, right, top and left sides.
        let mut sides = [
            (min_z, (min_x, max_x), false),
            (max_x, (min_z, max_z), true),
            (max_z, (min_x, max_x), false),
            (min_x, (min_z, max_z), true),
        ]
        .into_iter()
        .enumerate()
        .map(|(slot, (fixed, along, x_line))| {
            let (points, from_window) = Self::regular_terrain_filler_side(
                patch,
                windows,
                fixed,
                along,
                x_line,
                boundary_step_m,
            );
            (slot, points, from_window)
        })
        .collect::<Vec<_>>();
        sides.sort_by_key(|(_, _, from_window)| !*from_window);
        let mut side_rows = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for (slot, points, _) in sides {
            side_rows[slot] = points
                .into_iter()
                .map(|point| {
                    Self::regular_terrain_filler_vertex(export, patch, &mut indices, point)
                })
                .collect::<Vec<_>>();
        }
        let (centre_x, centre_z) = patch.frame_centre();
        let grid = zs
            .iter()
            .map(|&z| {
                xs.iter()
                    .map(|&x| {
                        let point = Vector3::new(
                            patch_local(x, centre_x),
                            Self::terrain_patch_height_at_world_m(patch, x, z),
                            patch_local(z, centre_z),
                        );
                        Self::regular_terrain_filler_vertex(export, patch, &mut indices, point)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for rows in grid.windows(2) {
            for x_index in 0..xs.len() - 1 {
                let (i00, i10) = (rows[0][x_index], rows[0][x_index + 1]);
                let (i01, i11) = (rows[1][x_index], rows[1][x_index + 1]);
                Self::append_regular_terrain_triangle(export, [i00, i11, i10]);
                Self::append_regular_terrain_triangle(export, [i00, i01, i11]);
            }
        }
        let first_column = grid.iter().map(|row| row[0]).collect::<Vec<_>>();
        let last_column = grid.iter().map(|row| row[xs.len() - 1]).collect::<Vec<_>>();
        let [bottom, right, top, left] = &side_rows;
        Self::append_regular_terrain_strip(export, bottom, &grid[0], false);
        Self::append_regular_terrain_strip(export, right, &last_column, true);
        Self::append_regular_terrain_strip(export, top, &grid[zs.len() - 1], false);
        Self::append_regular_terrain_strip(export, left, &first_column, true);
        for (normal, normal_sum_length) in export.normals[base_index..]
            .iter_mut()
            .zip(&mut export.normal_sum_lengths[base_index..])
        {
            *normal_sum_length = normal.length();
            *normal = if normal.length_squared() == 0.0 {
                Vector3::UP
            } else {
                normal.normalized()
            };
        }
    }

    // The patch-local vertices of the cell side at `fixed` (an x line when `x_line`) spanning
    // `start..end`, in increasing along order, and whether a window supplied them: a window
    // across the side supplies its exported side vertices, and any other side carries the
    // boundary lattice at the patch heights.
    fn regular_terrain_filler_side(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        windows: &[TerrainCdtWindowBounds],
        fixed: f32,
        (start, end): (f32, f32),
        x_line: bool,
        boundary_step_m: f32,
    ) -> (Vec<Vector3>, bool) {
        let (centre_x, centre_z) = patch.frame_centre();
        let lattice_point = |along: f32| {
            let (x, z) = if x_line {
                (fixed, along)
            } else {
                (along, fixed)
            };
            Vector3::new(
                patch_local(x, centre_x),
                Self::terrain_patch_height_at_world_m(patch, x, z),
                patch_local(z, centre_z),
            )
        };
        let (fixed_local, along_centre) = if x_line {
            (patch_local(fixed, centre_x), centre_z)
        } else {
            (patch_local(fixed, centre_z), centre_x)
        };
        let (start_local, end_local) = (
            patch_local(start, along_centre),
            patch_local(end, along_centre),
        );
        let split = |point: &Vector3| {
            if x_line {
                (point.x, point.z)
            } else {
                (point.z, point.x)
            }
        };
        let across = windows.iter().find(|window| {
            let (low, high, from, to) = if x_line {
                (window.min_x, window.max_x, window.min_z, window.max_z)
            } else {
                (window.min_z, window.max_z, window.min_x, window.max_x)
            };
            (low == fixed || high == fixed) && from <= start && to >= end
        });
        let points = match across {
            Some(window) => {
                let mut points = window
                    .side_vertices
                    .iter()
                    .copied()
                    .filter(|point| {
                        let (on, along) = split(point);
                        on.to_bits() == fixed_local.to_bits()
                            && along >= start_local
                            && along <= end_local
                    })
                    .collect::<Vec<_>>();
                points.sort_by(|a, b| split(a).1.total_cmp(&split(b).1));
                points.dedup_by(|a, b| split(a).1.to_bits() == split(b).1.to_bits());
                // A window always has its corners; the cell's ends are window corners.
                if points
                    .first()
                    .is_none_or(|point| split(point).1 != start_local)
                {
                    points.insert(0, lattice_point(start));
                }
                if points
                    .last()
                    .is_none_or(|point| split(point).1 != end_local)
                {
                    points.push(lattice_point(end));
                }
                points
            }
            None => Self::terrain_cdt_axis_samples(start, end, boundary_step_m)
                .into_iter()
                .map(lattice_point)
                .collect(),
        };
        (points, across.is_some())
    }

    fn regular_terrain_filler_vertex(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        indices: &mut HashMap<(u32, u32), usize>,
        point: Vector3,
    ) -> usize {
        *indices
            .entry((point.x.to_bits(), point.z.to_bits()))
            .or_insert_with(|| {
                let uv = |local: f32, size: f32| {
                    ((f64::from(local) / f64::from(size.max(0.001))) + 0.5).clamp(0.0, 1.0) as f32
                };
                export.vertices.push(point);
                export.normals.push(Vector3::ZERO);
                export.normal_sum_lengths.push(0.0);
                export.uvs.push(Vector2::new(
                    uv(point.x, patch.world_size_x),
                    uv(point.z, patch.world_size_z),
                ));
                export.vertices.len() - 1
            })
    }

    // Joins a cell side (`outer`) to the parallel grid row inside it (`inner`), both in
    // increasing along order, with their first and last vertices joined by the neighbouring
    // strips. Each triangle takes two consecutive vertices of one row and one of the other,
    // advancing by along position, so the strip has no gap and no overlap.
    fn append_regular_terrain_strip(
        export: &mut TerrainCdtTriangleBufferExport,
        outer: &[usize],
        inner: &[usize],
        along_z: bool,
    ) {
        let along = |export: &TerrainCdtTriangleBufferExport, index: usize| {
            let point = export.vertices[index];
            if along_z { point.z } else { point.x }
        };
        let (mut i, mut j) = (0, 0);
        while i + 1 < outer.len() || j + 1 < inner.len() {
            let advance_outer = j + 1 == inner.len()
                || (i + 1 < outer.len()
                    && along(export, outer[i + 1]) <= along(export, inner[j + 1]));
            if advance_outer {
                Self::append_regular_terrain_triangle(export, [outer[i], outer[i + 1], inner[j]]);
                i += 1;
            } else {
                Self::append_regular_terrain_triangle(export, [outer[i], inner[j + 1], inner[j]]);
                j += 1;
            }
        }
    }

    /// The window's patch-clipped bounds and its exported (patch-local) terrain vertices on
    /// its four sides.
    pub(in crate::nodes::simulation_node) fn terrain_cdt_window_bounds(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_patch: TerrainCdtPatch,
        exported_vertices: &[Vector3],
    ) -> Option<TerrainCdtWindowBounds> {
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        let min_x = (cdt_patch.min_x as f32).clamp(patch_min_x, patch_max_x);
        let min_z = (cdt_patch.min_z as f32).clamp(patch_min_z, patch_max_z);
        let max_x = (cdt_patch.max_x as f32).clamp(patch_min_x, patch_max_x);
        let max_z = (cdt_patch.max_z as f32).clamp(patch_min_z, patch_max_z);
        if max_x <= min_x || max_z <= min_z {
            return None;
        }
        let (centre_x, centre_z) = patch.frame_centre();
        let xs = [min_x, max_x].map(|x| patch_local(x, centre_x).to_bits());
        let zs = [min_z, max_z].map(|z| patch_local(z, centre_z).to_bits());
        let side_vertices = exported_vertices
            .iter()
            .copied()
            .filter(|point| xs.contains(&point.x.to_bits()) || zs.contains(&point.z.to_bits()))
            .collect();
        Some(TerrainCdtWindowBounds {
            min_x,
            min_z,
            max_x,
            max_z,
            side_vertices,
        })
    }

    fn sort_dedup_axis_lines(values: &mut Vec<f32>) {
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
                export.normal_sum_lengths.push(0.0);
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

        for (normal, normal_sum_length) in export.normals[base_index..]
            .iter_mut()
            .zip(&mut export.normal_sum_lengths[base_index..])
        {
            *normal_sum_length = normal.length();
            if normal.length_squared() <= 0.000_001 {
                *normal = Vector3::UP;
            } else {
                *normal = normal.normalized();
            }
        }
    }

    // Emits one filler face with an upward cross product, as CDT faces have. Only an exactly
    // degenerate face is skipped: it covers nothing.
    pub(in crate::nodes::simulation_node) fn append_regular_terrain_triangle(
        export: &mut TerrainCdtTriangleBufferExport,
        mut triangle: [usize; 3],
    ) {
        let mut points = triangle.map(|index| export.vertices[index]);
        let mut normal = (points[1] - points[0]).cross(points[2] - points[0]);
        if normal.length_squared() == 0.0 {
            return;
        }
        if normal.y < 0.0 {
            triangle.swap(1, 2);
            points.swap(1, 2);
            normal = -normal;
        }
        let normal = normal.normalized();
        let (face_y_delta_m, face_slope_ratio, face_longest_edge_m) =
            Self::terrain_buffer_triangle_metrics(points);
        export.max_face_y_delta_m = export.max_face_y_delta_m.max(face_y_delta_m);
        export.max_face_slope_ratio = export.max_face_slope_ratio.max(face_slope_ratio);
        export.longest_triangle_edge_m = export.longest_triangle_edge_m.max(face_longest_edge_m);
        for index in triangle {
            export.normals[index] += normal;
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
        if horizontal_normal == 0.0 {
            return 0.0;
        }
        if normal.y == 0.0 {
            return 1_000_000.0;
        }
        horizontal_normal / normal.y.abs()
    }
}
