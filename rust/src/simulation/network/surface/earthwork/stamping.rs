// SPDX-License-Identifier: GPL-2.0-only

//! Terrain visual-height stamping for road-owned earthwork support.

use super::super::{
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    SurfaceChunkKey,
    backend::{RoadVec2, RoadVec3},
};
use crate::config;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;

const EARTHWORK_MIN_TRIANGLE_DOUBLE_AREA_M2: f64 = 1.0e-8;
const EARTHWORK_MIN_TRIANGLE_ALTITUDE_M: f64 = 0.01;
const EARTHWORK_STAMP_TILE_CELLS: usize = 8;

/// One final visual-terrain sample write produced by a chunk-local earthwork stamp pass.
#[derive(Clone, Copy, Debug)]
pub(super) struct EarthworkStampWrite {
    pub(super) grid_x: usize,
    pub(super) grid_z: usize,
    pub(super) height_sample: f32,
}

/// Per-chunk output from the read-only earthwork stamp planner.
#[derive(Clone, Debug)]
pub(super) struct EarthworkChunkStampResult {
    pub(super) chunk: SurfaceChunkKey,
    pub(super) writes: Vec<EarthworkStampWrite>,
    pub(super) stats: EarthworkStampStats,
    pub(super) collect_ms: f64,
}

/// Counters for road-earthwork stamp performance diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct EarthworkStampStats {
    pub(super) chunks: usize,
    pub(super) chunks_with_cache: usize,
    pub(super) span_owners: usize,
    pub(super) node_owners: usize,
    pub(super) regions_visited: usize,
    pub(super) regions_stamped: usize,
    pub(super) triangles_visited: usize,
    pub(super) degenerate_triangles: usize,
    pub(super) valid_triangles: usize,
    pub(super) triangle_grid_cells_scanned: usize,
    pub(super) tile_triangle_refs: usize,
    pub(super) point_triangle_tests: usize,
    pub(super) candidate_inserts: usize,
    pub(super) candidate_replacements: usize,
    pub(super) final_unique_writes: usize,
}

impl EarthworkStampStats {
    pub(super) fn add_assign(&mut self, other: Self) {
        self.chunks += other.chunks;
        self.chunks_with_cache += other.chunks_with_cache;
        self.span_owners += other.span_owners;
        self.node_owners += other.node_owners;
        self.regions_visited += other.regions_visited;
        self.regions_stamped += other.regions_stamped;
        self.triangles_visited += other.triangles_visited;
        self.degenerate_triangles += other.degenerate_triangles;
        self.valid_triangles += other.valid_triangles;
        self.triangle_grid_cells_scanned += other.triangle_grid_cells_scanned;
        self.tile_triangle_refs += other.tile_triangle_refs;
        self.point_triangle_tests += other.point_triangle_tests;
        self.candidate_inserts += other.candidate_inserts;
        self.candidate_replacements += other.candidate_replacements;
        self.final_unique_writes += other.final_unique_writes;
    }
}

#[derive(Clone, Copy, Debug)]
struct EarthworkStampCandidate {
    distance_squared: f32,
    height_sample: f32,
}

#[derive(Clone, Copy, Debug)]
struct EarthworkStampTriangle {
    triangle: [RoadVec3; 3],
    triangle_xz: [RoadVec2; 3],
    area_xz: f64,
    height_offset_m: f32,
    grid_min_x: usize,
    grid_max_x: usize,
    grid_min_z: usize,
    grid_max_z: usize,
}

