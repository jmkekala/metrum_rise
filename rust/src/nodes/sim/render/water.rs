// SPDX-License-Identifier: GPL-2.0-only

//! Water patch mesh generation and cache helpers for the Godot renderer.

use crate::nodes::sim::core::SimCore;
use crate::simulation::water::WaterPatchSnapshot;
use godot::prelude::{Vector2, Vector3};
use rayon::prelude::*;

const WATER_MIN_VISIBLE_DEPTH_M: f32 = 0.001;

/// Cache key for one static water mesh variant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WaterPatchMeshCacheKey {
    /// Water render-patch X index.
    pub(crate) patch_x: usize,
    /// Water render-patch Z index.
    pub(crate) patch_z: usize,
    /// Mesh LOD sample step used for this mesh.
    pub(crate) lod_step: usize,
    /// The patch's terrain payload generation: the roads and building sites that drop its
    /// cells (`docs/terrain.md` T32 (a)) change only with it.
    pub(crate) cover_generation: u64,
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
    /// Cells with water dropped because a road or a building site's footprint meets them.
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
    /// The mesh cells (row-major by `z`, at the key's LOD step) that a grounded road top
    /// triangle or a building site's support footprint meets
    /// ([`SimCore::water_cells_under_roads_and_pads`]); empty when none does.
    pub(crate) dropped: Vec<bool>,
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

    /// The cells of a water patch's mesh at `lod_step` (row-major by `z`) that a grounded road
    /// top triangle or a building site's support footprint meets, exactly
    /// ([`crate::simulation::terrain::graded::GradedSurface::mark_cells_met_by_roads_and_pads`],
    /// `docs/terrain.md` T32 (a)): water there would show over the road or the pad, so the mesh
    /// drops it. Empty when no cell is met. The site index must be ready. `O(T + S + C)` for the
    /// triangles and sites in the patch and the cells in their boxes.
    pub(crate) fn water_cells_under_roads_and_pads(
        &self,
        patch: &WaterPatchSnapshot,
        lod_step: usize,
    ) -> Vec<bool> {
        let lod_step = lod_step.max(1);
        let sides = |origin: f32, size: f32, count: usize| {
            (0..=count)
                .map(|index| f64::from(mesh_cell_side(origin, size, index, count)))
                .collect::<Vec<_>>()
        };
        let xs = sides(
            patch.world_origin_x,
            patch.world_size_x,
            mesh_interval_count(patch.sample_width, lod_step),
        );
        let zs = sides(
            patch.world_origin_z,
            patch.world_size_z,
            mesh_interval_count(patch.sample_height, lod_step),
        );
        let mut dropped = Vec::new();
        self.with_graded_surface(|surface, scratch| {
            surface.mark_cells_met_by_roads_and_pads(&xs, &zs, &mut dropped, scratch)
        });
        if !dropped.contains(&true) {
            dropped.clear();
        }
        dropped
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
    if input.patch.depth_nonzero_count == 0 {
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

    let mut builder = WaterMeshBuilder::new(input.key, input.patch, input.dropped);
    builder.build()
}

struct WaterMeshBuilder {
    key: WaterPatchMeshCacheKey,
    patch: WaterPatchSnapshot,
    dropped: Vec<bool>,
    vertices: Vec<Vector3>,
    normals: Vec<Vector3>,
    uvs: Vec<Vector2>,
    stats: WaterPatchMeshStats,
}

impl WaterMeshBuilder {
    fn new(key: WaterPatchMeshCacheKey, patch: WaterPatchSnapshot, dropped: Vec<bool>) -> Self {
        let lod_step = key.lod_step.max(1);
        let x_intervals = mesh_interval_count(patch.sample_width, lod_step);
        let z_intervals = mesh_interval_count(patch.sample_height, lod_step);
        let estimated_cells = x_intervals.saturating_mul(z_intervals);
        let estimated_vertices = estimated_cells.saturating_mul(6).min(65_536);
        Self {
            key,
            patch,
            dropped,
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
        if self.dropped.is_empty()
            && self.can_emit_indexed_full_grid(x_interval_count, z_interval_count)
        {
            return self.build_indexed_full_grid(x_interval_count, z_interval_count);
        }
        let center_x = self.patch.world_origin_x + self.patch.world_size_x * 0.5;
        let center_z = self.patch.world_origin_z + self.patch.world_size_z * 0.5;

        let (origin_x, size_x) = (self.patch.world_origin_x, self.patch.world_size_x);
        let (origin_z, size_z) = (self.patch.world_origin_z, self.patch.world_size_z);
        for z_index in 0..z_interval_count {
            let world_z0 = mesh_cell_side(origin_z, size_z, z_index, z_interval_count);
            let world_z1 = mesh_cell_side(origin_z, size_z, z_index + 1, z_interval_count);
            for x_index in 0..x_interval_count {
                self.stats.cells_total += 1;
                let world_x0 = mesh_cell_side(origin_x, size_x, x_index, x_interval_count);
                let world_x1 = mesh_cell_side(origin_x, size_x, x_index + 1, x_interval_count);
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
                // Any cell touching water is drawn whole. Its vertices sit at the water level and
                // the terrain hides the part above the shoreline, so the visible edge is the
                // per-pixel terrain intersection rather than a polygon cut across the cell.
                match wet_corner_count(corner_depths) {
                    4 => self.stats.full_cells += 1,
                    0 if self.water_cell_has_visible_depth(
                        x_index,
                        z_index,
                        x_interval_count,
                        z_interval_count,
                    ) =>
                    {
                        self.stats.conservative_cells += 1
                    }
                    0 => {
                        self.stats.dry_cells += 1;
                        continue;
                    }
                    _ => self.stats.partial_cells += 1,
                }

                if self
                    .dropped
                    .get(z_index * x_interval_count + x_index)
                    .is_some_and(|&dropped| dropped)
                {
                    self.stats.road_clipped_cells += 1;
                    continue;
                }
                self.emit_water_cell(&cell, center_x, center_z);
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
        let (origin_x, size_x) = (self.patch.world_origin_x, self.patch.world_size_x);
        let (origin_z, size_z) = (self.patch.world_origin_z, self.patch.world_size_z);
        for z_index in 0..vertex_rows {
            let world_z = mesh_cell_side(origin_z, size_z, z_index, z_interval_count);
            for x_index in 0..vertex_cols {
                let world_x = mesh_cell_side(origin_x, size_x, x_index, x_interval_count);
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

    // Two triangles of one cell's corners, unindexed.
    fn emit_water_cell(&mut self, cell: &[Vector2; 4], center_x: f32, center_z: f32) {
        for index in 1..cell.len() - 1 {
            self.add_water_vertex(cell[0], center_x, center_z);
            self.add_water_vertex(cell[index + 1], center_x, center_z);
            self.add_water_vertex(cell[index], center_x, center_z);
        }
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

// The world coordinate of side `index` of `count` mesh cells across a patch: one formula for
// the mesh's vertices and the cells the roads and pads drop.
fn mesh_cell_side(origin: f32, size: f32, index: usize, count: usize) -> f32 {
    origin + (index as f32 / count as f32) * size
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

// `TERRAIN-07-F` water cell digest (T32), a diagnostic compared across builds: with the whole
// world under 1 m of water, the mesh cells each patch drops at LOD steps 1, 2, 4 and 8, found
// from the built mesh (every cell is wet, so a cell the mesh does not emit is dropped). One
// `WATER_DROP <case> <patch_x> <patch_z> <lod> <count> <i,j>...` line per patch and step that
// drops any, then `WATER_CASE <case> meshes=<n> cells=<n> dropped=<n>`.
#[cfg(test)]
pub(crate) fn print_dropped_water_cells(case: &str, core: &mut SimCore) {
    let (width, height) = (core.watermap.width, core.watermap.height);
    core.watermap
        .replace_baseline_depth_from_dense(&vec![1.0; width * height])
        .expect("the flood fills every sample");
    let cell_m = core.config.zone_cell_m;
    core.allocator.prepare_building_site_query_index(cell_m);
    let (mut meshes, mut cells, mut dropped_total) = (0, 0, 0);
    for patch_z in 0..core.watermap.render_patch_rows() {
        for patch_x in 0..core.watermap.render_patch_cols() {
            let patch = core
                .watermap
                .visible_patch_snapshot(patch_x, patch_z)
                .expect("every patch of the grid has a snapshot");
            for lod_step in [1, 2, 4, 8] {
                let key = WaterPatchMeshCacheKey {
                    patch_x,
                    patch_z,
                    lod_step,
                    cover_generation: 0,
                    depth_signature: 0,
                };
                let dropped = core.water_cells_under_roads_and_pads(&patch, lod_step);
                let mesh = build_water_patch_mesh(WaterPatchMeshBuildInput {
                    key,
                    patch: patch.clone(),
                    dropped,
                });
                let (columns, rows) = (
                    mesh_interval_count(patch.sample_width, lod_step),
                    mesh_interval_count(patch.sample_height, lod_step),
                );
                let list = dropped_cells_of(&mesh, &patch, columns, rows);
                meshes += 1;
                cells += columns * rows;
                dropped_total += list.len();
                if !list.is_empty() {
                    let names: Vec<String> = list.iter().map(|(i, j)| format!("{i},{j}")).collect();
                    println!(
                        "WATER_DROP {case} {patch_x} {patch_z} {lod_step} {} {}",
                        list.len(),
                        names.join(" ")
                    );
                }
            }
        }
    }
    println!("WATER_CASE {case} meshes={meshes} cells={cells} dropped={dropped_total}");
}

// The cells of a flooded patch's mesh it does not emit: an indexed mesh is the full grid, an
// unindexed one emits each cell as six vertices led by its first corner.
#[cfg(test)]
fn dropped_cells_of(
    mesh: &CachedWaterPatchMesh,
    patch: &WaterPatchSnapshot,
    columns: usize,
    rows: usize,
) -> Vec<(usize, usize)> {
    if !mesh.indices.is_empty() {
        return Vec::new();
    }
    let center_x = patch.world_origin_x + patch.world_size_x * 0.5;
    let center_z = patch.world_origin_z + patch.world_size_z * 0.5;
    let mut first_corners = std::collections::HashMap::new();
    for j in 0..rows {
        for i in 0..columns {
            let x = mesh_cell_side(patch.world_origin_x, patch.world_size_x, i, columns) - center_x;
            let z = mesh_cell_side(patch.world_origin_z, patch.world_size_z, j, rows) - center_z;
            first_corners.insert((x.to_bits(), z.to_bits()), (i, j));
        }
    }
    let mut emitted = std::collections::HashSet::new();
    for vertex in mesh.vertices.chunks(6).map(|cell| cell[0]) {
        emitted.insert(first_corners[&(vertex.x.to_bits(), vertex.z.to_bits())]);
    }
    let mut dropped = Vec::new();
    for j in 0..rows {
        for i in 0..columns {
            if !emitted.contains(&(i, j)) {
                dropped.push((i, j));
            }
        }
    }
    dropped
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
            cover_generation: 0,
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
                dropped: Vec::new(),
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
    fn shoreline_cells_are_emitted_whole() {
        // One wet sample in the middle of a 3x3 patch: each of the four cells has one wet corner.
        let mut depth = vec![0.0; 25];
        depth[2 * 5 + 2] = 1.0;
        let patch = test_patch(depth, 1);
        let key = test_key(water_patch_depth_signature(&patch));
        let [mesh] =
            SimCore::build_water_patch_mesh_cache_entries(vec![WaterPatchMeshBuildInput {
                key,
                patch,
                dropped: Vec::new(),
            }])
            .try_into()
            .expect("one shoreline water patch mesh should be built");

        assert_eq!(mesh.stats.partial_cells, 4);
        assert_eq!(mesh.stats.emitted_triangles, 8);
        let max_x = mesh.vertices.iter().map(|v| v.x).fold(f32::MIN, f32::max);
        let min_x = mesh.vertices.iter().map(|v| v.x).fold(f32::MAX, f32::min);
        assert_eq!((min_x, max_x), (-10.0, 10.0));
    }

    #[test]
    fn dry_patch_mesh_returns_empty_cache_entry() {
        let patch = test_patch(vec![0.0; 25], 0);
        let key = test_key(water_patch_depth_signature(&patch));
        let [mesh] =
            SimCore::build_water_patch_mesh_cache_entries(vec![WaterPatchMeshBuildInput {
                key,
                patch,
                dropped: Vec::new(),
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
