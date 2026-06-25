//! Water patch mesh generation and cache helpers for the Godot renderer.

use crate::nodes::sim::core::SimCore;
use crate::simulation::terrain::cdt::TerrainCdtRoadLoop;
use crate::simulation::water::WaterPatchSnapshot;
use godot::prelude::{Vector2, Vector3};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

const WATER_MIN_VISIBLE_DEPTH_M: f32 = 0.001;
const WATER_POINT_EPSILON: f32 = 0.000001;
const WATER_SEGMENT_EPSILON: f32 = 0.00001;

/// Cache key for one static water mesh variant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WaterPatchMeshCacheKey {
    /// Water render-patch X index.
    pub(crate) patch_x: usize,
    /// Water render-patch Z index.
    pub(crate) patch_z: usize,
    /// Mesh LOD sample step used for this mesh.
    pub(crate) lod_step: usize,
    /// Stable signature of road/building clip loops that affect this patch.
    pub(crate) road_clip_signature: i64,
    /// Stable signature of visible water depth samples for this patch.
    pub(crate) depth_signature: u64,
}

/// CPU-side water mesh buffers ready for Godot `ArrayMesh` upload.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CachedWaterPatchMesh {
    /// Cache key this mesh was built from.
    pub(crate) key: WaterPatchMeshCacheKey,
    /// Local patch-space vertex positions.
    pub(crate) vertices: Vec<Vector3>,
    /// Per-vertex normals.
    pub(crate) normals: Vec<Vector3>,
    /// Per-vertex patch UVs.
    pub(crate) uvs: Vec<Vector2>,
    /// Optional triangle indices. Empty means the vertex buffer is already expanded.
    pub(crate) indices: Vec<i32>,
    /// Local X span covered by this patch mesh.
    pub(crate) world_size_x: f32,
    /// Local Z span covered by this patch mesh.
    pub(crate) world_size_z: f32,
    /// Mesh generation counters used by water debug output.
    pub(crate) stats: WaterPatchMeshStats,
}

/// Mesh generation counters used by renderer debug output.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct WaterPatchMeshStats {
    /// Mesh LOD sample step.
    pub(crate) lod_step: usize,
    /// Number of mesh cells visited.
    pub(crate) cells_total: usize,
    /// Cells with all four corners wet.
    pub(crate) full_cells: usize,
    /// Cells clipped by a wet/dry edge.
    pub(crate) partial_cells: usize,
    /// Coarse LOD cells with wet interior samples but dry corners.
    pub(crate) conservative_cells: usize,
    /// Cells skipped because they contained no visible water.
    pub(crate) dry_cells: usize,
    /// Cells removed by road/building clipping.
    pub(crate) road_clipped_cells: usize,
    /// Number of emitted vertices.
    pub(crate) emitted_vertices: usize,
    /// Number of emitted triangles.
    pub(crate) emitted_triangles: usize,
}

/// Complete input needed to build a water mesh off the Godot script path.
pub(crate) struct WaterPatchMeshBuildInput {
    /// Cache key for the produced mesh.
    pub(crate) key: WaterPatchMeshCacheKey,
    /// Visible water depth patch.
    pub(crate) patch: WaterPatchSnapshot,
    /// Road/building clip loops overlapping this patch.
    pub(crate) road_clip_loops: Vec<TerrainCdtRoadLoop>,
    /// Terrain clip setup error, if road-boundary collection failed.
    pub(crate) clip_failed: bool,
}

#[derive(Clone, Copy)]
struct Rect {
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
}

impl Rect {
    fn from_points(points: &[Vector2]) -> Self {
        if points.is_empty() {
            return Self {
                min_x: 0.0,
                min_z: 0.0,
                max_x: 0.0,
                max_z: 0.0,
            };
        }
        let mut bounds = Self {
            min_x: points[0].x,
            min_z: points[0].y,
            max_x: points[0].x,
            max_z: points[0].y,
        };
        for point in points {
            bounds.min_x = bounds.min_x.min(point.x);
            bounds.min_z = bounds.min_z.min(point.y);
            bounds.max_x = bounds.max_x.max(point.x);
            bounds.max_z = bounds.max_z.max(point.y);
        }
        bounds
    }

    fn merge(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_z: self.min_z.min(other.min_z),
            max_x: self.max_x.max(other.max_x),
            max_z: self.max_z.max(other.max_z),
        }
    }

    fn intersects(self, other: Self) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_z <= other.max_z
            && self.max_z >= other.min_z
    }

    fn contains_point(self, point: Vector2) -> bool {
        const EPSILON: f32 = 0.01;
        point.x >= self.min_x - EPSILON
            && point.x <= self.max_x + EPSILON
            && point.y >= self.min_z - EPSILON
            && point.y <= self.max_z + EPSILON
    }
}