impl EarthworkStampTriangle {
    fn new(
        system: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
        chunk: SurfaceChunkKey,
        triangle: [RoadVec3; 3],
        conservative_margin_m: f32,
        height_offset_m: f32,
        stats: &mut EarthworkStampStats,
    ) -> Option<Self> {
        stats.triangles_visited += 1;
        let triangle_xz = [
            RoadVec2::new(triangle[0].x, triangle[0].z),
            RoadVec2::new(triangle[1].x, triangle[1].z),
            RoadVec2::new(triangle[2].x, triangle[2].z),
        ];
        let area_xz = (triangle_xz[1].x - triangle_xz[0].x) * (triangle_xz[2].y - triangle_xz[0].y)
            - (triangle_xz[1].y - triangle_xz[0].y) * (triangle_xz[2].x - triangle_xz[0].x);
        let edge_ab = triangle_xz[1] - triangle_xz[0];
        let edge_bc = triangle_xz[2] - triangle_xz[1];
        let edge_ca = triangle_xz[0] - triangle_xz[2];
        let max_edge_m = edge_ab.length().max(edge_bc.length()).max(edge_ca.length());
        if area_xz.abs() <= EARTHWORK_MIN_TRIANGLE_DOUBLE_AREA_M2
            || area_xz.abs() / max_edge_m.max(f64::from(SAMPLE_EPSILON_M))
                < EARTHWORK_MIN_TRIANGLE_ALTITUDE_M
        {
            stats.degenerate_triangles += 1;
            return None;
        }

        let (chunk_min, chunk_max) = system.chunk_bounds(chunk);
        let min_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(chunk_max.x, f64::min)
            .max(chunk_min.x - f64::from(conservative_margin_m));
        let max_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(chunk_min.x, f64::max)
            .min(chunk_max.x + f64::from(conservative_margin_m));
        let min_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(chunk_max.z, f64::min)
            .max(chunk_min.z - f64::from(conservative_margin_m));
        let max_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(chunk_min.z, f64::max)
            .min(chunk_max.z + f64::from(conservative_margin_m));
        let Some((min_grid_x, max_grid_x, min_grid_z, max_grid_z)) = terrain
            .grid_rect_for_world_bounds(min_x as f32, min_z as f32, max_x as f32, max_z as f32)
        else {
            return None;
        };
        let (grid_width, grid_height) = terrain.grid_dimensions();
        if grid_width == 0 || grid_height == 0 {
            return None;
        }
        let max_grid_x_index = grid_width.saturating_sub(1);
        let max_grid_z_index = grid_height.saturating_sub(1);
        let grid_min_x = min_grid_x.saturating_sub(1).min(max_grid_x_index);
        let grid_max_x = max_grid_x.saturating_add(1).min(max_grid_x_index);
        let grid_min_z = min_grid_z.saturating_sub(1).min(max_grid_z_index);
        let grid_max_z = max_grid_z.saturating_add(1).min(max_grid_z_index);
        stats.valid_triangles += 1;
        stats.triangle_grid_cells_scanned +=
            (grid_max_x - grid_min_x + 1) * (grid_max_z - grid_min_z + 1);
        Some(Self {
            triangle,
            triangle_xz,
            area_xz,
            height_offset_m,
            grid_min_x,
            grid_max_x,
            grid_min_z,
            grid_max_z,
        })
    }

    fn contains_grid_sample(&self, grid_x: usize, grid_z: usize) -> bool {
        (self.grid_min_x..=self.grid_max_x).contains(&grid_x)
            && (self.grid_min_z..=self.grid_max_z).contains(&grid_z)
    }

    fn point_is_inside_or_near_xz(&self, point: RoadVec2, margin_m: f64) -> bool {
        if self.barycentric_weights_xz(point).is_some() {
            return true;
        }
        self.distance_point_to_triangle_xz(point) <= margin_m
    }

    fn candidate_from_point(&self, point_xz: RoadVec2) -> Option<EarthworkStampCandidate> {
        let sample_point_xz = self.closest_point_on_triangle_xz(point_xz);
        let (wa, wb, wc) = self.barycentric_weights_xz(sample_point_xz)?;
        let support_height_m =
            self.triangle[0].y * wa + self.triangle[1].y * wb + self.triangle[2].y * wc;
        let clearance_sample = ((support_height_m - f64::from(self.height_offset_m))
            / f64::from(config::HEIGHT_SCALE)) as f32;
        Some(EarthworkStampCandidate {
            distance_squared: point_xz.distance_squared(sample_point_xz) as f32,
            height_sample: clearance_sample,
        })
    }

