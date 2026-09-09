// SPDX-License-Identifier: GPL-2.0-only

//! Road-owned earthwork generation, terrain stamping, and structural visibility rules.

use super::{ChunkCacheKind, RoadSurfaceCompileReason, RoadSurfaceSystem, SurfaceChunkKey};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use rayon::prelude::*;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Instant;

mod boundary;
mod geometry;
mod model;
mod planning;
mod ranges;
mod stamping;

pub(crate) use planning::RoadEarthworkPlan;
use stamping::{EarthworkChunkStampResult, EarthworkStampStats};

pub(crate) use model::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkGeometryError,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceEarthworkSupportPolicy,
};

// Vertical roadbed offset applied when terrain earthworks need pavement clearance.
pub(super) const EARTHWORK_PAVEMENT_DEPTH_M: f32 = 0.04;

// Lateral terrain probing envelope and sampling cadence for slopes.
const EARTHWORK_MIN_MARGIN_M: f32 = 4.0;
pub(super) const EARTHWORK_MAX_MARGIN_M: f32 = 18.0;
const EARTHWORK_MARGIN_SAMPLE_STEP_M: f32 = 1.0;

// Earthwork slope rates and retaining-wall classification threshold.
const EARTHWORK_CUT_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_FILL_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD: f32 = 1.25;

// Structural end caps that constrain tunnel portal stamps.
const TUNNEL_PORTAL_STAMP_DEPTH_M: f32 = 1.0;
const PARALLEL_EARTHWORK_CHUNK_MIN_ITEMS: usize = 2;

impl RoadSurfaceSystem {
    /// Offers immutable preview stamp products for the next staged earthwork publication only.
    pub(crate) fn enqueue_planned_earthworks(&mut self, plan: Option<Arc<RoadEarthworkPlan>>) {
        self.pending_planned_earthworks = plan;
    }

    /// Marks terrain render patches touched by dirty road chunks and local CDT grading support.
    pub(crate) fn mark_render_patches_for_chunk_grading_envelopes(
        &self,
        terrain: &mut TerrainSystem,
        chunks: &[SurfaceChunkKey],
        render_step_m: f32,
    ) -> Vec<(usize, usize)> {
        let dirty_patch_keys =
            self.render_patch_keys_for_chunk_grading_envelopes(terrain, chunks, render_step_m);
        for &(patch_x, patch_z) in &dirty_patch_keys {
            terrain.mark_render_patch_dirty(patch_x, patch_z);
        }
        dirty_patch_keys
    }

    pub(super) fn render_patch_keys_for_chunk_grading_envelopes(
        &self,
        terrain: &TerrainSystem,
        chunks: &[SurfaceChunkKey],
        render_step_m: f32,
    ) -> Vec<(usize, usize)> {
        if chunks.is_empty() {
            return Vec::new();
        }
        let grading_envelope_m = EARTHWORK_MAX_MARGIN_M
            + crate::simulation::terrain::terrain_cdt_local_sample_margin_m(terrain, render_step_m);
        let mut dirty_patch_keys = Vec::new();
        for &chunk in chunks {
            let (chunk_min, chunk_max) = self.chunk_bounds(chunk);
            dirty_patch_keys.extend(terrain.render_patch_keys_for_world_bounds(
                chunk_min.x as f32 - grading_envelope_m,
                chunk_min.z as f32 - grading_envelope_m,
                chunk_max.x as f32 + grading_envelope_m,
                chunk_max.z as f32 + grading_envelope_m,
            ));
        }

        dirty_patch_keys.sort_unstable();
        dirty_patch_keys.dedup();
        dirty_patch_keys
    }