#[derive(Clone)]
struct ClipLoop {
    points: Vec<Vector2>,
    bounds: Rect,
}

#[derive(Clone)]
struct ClipGroup {
    outer_loops: Vec<ClipLoop>,
    hole_loops: Vec<ClipLoop>,
    bounds: Rect,
}

impl SimCore {
    /// Builds water mesh cache entries in parallel.
    pub(crate) fn build_water_patch_mesh_cache_entries(
        inputs: Vec<WaterPatchMeshBuildInput>,
    ) -> Vec<CachedWaterPatchMesh> {
        let mut entries = inputs
            .into_par_iter()
            .map(build_water_patch_mesh)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| (entry.key.patch_z, entry.key.patch_x, entry.key.lod_step));
        entries
    }

    /// Inserts completed water mesh cache entries.
    pub(crate) fn insert_water_patch_mesh_cache_entries(
        &mut self,
        entries: Vec<CachedWaterPatchMesh>,
    ) {
        for entry in entries {
            self.water_patch_mesh_cache.insert(entry.key, entry);
        }
    }

    /// Removes cached mesh variants for patches whose depth or road clip inputs changed.
    pub(crate) fn clear_water_patch_mesh_cache_entries(&mut self, patch_keys: &[(usize, usize)]) {
        if patch_keys.is_empty() {
            self.water_patch_mesh_cache.clear();
            return;
        }
        let patch_key_lookup = patch_keys.iter().copied().collect::<HashSet<_>>();
        self.water_patch_mesh_cache
            .retain(|key, _| !patch_key_lookup.contains(&(key.patch_x, key.patch_z)));
    }
}

/// Returns a stable depth signature for one patch snapshot.
pub(crate) fn water_patch_depth_signature(patch: &WaterPatchSnapshot) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    mix_signature(&mut hash, patch.sample_width as u64);
    mix_signature(&mut hash, patch.sample_height as u64);
    mix_signature(&mut hash, patch.texture_width as u64);
    mix_signature(&mut hash, patch.texture_height as u64);
    mix_signature(&mut hash, patch.inner_offset_x as u64);
    mix_signature(&mut hash, patch.inner_offset_z as u64);
    mix_signature(&mut hash, u64::from(patch.world_origin_x.to_bits()));
    mix_signature(&mut hash, u64::from(patch.world_origin_z.to_bits()));
    mix_signature(&mut hash, u64::from(patch.world_size_x.to_bits()));
    mix_signature(&mut hash, u64::from(patch.world_size_z.to_bits()));
    mix_signature(&mut hash, patch.depth_nonzero_count as u64);
    for depth in &patch.depth_data {
        mix_signature(&mut hash, u64::from(depth.to_bits()));
    }
    hash
}

fn mix_signature(hash: &mut u64, value: u64) {
    *hash ^= value;
    *hash = hash.wrapping_mul(0x100000001b3);
}

fn build_water_patch_mesh(input: WaterPatchMeshBuildInput) -> CachedWaterPatchMesh {
    if input.clip_failed || input.patch.depth_nonzero_count == 0 {
        return CachedWaterPatchMesh {
            key: input.key,
            vertices: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            world_size_x: input.patch.world_size_x,
            world_size_z: input.patch.world_size_z,
            stats: WaterPatchMeshStats {
                lod_step: input.key.lod_step,
                ..WaterPatchMeshStats::default()
            },
        };
    }

    let mut builder = WaterMeshBuilder::new(input.key, input.patch, input.road_clip_loops);
    builder.build()
}

struct WaterMeshBuilder {
    key: WaterPatchMeshCacheKey,
    patch: WaterPatchSnapshot,
    clip_groups: Vec<ClipGroup>,
    vertices: Vec<Vector3>,
    normals: Vec<Vector3>,
    uvs: Vec<Vector2>,
    stats: WaterPatchMeshStats,
}

impl WaterMeshBuilder {
    fn new(
        key: WaterPatchMeshCacheKey,
        patch: WaterPatchSnapshot,
        road_clip_loops: Vec<TerrainCdtRoadLoop>,
    ) -> Self {
        let lod_step = key.lod_step.max(1);
        let x_intervals = mesh_interval_count(patch.sample_width, lod_step);
        let z_intervals = mesh_interval_count(patch.sample_height, lod_step);
        let estimated_cells = x_intervals.saturating_mul(z_intervals);
        let estimated_vertices = estimated_cells.saturating_mul(6).min(65_536);
        Self {
            key,
            patch,
            clip_groups: clip_groups_from_road_loops(road_clip_loops),
            vertices: Vec::with_capacity(estimated_vertices),
            normals: Vec::with_capacity(estimated_vertices),
            uvs: Vec::with_capacity(estimated_vertices),
            stats: WaterPatchMeshStats {
                lod_step,
                ..WaterPatchMeshStats::default()
            },
        }
    }