    fn barycentric_weights_xz(&self, point: RoadVec2) -> Option<(f64, f64, f64)> {
        if self.area_xz.abs() <= f64::from(SAMPLE_EPSILON_M) {
            return None;
        }

        let w0 = ((self.triangle_xz[1].x - point.x) * (self.triangle_xz[2].y - point.y)
            - (self.triangle_xz[1].y - point.y) * (self.triangle_xz[2].x - point.x))
            / self.area_xz;
        let w1 = ((self.triangle_xz[2].x - point.x) * (self.triangle_xz[0].y - point.y)
            - (self.triangle_xz[2].y - point.y) * (self.triangle_xz[0].x - point.x))
            / self.area_xz;
        let w2 = 1.0 - w0 - w1;
        let epsilon = 0.001;
        if w0 < -epsilon || w1 < -epsilon || w2 < -epsilon {
            return None;
        }
        Some((w0, w1, w2))
    }

    fn closest_point_on_triangle_xz(&self, point: RoadVec2) -> RoadVec2 {
        if self.barycentric_weights_xz(point).is_some() {
            return point;
        }

        let mut best = self.triangle_xz[0];
        let mut best_distance_squared = point.distance_squared(best);
        for &(start, end) in &[
            (self.triangle_xz[0], self.triangle_xz[1]),
            (self.triangle_xz[1], self.triangle_xz[2]),
            (self.triangle_xz[2], self.triangle_xz[0]),
        ] {
            let candidate =
                RoadSurfaceSystem::earthwork_closest_point_on_segment_xz(point, start, end);
            let distance_squared = point.distance_squared(candidate);
            if distance_squared < best_distance_squared {
                best = candidate;
                best_distance_squared = distance_squared;
            }
        }

        best
    }

    fn distance_point_to_triangle_xz(&self, point: RoadVec2) -> f64 {
        RoadSurfaceSystem::earthwork_distance_point_to_segment_xz(
            point,
            self.triangle_xz[0],
            self.triangle_xz[1],
        )
        .min(RoadSurfaceSystem::earthwork_distance_point_to_segment_xz(
            point,
            self.triangle_xz[1],
            self.triangle_xz[2],
        ))
        .min(RoadSurfaceSystem::earthwork_distance_point_to_segment_xz(
            point,
            self.triangle_xz[2],
            self.triangle_xz[0],
        ))
    }
}

struct EarthworkChunkStampBuilder<'a> {
    system: &'a RoadSurfaceSystem,
    terrain: &'a TerrainSystem,
    chunk: SurfaceChunkKey,
    conservative_margin_m: f32,
    triangles: Vec<EarthworkStampTriangle>,
    stats: EarthworkStampStats,
}

impl<'a> EarthworkChunkStampBuilder<'a> {
    fn new(
        system: &'a RoadSurfaceSystem,
        terrain: &'a TerrainSystem,
        chunk: SurfaceChunkKey,
    ) -> Self {
        Self {
            system,
            terrain,
            chunk,
            conservative_margin_m: terrain.cell_size_m() * std::f32::consts::SQRT_2 * 0.5,
            triangles: Vec::new(),
            stats: EarthworkStampStats {
                chunks: 1,
                ..EarthworkStampStats::default()
            },
        }
    }

    fn mark_cache_present(&mut self) {
        self.stats.chunks_with_cache += 1;
    }

    fn collect_polygon(&mut self, polygon: &RoadSurfaceVisualPolygon, height_offset_m: f32) {
        self.stats.regions_visited += 1;
        self.stats.regions_stamped += 1;
        RoadSurfaceSystem::visit_visual_polygon_triangles(polygon, &mut |triangle| {
            if let Some(prepared) = EarthworkStampTriangle::new(
                self.system,
                self.terrain,
                self.chunk,
                triangle,
                self.conservative_margin_m,
                height_offset_m,
                &mut self.stats,
            ) {
                self.triangles.push(prepared);
            }
        });
    }