    /// Rebuilds terrain earthworks only for the currently dirty road-surface chunks.
    pub fn rebuild_dirty_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
    ) -> Vec<SurfaceChunkKey> {
        self.rebuild_dirty_earthworks_with_reason(
            graph,
            terrain,
            RoadSurfaceCompileReason::TerrainEarthwork,
        )
    }

    pub(crate) fn rebuild_dirty_earthworks_with_reason(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
        reason: RoadSurfaceCompileReason,
    ) -> Vec<SurfaceChunkKey> {
        let had_dirty_work = self.has_pending_rebuild_work();
        self.compile_dirty_with_reason(graph, terrain, reason);
        if !self.published_generation_matches_source() {
            self.pending_planned_earthworks = None;
            self.last_reused_earthwork_chunk_count = 0;
            return Vec::new();
        }

        let chunks = if had_dirty_work {
            self.last_rebuilt_terrain_chunks.clone()
        } else {
            self.collect_all_chunks(ChunkCacheKind::Earthwork)
        };
        self.apply_earthwork_chunks(graph, terrain, &chunks);
        chunks
    }

    /// Rebuilds terrain earthworks for the whole world from the current compiled roadbed cache.
    pub fn rebuild_all_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
    ) -> Vec<SurfaceChunkKey> {
        self.pending_planned_earthworks = None;
        self.compile_dirty_with_reason(graph, terrain, RoadSurfaceCompileReason::TerrainEarthwork);
        if !self.published_generation_matches_source() {
            return Vec::new();
        }
        terrain.reset_visuals_from_source();
        let chunks = self.collect_all_chunks(ChunkCacheKind::Earthwork);
        self.apply_earthwork_chunks(graph, terrain, &chunks);
        chunks
    }

    fn apply_earthwork_chunks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
        chunks: &[SurfaceChunkKey],
    ) {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let collect_start = road_debug.then(Instant::now);
        let terrain_read: &TerrainSystem = terrain;
        let plan = self
            .pending_planned_earthworks
            .take()
            .filter(|plan| plan.dependencies_match(self, terrain_read));
        let collect_chunk = |chunk| {
            let chunk_start = road_debug.then(Instant::now);
            let input = self.prepare_earthwork_chunk_stamp(graph, terrain_read, chunk);
            let result = if let Some(result) = plan
                .as_ref()
                .and_then(|plan| plan.matching_chunk(chunk, &input))
            {
                Cow::Borrowed(result)
            } else {
                Cow::Owned(input.finish())
            };
            let collect_ms = chunk_start
                .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
            (result, collect_ms)
        };
        let stamp_results: Vec<(Cow<'_, EarthworkChunkStampResult>, f64)> =
            if chunks.len() >= PARALLEL_EARTHWORK_CHUNK_MIN_ITEMS {
                chunks.par_iter().copied().map(collect_chunk).collect()
            } else {
                chunks.iter().copied().map(collect_chunk).collect()
            };
        let collect_ms = collect_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let apply_start = road_debug.then(Instant::now);
        let mut stats = EarthworkStampStats::default();
        let mut chunk_collect_total_ms = 0.0;
        let mut chunk_collect_max_ms = 0.0_f64;
        let chunk_collect_count = stamp_results.len();
        self.last_reused_earthwork_chunk_count = 0;
        for (result, collect_ms) in stamp_results {
            self.last_reused_earthwork_chunk_count +=
                usize::from(matches!(result, Cow::Borrowed(_)));
            let chunk = result.chunk;
            chunk_collect_total_ms += collect_ms;
            chunk_collect_max_ms = chunk_collect_max_ms.max(collect_ms);
            let (chunk_min, chunk_max) = self.chunk_bounds(chunk);
            terrain.reset_visual_region_from_source_world(
                chunk_min.x as f32,
                chunk_min.z as f32,
                chunk_max.x as f32,
                chunk_max.z as f32,
            );

            terrain.set_visual_heights_at_grid_unmarked(&result.writes, |write| {
                (write.grid_x, write.grid_z, write.height_sample)
            });
            stats.add_assign(result.stats);
        }
        let apply_ms = apply_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        if road_debug {
            let chunk_collect_avg_ms = if chunk_collect_count == 0 {
                0.0
            } else {
                chunk_collect_total_ms / chunk_collect_count as f64
            };
            crate::debug_log!(
                "road",
                "road_edit_earthworks reused_chunks={} rebuilt_chunks={}",
                self.last_reused_earthwork_chunk_count,
                chunk_collect_count - self.last_reused_earthwork_chunk_count,
            );
            crate::debug_log!(
                "road",
                "earthwork_stamp_detail chunks={} chunks_with_cache={} span_owners={} node_owners={} regions_visited={} regions_stamped={} triangles_visited={} degenerate_triangles={} valid_triangles={} triangle_grid_cells_scanned={} tile_triangle_refs={} point_triangle_tests={} candidate_inserts={} candidate_replacements={} final_unique_writes={} chunk_collect_avg_ms={:.3} chunk_collect_max_ms={:.3} collect_ms={:.3} apply_ms={:.3} total_ms={:.3}",
                stats.chunks,
                stats.chunks_with_cache,
                stats.span_owners,
                stats.node_owners,
                stats.regions_visited,
                stats.regions_stamped,
                stats.triangles_visited,
                stats.degenerate_triangles,
                stats.valid_triangles,
                stats.triangle_grid_cells_scanned,
                stats.tile_triangle_refs,
                stats.point_triangle_tests,
                stats.candidate_inserts,
                stats.candidate_replacements,
                stats.final_unique_writes,
                chunk_collect_avg_ms,
                chunk_collect_max_ms,
                collect_ms,
                apply_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }
}