    fn build(&mut self) -> CachedWaterPatchMesh {
        let lod_step = self.key.lod_step.max(1);
        let x_interval_count = mesh_interval_count(self.patch.sample_width, lod_step);
        let z_interval_count = mesh_interval_count(self.patch.sample_height, lod_step);
        if self.clip_groups.is_empty()
            && self.can_emit_indexed_full_grid(x_interval_count, z_interval_count)
        {
            return self.build_indexed_full_grid(x_interval_count, z_interval_count);
        }
        let clip_bins = road_clip_group_bins(
            &self.clip_groups,
            self.patch.world_origin_x,
            self.patch.world_origin_z,
            self.patch.world_size_x,
            self.patch.world_size_z,
            x_interval_count,
            z_interval_count,
        );
        let center_x = self.patch.world_origin_x + self.patch.world_size_x * 0.5;
        let center_z = self.patch.world_origin_z + self.patch.world_size_z * 0.5;

        for z_index in 0..z_interval_count {
            let z0 = z_index as f32 / z_interval_count as f32;
            let z1 = (z_index + 1) as f32 / z_interval_count as f32;
            let world_z0 = self.patch.world_origin_z + z0 * self.patch.world_size_z;
            let world_z1 = self.patch.world_origin_z + z1 * self.patch.world_size_z;
            for x_index in 0..x_interval_count {
                self.stats.cells_total += 1;
                let x0 = x_index as f32 / x_interval_count as f32;
                let x1 = (x_index + 1) as f32 / x_interval_count as f32;
                let world_x0 = self.patch.world_origin_x + x0 * self.patch.world_size_x;
                let world_x1 = self.patch.world_origin_x + x1 * self.patch.world_size_x;
                let cell = [
                    Vector2::new(world_x0, world_z0),
                    Vector2::new(world_x1, world_z0),
                    Vector2::new(world_x1, world_z1),
                    Vector2::new(world_x0, world_z1),
                ];
                let corner_depths = self.water_cell_corner_depths(
                    x_index,
                    z_index,
                    x_interval_count,
                    z_interval_count,
                );
                let wet_corner_count = wet_corner_count(corner_depths);
                let mut wet_polygons = Vec::new();
                if wet_corner_count == 4 {
                    self.stats.full_cells += 1;
                    wet_polygons.push(cell.to_vec());
                } else if wet_corner_count > 0 {
                    self.stats.partial_cells += 1;
                    wet_polygons.extend(wet_water_cell_polygons(
                        cell,
                        corner_depths,
                        wet_corner_count,
                    ));
                } else if self.water_cell_has_visible_depth(
                    x_index,
                    z_index,
                    x_interval_count,
                    z_interval_count,
                ) {
                    self.stats.conservative_cells += 1;
                    wet_polygons.push(cell.to_vec());
                } else {
                    self.stats.dry_cells += 1;
                    continue;
                }

                if wet_polygons.is_empty() {
                    self.stats.dry_cells += 1;
                    continue;
                }

                let bin_index = z_index * x_interval_count + x_index;
                let cell_clip_groups = clip_bins.get(&bin_index).map(Vec::as_slice).unwrap_or(&[]);
                for wet_polygon in wet_polygons {
                    if wet_polygon.len() < 3 {
                        continue;
                    }
                    let emitted = self.emit_clipped_water_cell(
                        &wet_polygon,
                        cell_clip_groups,
                        center_x,
                        center_z,
                    );
                    if emitted == 0 && !cell_clip_groups.is_empty() {
                        self.stats.road_clipped_cells += 1;
                    }
                }
            }
        }

        self.stats.emitted_vertices = self.vertices.len();
        self.stats.emitted_triangles = self.vertices.len() / 3;
        CachedWaterPatchMesh {
            key: self.key,
            vertices: std::mem::take(&mut self.vertices),
            normals: std::mem::take(&mut self.normals),
            uvs: std::mem::take(&mut self.uvs),
            indices: Vec::new(),
            world_size_x: self.patch.world_size_x,
            world_size_z: self.patch.world_size_z,
            stats: self.stats,
        }
    }

    fn can_emit_indexed_full_grid(&self, x_interval_count: usize, z_interval_count: usize) -> bool {
        if x_interval_count == 0 || z_interval_count == 0 {
            return false;
        }
        for z_index in 0..=z_interval_count {
            for x_index in 0..=x_interval_count {
                if self.water_depth_at_mesh_corner(
                    x_index,
                    z_index,
                    x_interval_count,
                    z_interval_count,
                ) <= WATER_MIN_VISIBLE_DEPTH_M
                {
                    return false;
                }
            }
        }
        true
    }