    fn skip_region(&mut self) {
        self.stats.regions_visited += 1;
    }

    fn finish(mut self) -> EarthworkChunkStampResult {
        if self.triangles.is_empty() {
            return EarthworkChunkStampResult {
                chunk: self.chunk,
                writes: Vec::new(),
                stats: self.stats,
                collect_ms: 0.0,
            };
        }

        let grid_min_x = self
            .triangles
            .iter()
            .map(|triangle| triangle.grid_min_x)
            .min()
            .unwrap_or(0);
        let grid_max_x = self
            .triangles
            .iter()
            .map(|triangle| triangle.grid_max_x)
            .max()
            .unwrap_or(0);
        let grid_min_z = self
            .triangles
            .iter()
            .map(|triangle| triangle.grid_min_z)
            .min()
            .unwrap_or(0);
        let grid_max_z = self
            .triangles
            .iter()
            .map(|triangle| triangle.grid_max_z)
            .max()
            .unwrap_or(0);

        let grid_width = grid_max_x - grid_min_x + 1;
        let grid_height = grid_max_z - grid_min_z + 1;
        let tile_cols = grid_width.div_ceil(EARTHWORK_STAMP_TILE_CELLS);
        let tile_rows = grid_height.div_ceil(EARTHWORK_STAMP_TILE_CELLS);
        let mut tile_buckets = vec![Vec::<usize>::new(); tile_cols * tile_rows];

        for (triangle_index, triangle) in self.triangles.iter().enumerate() {
            let tile_min_x = (triangle.grid_min_x - grid_min_x) / EARTHWORK_STAMP_TILE_CELLS;
            let tile_max_x = (triangle.grid_max_x - grid_min_x) / EARTHWORK_STAMP_TILE_CELLS;
            let tile_min_z = (triangle.grid_min_z - grid_min_z) / EARTHWORK_STAMP_TILE_CELLS;
            let tile_max_z = (triangle.grid_max_z - grid_min_z) / EARTHWORK_STAMP_TILE_CELLS;
            for tile_z in tile_min_z..=tile_max_z {
                for tile_x in tile_min_x..=tile_max_x {
                    tile_buckets[tile_z * tile_cols + tile_x].push(triangle_index);
                    self.stats.tile_triangle_refs += 1;
                }
            }
        }

        let mut candidates = vec![None::<EarthworkStampCandidate>; grid_width * grid_height];
        for tile_z in 0..tile_rows {
            let sample_min_z = grid_min_z + tile_z * EARTHWORK_STAMP_TILE_CELLS;
            let sample_max_z = (sample_min_z + EARTHWORK_STAMP_TILE_CELLS - 1).min(grid_max_z);
            for tile_x in 0..tile_cols {
                let bucket = &tile_buckets[tile_z * tile_cols + tile_x];
                if bucket.is_empty() {
                    continue;
                }
                let sample_min_x = grid_min_x + tile_x * EARTHWORK_STAMP_TILE_CELLS;
                let sample_max_x = (sample_min_x + EARTHWORK_STAMP_TILE_CELLS - 1).min(grid_max_x);
                for grid_z in sample_min_z..=sample_max_z {
                    for grid_x in sample_min_x..=sample_max_x {
                        let candidate_index =
                            (grid_z - grid_min_z) * grid_width + (grid_x - grid_min_x);
                        let (world_x, world_z) = self.terrain.grid_to_world_coords(grid_x, grid_z);
                        let point_xz = RoadVec2::new(f64::from(world_x), f64::from(world_z));
                        for &triangle_index in bucket {
                            let triangle = self.triangles[triangle_index];
                            if !triangle.contains_grid_sample(grid_x, grid_z) {
                                continue;
                            }
                            self.stats.point_triangle_tests += 1;
                            if !triangle.point_is_inside_or_near_xz(
                                point_xz,
                                f64::from(self.conservative_margin_m),
                            ) {
                                continue;
                            }
                            let Some(candidate) = triangle.candidate_from_point(point_xz) else {
                                continue;
                            };
                            Self::record_candidate(
                                &mut candidates[candidate_index],
                                candidate,
                                &mut self.stats,
                            );
                        }
                    }
                }
            }
        }

        let mut writes = Vec::new();
        let storage_chunk_size = self.terrain.storage_chunk_size_cells();
        let min_storage_chunk_x = grid_min_x / storage_chunk_size;
        let max_storage_chunk_x = grid_max_x / storage_chunk_size;
        let min_storage_chunk_z = grid_min_z / storage_chunk_size;
        let max_storage_chunk_z = grid_max_z / storage_chunk_size;
        for storage_chunk_z in min_storage_chunk_z..=max_storage_chunk_z {
            let chunk_min_z = grid_min_z.max(storage_chunk_z * storage_chunk_size);
            let chunk_max_z = grid_max_z.min((storage_chunk_z + 1) * storage_chunk_size - 1);
            for storage_chunk_x in min_storage_chunk_x..=max_storage_chunk_x {
                let chunk_min_x = grid_min_x.max(storage_chunk_x * storage_chunk_size);
                let chunk_max_x = grid_max_x.min((storage_chunk_x + 1) * storage_chunk_size - 1);
                for grid_z in chunk_min_z..=chunk_max_z {
                    for grid_x in chunk_min_x..=chunk_max_x {
                        let candidate_index =
                            (grid_z - grid_min_z) * grid_width + (grid_x - grid_min_x);
                        let Some(candidate) = candidates[candidate_index] else {
                            continue;
                        };
                        writes.push(EarthworkStampWrite {
                            grid_x,
                            grid_z,
                            height_sample: candidate.height_sample,
                        });
                    }
                }
            }
        }
        self.stats.final_unique_writes = writes.len();

        EarthworkChunkStampResult {
            chunk: self.chunk,
            writes,
            stats: self.stats,
            collect_ms: 0.0,
        }
    }