    fn build_indexed_full_grid(
        &mut self,
        x_interval_count: usize,
        z_interval_count: usize,
    ) -> CachedWaterPatchMesh {
        let vertex_cols = x_interval_count + 1;
        let vertex_rows = z_interval_count + 1;
        let vertex_count = vertex_cols.saturating_mul(vertex_rows);
        self.vertices.reserve(vertex_count);
        self.normals.reserve(vertex_count);
        self.uvs.reserve(vertex_count);
        let center_x = self.patch.world_origin_x + self.patch.world_size_x * 0.5;
        let center_z = self.patch.world_origin_z + self.patch.world_size_z * 0.5;
        for z_index in 0..vertex_rows {
            let z = z_index as f32 / z_interval_count as f32;
            let world_z = self.patch.world_origin_z + z * self.patch.world_size_z;
            for x_index in 0..vertex_cols {
                let x = x_index as f32 / x_interval_count as f32;
                let world_x = self.patch.world_origin_x + x * self.patch.world_size_x;
                self.add_water_vertex(Vector2::new(world_x, world_z), center_x, center_z);
            }
        }

        let mut indices = Vec::with_capacity(x_interval_count * z_interval_count * 6);
        for z_index in 0..z_interval_count {
            for x_index in 0..x_interval_count {
                let top_left = z_index * vertex_cols + x_index;
                let top_right = top_left + 1;
                let bottom_left = top_left + vertex_cols;
                let bottom_right = bottom_left + 1;
                indices.push(top_left as i32);
                indices.push(bottom_right as i32);
                indices.push(top_right as i32);
                indices.push(top_left as i32);
                indices.push(bottom_left as i32);
                indices.push(bottom_right as i32);
            }
        }

        self.stats.cells_total = x_interval_count * z_interval_count;
        self.stats.full_cells = self.stats.cells_total;
        self.stats.emitted_vertices = self.vertices.len();
        self.stats.emitted_triangles = indices.len() / 3;
        CachedWaterPatchMesh {
            key: self.key,
            vertices: std::mem::take(&mut self.vertices),
            normals: std::mem::take(&mut self.normals),
            uvs: std::mem::take(&mut self.uvs),
            indices,
            world_size_x: self.patch.world_size_x,
            world_size_z: self.patch.world_size_z,
            stats: self.stats,
        }
    }

    fn water_cell_corner_depths(
        &self,
        x_index: usize,
        z_index: usize,
        x_interval_count: usize,
        z_interval_count: usize,
    ) -> [f32; 4] {
        [
            self.water_depth_at_mesh_corner(x_index, z_index, x_interval_count, z_interval_count),
            self.water_depth_at_mesh_corner(
                x_index + 1,
                z_index,
                x_interval_count,
                z_interval_count,
            ),
            self.water_depth_at_mesh_corner(
                x_index + 1,
                z_index + 1,
                x_interval_count,
                z_interval_count,
            ),
            self.water_depth_at_mesh_corner(
                x_index,
                z_index + 1,
                x_interval_count,
                z_interval_count,
            ),
        ]
    }

    fn water_depth_at_mesh_corner(
        &self,
        mesh_x_index: usize,
        mesh_z_index: usize,
        x_interval_count: usize,
        z_interval_count: usize,
    ) -> f32 {
        if self.patch.depth_data.is_empty()
            || self.patch.sample_width == 0
            || self.patch.sample_height == 0
            || self.patch.texture_width == 0
            || self.patch.texture_height == 0
        {
            return 0.0;
        }
        let max_sample_x = self.patch.sample_width.saturating_sub(1);
        let max_sample_z = self.patch.sample_height.saturating_sub(1);
        let sample_x = ((mesh_x_index as f32 / x_interval_count.max(1) as f32)
            * max_sample_x as f32)
            .round()
            .clamp(0.0, max_sample_x as f32) as usize;
        let sample_z = ((mesh_z_index as f32 / z_interval_count.max(1) as f32)
            * max_sample_z as f32)
            .round()
            .clamp(0.0, max_sample_z as f32) as usize;
        let texture_x = (self.patch.inner_offset_x + sample_x).min(self.patch.texture_width - 1);
        let texture_z = (self.patch.inner_offset_z + sample_z).min(self.patch.texture_height - 1);
        let sample_index = texture_z * self.patch.texture_width + texture_x;
        self.patch
            .depth_data
            .get(sample_index)
            .copied()
            .unwrap_or(0.0)
    }

    fn water_cell_has_visible_depth(
        &self,
        x_index: usize,
        z_index: usize,
        x_interval_count: usize,
        z_interval_count: usize,
    ) -> bool {
        if self.patch.depth_data.is_empty()
            || self.patch.sample_width == 0
            || self.patch.sample_height == 0
            || self.patch.texture_width == 0
            || self.patch.texture_height == 0
        {
            return false;
        }
        let max_sample_x = self.patch.sample_width.saturating_sub(1);
        let max_sample_z = self.patch.sample_height.saturating_sub(1);
        let start_sample_x = ((x_index as f32 / x_interval_count.max(1) as f32)
            * max_sample_x as f32)
            .floor()
            .clamp(0.0, max_sample_x as f32) as usize;
        let end_sample_x = (((x_index + 1) as f32 / x_interval_count.max(1) as f32)
            * max_sample_x as f32)
            .ceil()
            .clamp(0.0, max_sample_x as f32) as usize;
        let start_sample_z = ((z_index as f32 / z_interval_count.max(1) as f32)
            * max_sample_z as f32)
            .floor()
            .clamp(0.0, max_sample_z as f32) as usize;
        let end_sample_z = (((z_index + 1) as f32 / z_interval_count.max(1) as f32)
            * max_sample_z as f32)
            .ceil()
            .clamp(0.0, max_sample_z as f32) as usize;
        for sample_z in start_sample_z..=end_sample_z {
            let texture_z =
                (self.patch.inner_offset_z + sample_z).min(self.patch.texture_height - 1);
            let row_offset = texture_z * self.patch.texture_width;
            for sample_x in start_sample_x..=end_sample_x {
                let texture_x =
                    (self.patch.inner_offset_x + sample_x).min(self.patch.texture_width - 1);
                let sample_index = row_offset + texture_x;
                if self
                    .patch
                    .depth_data
                    .get(sample_index)
                    .is_some_and(|depth| *depth > WATER_MIN_VISIBLE_DEPTH_M)
                {
                    return true;
                }
            }
        }
        false
    }

    fn emit_clipped_water_cell(
        &mut self,
        cell: &[Vector2],
        clip_group_indices: &[usize],
        center_x: f32,
        center_z: f32,
    ) -> usize {
        if clip_group_indices.is_empty() {
            return self.emit_unclipped_water_polygon(cell, center_x, center_z);
        }
        let cell_bounds = Rect::from_points(cell);
        for &clip_group_index in clip_group_indices {
            let Some(clip_group) = self.clip_groups.get(clip_group_index) else {
                continue;
            };
            if cell_touches_road_clip_group(cell, cell_bounds, clip_group) {
                return 0;
            }
        }
        self.emit_unclipped_water_polygon(cell, center_x, center_z)
    }

    fn emit_unclipped_water_polygon(
        &mut self,
        polygon: &[Vector2],
        center_x: f32,
        center_z: f32,
    ) -> usize {
        if polygon.len() < 3 {
            return 0;
        }
        let mut emitted_vertices = 0;
        for index in 1..polygon.len() - 1 {
            self.add_water_vertex(polygon[0], center_x, center_z);
            self.add_water_vertex(polygon[index + 1], center_x, center_z);
            self.add_water_vertex(polygon[index], center_x, center_z);
            emitted_vertices += 3;
        }
        emitted_vertices
    }

    fn add_water_vertex(&mut self, world_xz: Vector2, center_x: f32, center_z: f32) {
        let uv = Vector2::new(
            ((world_xz.x - self.patch.world_origin_x) / self.patch.world_size_x.max(0.001))
                .clamp(0.0, 1.0),
            ((world_xz.y - self.patch.world_origin_z) / self.patch.world_size_z.max(0.001))
                .clamp(0.0, 1.0),
        );
        self.vertices.push(Vector3::new(
            world_xz.x - center_x,
            0.0,
            world_xz.y - center_z,
        ));
        self.normals.push(Vector3::UP);
        self.uvs.push(uv);
    }
}

fn mesh_interval_count(sample_count: usize, lod_step: usize) -> usize {
    let interval_count = sample_count.saturating_sub(1);
    let lod_vertex_count = 2.max(interval_count.div_ceil(lod_step.max(1)) + 1);
    (lod_vertex_count - 1).max(1)
}

fn wet_corner_count(corner_depths: [f32; 4]) -> usize {
    corner_depths
        .iter()
        .filter(|depth| **depth > WATER_MIN_VISIBLE_DEPTH_M)
        .count()
}