    fn record_candidate(
        current: &mut Option<EarthworkStampCandidate>,
        candidate: EarthworkStampCandidate,
        stats: &mut EarthworkStampStats,
    ) {
        let Some(existing) = *current else {
            *current = Some(candidate);
            stats.candidate_inserts += 1;
            return;
        };
        if RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (existing.distance_squared, existing.height_sample),
            (candidate.distance_squared, candidate.height_sample),
        ) {
            *current = Some(candidate);
            stats.candidate_replacements += 1;
        }
    }
}

impl RoadSurfaceSystem {
    pub(super) fn section_is_tunnel_surface_visible(
        &self,
        section: &RoadSurfaceSection,
        terrain: &TerrainSystem,
    ) -> bool {
        let terrain_height = terrain
            .sample_height_world(section.center_xz.x as f32, section.center_xz.y as f32)
            * config::HEIGHT_SCALE;
        section.center_height_m >= terrain_height - super::TUNNEL_PORTAL_STAMP_DEPTH_M
    }

    pub(in crate::simulation::network::surface) fn tunnel_throat_is_visible(
        &self,
        edge_idx: usize,
        at_start: bool,
        terrain: &TerrainSystem,
    ) -> bool {
        let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
            return false;
        };
        let mouth = if at_start {
            piece.start_mouth_profile.as_ref()
        } else {
            piece.end_mouth_profile.as_ref()
        };
        let Some(mouth) = mouth else {
            return false;
        };
        if mouth.boundary_points_world.len() < 2 {
            return false;
        }
        let Some(sections) = self.compiled_sections.get(&edge_idx) else {
            return false;
        };
        let section = if at_start {
            sections.first()
        } else {
            sections.last()
        };
        let Some(section) = section else {
            return false;
        };
        self.section_is_tunnel_surface_visible(section, terrain)
    }

    pub(super) fn collect_earthwork_chunk_stamp_writes(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        chunk: SurfaceChunkKey,
    ) -> EarthworkChunkStampResult {
        let mut builder = EarthworkChunkStampBuilder::new(self, terrain, chunk);
        let Some(entry) = self.earthwork_chunk_cache.get(&chunk) else {
            return builder.finish();
        };
        builder.mark_cache_present();

        for &edge_idx in &entry.edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            if !self.span_piece_uses_visible_earthwork(piece) {
                continue;
            }
            builder.stats.span_owners += 1;
            let height_offset_m = self.span_piece_integrated_surface_offset_m(piece);
            for region in piece.span_earthwork_support_regions.iter() {
                builder.collect_polygon(&region.polygon, height_offset_m);
            }
        }

        for &node_id in &entry.node_ids {
            if node_id as usize >= graph.node_count() {
                continue;
            }
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_earthworks(graph, node_id, terrain)
                || !self.node_piece_uses_visible_earthwork(graph, node_id, terrain)
            {
                continue;
            }
            builder.stats.node_owners += 1;
            let height_offset_m =
                self.node_piece_integrated_surface_offset_m(graph, node_id, terrain);
            for region in &piece.owned_regions {
                if self.node_earthwork_owner_uses_visible_earthwork(
                    graph,
                    terrain,
                    node_id,
                    piece,
                    region.kind,
                    region.owner_index,
                ) {
                    builder.collect_polygon(&region.polygon, height_offset_m);
                } else {
                    builder.skip_region();
                }
            }
        }

        builder.finish()
    }

    fn top_surface_support_candidate_replaces(existing: (f32, f32), candidate: (f32, f32)) -> bool {
        let (existing_distance_squared, existing_height_sample) = existing;
        let (candidate_distance_squared, candidate_height_sample) = candidate;
        candidate_distance_squared < existing_distance_squared - 0.0001
            || ((candidate_distance_squared - existing_distance_squared).abs() <= 0.0001
                && candidate_height_sample < existing_height_sample)
    }

    fn earthwork_distance_point_to_segment_xz(
        point: RoadVec2,
        start: RoadVec2,
        end: RoadVec2,
    ) -> f64 {
        point.distance(Self::earthwork_closest_point_on_segment_xz(
            point, start, end,
        ))
    }

    fn earthwork_closest_point_on_segment_xz(
        point: RoadVec2,
        start: RoadVec2,
        end: RoadVec2,
    ) -> RoadVec2 {
        let segment = end - start;
        let length_squared = segment.length_squared();
        if length_squared <= f64::from(SAMPLE_EPSILON_M) {
            return start;
        }
        let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
        start + segment * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earthwork_support_candidates_use_lower_envelope_for_overlapping_top_surfaces() {
        assert!(RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (0.0, 12.0),
            (0.0, 10.0),
        ));
        assert!(!RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (0.0, 10.0),
            (0.0, 12.0),
        ));
    }

    #[test]
    fn earthwork_support_candidates_prefer_smaller_distance_before_height() {
        assert!(RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (4.0, 1.0),
            (1.0, 10.0),
        ));
        assert!(!RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (1.0, 10.0),
            (4.0, 1.0),
        ));
    }

    #[test]
    fn earthwork_hardcut_has_no_per_material_sequential_stamping_path() {
        let source = include_str!("../earthwork.rs");
        for forbidden in [
            concat!("stamp_piece_surface_", "geometry_for_chunk"),
            concat!("profile_clearance_", "candidate_from_triangle"),
            concat!("collect_profile_clearance_", "triangle_candidates"),
        ] {
            assert!(
                !source.contains(forbidden),
                "road-touched terrain support must use one canonical lower-envelope pass, not `{forbidden}`"
            );
        }
    }
}