fn wet_water_cell_polygons(
    cell: [Vector2; 4],
    corner_depths: [f32; 4],
    wet_corner_count: usize,
) -> Vec<Vec<Vector2>> {
    let mut polygons = Vec::new();
    if wet_corner_count == 2 && has_opposite_wet_corners(corner_depths) {
        for index in 0..4 {
            if corner_depths[index] <= WATER_MIN_VISIBLE_DEPTH_M {
                continue;
            }
            let previous_index = (index + 3) % 4;
            let next_index = (index + 1) % 4;
            let polygon = dedupe_adjacent_polygon_points(vec![
                cell[index],
                water_depth_edge_crossing(cell, corner_depths, index, next_index),
                water_depth_edge_crossing(cell, corner_depths, previous_index, index),
            ]);
            if polygon.len() >= 3 {
                polygons.push(polygon);
            }
        }
        return polygons;
    }
    let polygon = wet_water_cell_polygon(cell, corner_depths);
    if polygon.len() >= 3 {
        polygons.push(polygon);
    }
    polygons
}

fn has_opposite_wet_corners(corner_depths: [f32; 4]) -> bool {
    (corner_depths[0] > WATER_MIN_VISIBLE_DEPTH_M && corner_depths[2] > WATER_MIN_VISIBLE_DEPTH_M)
        || (corner_depths[1] > WATER_MIN_VISIBLE_DEPTH_M
            && corner_depths[3] > WATER_MIN_VISIBLE_DEPTH_M)
}

fn wet_water_cell_polygon(cell: [Vector2; 4], corner_depths: [f32; 4]) -> Vec<Vector2> {
    let mut polygon = Vec::with_capacity(6);
    for index in 0..4 {
        let next_index = (index + 1) % 4;
        let depth_a = corner_depths[index];
        let depth_b = corner_depths[next_index];
        let a_is_wet = depth_a > WATER_MIN_VISIBLE_DEPTH_M;
        let b_is_wet = depth_b > WATER_MIN_VISIBLE_DEPTH_M;
        if a_is_wet {
            polygon.push(cell[index]);
        }
        if a_is_wet != b_is_wet {
            polygon.push(water_depth_edge_crossing(
                cell,
                corner_depths,
                index,
                next_index,
            ));
        }
    }
    dedupe_adjacent_polygon_points(polygon)
}

fn water_depth_edge_crossing(
    cell: [Vector2; 4],
    corner_depths: [f32; 4],
    from_index: usize,
    to_index: usize,
) -> Vector2 {
    let depth_a = corner_depths[from_index];
    let depth_b = corner_depths[to_index];
    let denom = depth_b - depth_a;
    let mut t = 0.5;
    if denom.abs() > 0.000001 {
        t = ((WATER_MIN_VISIBLE_DEPTH_M - depth_a) / denom).clamp(0.0, 1.0);
    }
    cell[from_index].lerp(cell[to_index], t)
}

fn dedupe_adjacent_polygon_points(polygon: Vec<Vector2>) -> Vec<Vector2> {
    if polygon.len() <= 1 {
        return polygon;
    }
    let mut deduped = Vec::with_capacity(polygon.len());
    for point in polygon {
        if deduped
            .last()
            .is_none_or(|last: &Vector2| last.distance_squared_to(point) > WATER_POINT_EPSILON)
        {
            deduped.push(point);
        }
    }
    if deduped.len() > 1
        && deduped[0].distance_squared_to(*deduped.last().unwrap()) <= WATER_POINT_EPSILON
    {
        deduped.pop();
    }
    deduped
}

fn clip_groups_from_road_loops(road_clip_loops: Vec<TerrainCdtRoadLoop>) -> Vec<ClipGroup> {
    let mut groups_by_id = BTreeMap::<u64, ClipGroup>::new();
    for road_loop in road_clip_loops {
        if road_loop.vertices.len() < 3 {
            continue;
        }
        let points = road_loop
            .vertices
            .iter()
            .map(|vertex| Vector2::new(vertex.x as f32, vertex.z as f32))
            .collect::<Vec<_>>();
        let bounds = Rect::from_points(&points);
        let clip_loop = ClipLoop { points, bounds };
        let group = groups_by_id
            .entry(road_loop.footprint_group_id)
            .or_insert_with(|| ClipGroup {
                outer_loops: Vec::new(),
                hole_loops: Vec::new(),
                bounds,
            });
        group.bounds = group.bounds.merge(bounds);
        if road_loop.is_hole {
            group.hole_loops.push(clip_loop);
        } else {
            group.outer_loops.push(clip_loop);
        }
    }
    groups_by_id
        .into_values()
        .filter(|group| !group.outer_loops.is_empty())
        .collect()
}

fn road_clip_group_bins(
    clip_groups: &[ClipGroup],
    world_origin_x: f32,
    world_origin_z: f32,
    world_size_x: f32,
    world_size_z: f32,
    x_interval_count: usize,
    z_interval_count: usize,
) -> HashMap<usize, Vec<usize>> {
    let mut bins = HashMap::<usize, Vec<usize>>::new();
    if x_interval_count == 0 || z_interval_count == 0 {
        return bins;
    }
    let safe_world_size_x = world_size_x.max(0.001);
    let safe_world_size_z = world_size_z.max(0.001);
    for (group_index, clip_group) in clip_groups.iter().enumerate() {
        let min_x_index = (((clip_group.bounds.min_x - world_origin_x) / safe_world_size_x)
            * x_interval_count as f32)
            .floor()
            .clamp(0.0, (x_interval_count - 1) as f32) as usize;
        let max_x_index = (((clip_group.bounds.max_x - world_origin_x) / safe_world_size_x)
            * x_interval_count as f32)
            .floor()
            .clamp(0.0, (x_interval_count - 1) as f32) as usize;
        let min_z_index = (((clip_group.bounds.min_z - world_origin_z) / safe_world_size_z)
            * z_interval_count as f32)
            .floor()
            .clamp(0.0, (z_interval_count - 1) as f32) as usize;
        let max_z_index = (((clip_group.bounds.max_z - world_origin_z) / safe_world_size_z)
            * z_interval_count as f32)
            .floor()
            .clamp(0.0, (z_interval_count - 1) as f32) as usize;
        for z_index in min_z_index..=max_z_index {
            for x_index in min_x_index..=max_x_index {
                bins.entry(z_index * x_interval_count + x_index)
                    .or_default()
                    .push(group_index);
            }
        }
    }
    bins
}

fn cell_touches_road_clip_group(
    cell: &[Vector2],
    cell_bounds: Rect,
    clip_group: &ClipGroup,
) -> bool {
    if !cell_bounds.intersects(clip_group.bounds) {
        return false;
    }
    if cell_fully_inside_any_road_clip_hole(cell, clip_group) {
        return false;
    }
    for sample in cell_road_clip_samples(cell) {
        if point_in_road_clip_group(sample, clip_group) {
            return true;
        }
    }
    for outer in &clip_group.outer_loops {
        if cell_fully_inside_polygon(cell, &outer.points) {
            return true;
        }
        if polygon_intersects_cell(&outer.points, outer.bounds, cell, cell_bounds) {
            return true;
        }
    }
    false
}

fn cell_road_clip_samples(cell: &[Vector2]) -> Vec<Vector2> {
    let mut samples = Vec::with_capacity(cell.len() + 1);
    let mut centroid = Vector2::ZERO;
    for point in cell {
        samples.push(*point);
        centroid += *point;
    }
    if !cell.is_empty() {
        samples.push(centroid / cell.len() as f32);
    }
    samples
}

fn cell_fully_inside_any_road_clip_hole(cell: &[Vector2], clip_group: &ClipGroup) -> bool {
    clip_group
        .hole_loops
        .iter()
        .any(|hole| cell_fully_inside_polygon(cell, &hole.points))
}

fn point_in_road_clip_group(point: Vector2, clip_group: &ClipGroup) -> bool {
    let inside_outer = clip_group.outer_loops.iter().any(|outer| {
        point_in_polygon(point, &outer.points) || point_on_polygon_boundary(point, &outer.points)
    });
    if !inside_outer {
        return false;
    }
    for hole in &clip_group.hole_loops {
        if point_on_polygon_boundary(point, &hole.points) {
            return true;
        }
        if point_in_polygon(point, &hole.points) {
            return false;
        }
    }
    true
}

fn polygon_intersects_cell(
    polygon: &[Vector2],
    polygon_bounds: Rect,
    cell: &[Vector2],
    cell_bounds: Rect,
) -> bool {
    if !cell_bounds.intersects(polygon_bounds) {
        return false;
    }
    if cell.iter().any(|point| point_in_polygon(*point, polygon)) {
        return true;
    }
    if polygon
        .iter()
        .any(|point| cell_bounds.contains_point(*point))
    {
        return true;
    }
    for polygon_index in 0..polygon.len() {
        let polygon_a = polygon[polygon_index];
        let polygon_b = polygon[(polygon_index + 1) % polygon.len()];
        for cell_index in 0..cell.len() {
            let cell_a = cell[cell_index];
            let cell_b = cell[(cell_index + 1) % cell.len()];
            if segments_intersect(polygon_a, polygon_b, cell_a, cell_b) {
                return true;
            }
        }
    }
    false
}

fn cell_fully_inside_polygon(cell: &[Vector2], polygon: &[Vector2]) -> bool {
    cell.iter().all(|point| {
        point_in_polygon(*point, polygon) || point_on_polygon_boundary(*point, polygon)
    })
}

fn point_in_polygon(point: Vector2, polygon: &[Vector2]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for current in polygon {
        let crosses = (current.y > point.y) != (previous.y > point.y);
        if crosses {
            let x_intersection = (previous.x - current.x) * (point.y - current.y)
                / (previous.y - current.y)
                + current.x;
            if point.x < x_intersection {
                inside = !inside;
            }
        }
        previous = *current;
    }
    inside
}

fn point_on_polygon_boundary(point: Vector2, polygon: &[Vector2]) -> bool {
    if polygon.len() < 2 {
        return false;
    }
    (0..polygon.len())
        .any(|index| point_on_segment(point, polygon[index], polygon[(index + 1) % polygon.len()]))
}

fn segments_intersect(a: Vector2, b: Vector2, c: Vector2, d: Vector2) -> bool {
    let ab = b - a;
    let cd = d - c;
    let denom = cross_2d(ab, cd);
    let ca = c - a;
    if denom.abs() <= WATER_SEGMENT_EPSILON {
        return point_on_segment(c, a, b)
            || point_on_segment(d, a, b)
            || point_on_segment(a, c, d)
            || point_on_segment(b, c, d);
    }
    let t = cross_2d(ca, cd) / denom;
    let u = cross_2d(ca, ab) / denom;
    t >= -WATER_SEGMENT_EPSILON
        && t <= 1.0 + WATER_SEGMENT_EPSILON
        && u >= -WATER_SEGMENT_EPSILON
        && u <= 1.0 + WATER_SEGMENT_EPSILON
}

fn point_on_segment(point: Vector2, a: Vector2, b: Vector2) -> bool {
    let ab = b - a;
    let ap = point - a;
    if cross_2d(ab, ap).abs() > WATER_SEGMENT_EPSILON {
        return false;
    }
    let dot = ap.dot(ab);
    if dot < -WATER_SEGMENT_EPSILON {
        return false;
    }
    dot <= ab.length_squared() + WATER_SEGMENT_EPSILON
}

fn cross_2d(a: Vector2, b: Vector2) -> f32 {
    a.x * b.y - a.y * b.x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_patch(depth_data: Vec<f32>, depth_nonzero_count: usize) -> WaterPatchSnapshot {
        WaterPatchSnapshot {
            patch_x: 2,
            patch_z: 3,
            sample_width: 3,
            sample_height: 3,
            texture_width: 5,
            texture_height: 5,
            inner_offset_x: 1,
            inner_offset_z: 1,
            world_origin_x: 100.0,
            world_origin_z: 200.0,
            world_size_x: 20.0,
            world_size_z: 20.0,
            depth_data,
            depth_nonzero_count,
        }
    }

    fn test_key(depth_signature: u64) -> WaterPatchMeshCacheKey {
        WaterPatchMeshCacheKey {
            patch_x: 2,
            patch_z: 3,
            lod_step: 1,
            road_clip_signature: 0,
            depth_signature,
        }
    }

    #[test]
    fn wet_patch_mesh_emits_indexed_full_grid() {
        let patch = test_patch(vec![1.0; 25], 25);
        let key = test_key(water_patch_depth_signature(&patch));
        let [mesh] =
            SimCore::build_water_patch_mesh_cache_entries(vec![WaterPatchMeshBuildInput {
                key,
                patch,
                road_clip_loops: Vec::new(),
                clip_failed: false,
            }])
            .try_into()
            .expect("one water patch mesh should be built");

        assert_eq!(mesh.key, key);
        assert_eq!(mesh.vertices.len(), 9);
        assert_eq!(mesh.normals.len(), mesh.vertices.len());
        assert_eq!(mesh.uvs.len(), mesh.vertices.len());
        assert_eq!(mesh.indices.len(), 24);
        assert_eq!(mesh.stats.cells_total, 4);
        assert_eq!(mesh.stats.full_cells, 4);
        assert_eq!(mesh.stats.emitted_vertices, 9);
        assert_eq!(mesh.stats.emitted_triangles, 8);
    }

    #[test]
    fn dry_patch_mesh_returns_empty_cache_entry() {
        let patch = test_patch(vec![0.0; 25], 0);
        let key = test_key(water_patch_depth_signature(&patch));
        let [mesh] =
            SimCore::build_water_patch_mesh_cache_entries(vec![WaterPatchMeshBuildInput {
                key,
                patch,
                road_clip_loops: Vec::new(),
                clip_failed: false,
            }])
            .try_into()
            .expect("one dry water patch cache entry should be built");

        assert_eq!(mesh.key, key);
        assert!(mesh.vertices.is_empty());
        assert!(mesh.normals.is_empty());
        assert!(mesh.uvs.is_empty());
        assert_eq!(mesh.stats.lod_step, 1);
        assert_eq!(mesh.stats.emitted_triangles, 0);
    }
}
