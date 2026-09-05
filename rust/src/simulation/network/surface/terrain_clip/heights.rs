// SPDX-License-Identifier: GPL-2.0-only

//! Terrain-clip cutter height recovery and interval coverage.

use super::super::backend::RoadVec3;
use super::super::{
    NodeOverlayPoint, RoadSurfaceSystem,
    keys::{SURFACE_MM_PER_M, SurfaceHeightMmKey},
};
use super::geometry::{interpolate_height_f64, interpolate_overlay_point};
use super::model::{
    TerrainClipPreparedSource, TerrainClipSegmentPointRecovery, TerrainClipSourceEdge,
    TerrainClipSourceInterval,
};

/// Height-key tolerance used only for numeric-dust terrain-clip connector ties.
pub(super) const TERRAIN_CLIP_DUST_HEIGHT_TIE_TOLERANCE_MM: u64 = 1;

impl RoadSurfaceSystem {
    pub(super) fn terrain_clip_top_envelope_points_from_prepared_sources(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        sources: &[TerrainClipPreparedSource],
    ) -> TerrainClipSegmentPointRecovery {
        if sources.is_empty() {
            return TerrainClipSegmentPointRecovery::Missing;
        }

        let mut breakpoints = Self::terrain_clip_interval_breakpoints(sources);
        Self::append_terrain_clip_height_crossings(sources, &mut breakpoints);
        breakpoints.sort_by(|a, b| a.total_cmp(b));
        breakpoints.dedup_by(|a, b| *a == *b);

        // Spec-scoped exception: this is the final terrain-removal cutter
        // top envelope. It is not used for output owner selection or dust
        // height recovery, which reject ambiguous provenance/height inputs.
        let mut heights = vec![None; breakpoints.len()];
        let mut covered_any = false;
        for index in 0..breakpoints.len().saturating_sub(1) {
            let start_t = breakpoints[index];
            let end_t = breakpoints[index + 1];
            if end_t <= start_t {
                continue;
            }

            let Some((start_y, end_y)) =
                Self::terrain_clip_top_envelope_heights(sources, start_t, end_t)
            else {
                return TerrainClipSegmentPointRecovery::Partial;
            };
            covered_any = true;
            Self::merge_terrain_clip_top_envelope_height(&mut heights[index], start_y);
            Self::merge_terrain_clip_top_envelope_height(&mut heights[index + 1], end_y);
        }
        if !covered_any {
            return TerrainClipSegmentPointRecovery::Missing;
        }

        let mut points = Vec::new();
        for (t, height) in breakpoints.into_iter().zip(heights) {
            let Some(height) = height else {
                continue;
            };
            let point = interpolate_overlay_point(start, end, t);
            points.push(RoadVec3::new(point[0], height, point[1]));
        }
        Self::dedup_terrain_clip_top_envelope_points(&mut points);
        if points.len() >= 2 {
            TerrainClipSegmentPointRecovery::Covered(points)
        } else {
            TerrainClipSegmentPointRecovery::Degenerate
        }
    }

    fn terrain_clip_interval_breakpoints(sources: &[TerrainClipPreparedSource]) -> Vec<f64> {
        let mut breakpoints = Vec::with_capacity(sources.len() * 2 + 2);
        breakpoints.push(0.0);
        breakpoints.push(1.0);
        for source in sources {
            let interval = source.interval;
            breakpoints.push(interval.start_t.clamp(0.0, 1.0));
            breakpoints.push(interval.end_t.clamp(0.0, 1.0));
        }
        breakpoints
    }

    fn append_terrain_clip_height_crossings(
        sources: &[TerrainClipPreparedSource],
        breakpoints: &mut Vec<f64>,
    ) {
        for first_index in 0..sources.len() {
            for second_index in first_index + 1..sources.len() {
                let first = sources[first_index].interval;
                let second = sources[second_index].interval;
                let start_t = first.start_t.max(second.start_t).max(0.0);
                let end_t = first.end_t.min(second.end_t).min(1.0);
                if end_t <= start_t {
                    continue;
                }
                let start_delta =
                    interval_height_at(first, start_t) - interval_height_at(second, start_t);
                let end_delta =
                    interval_height_at(first, end_t) - interval_height_at(second, end_t);
                if Self::overlay_heights_equal(start_delta, 0.0) {
                    breakpoints.push(start_t);
                }
                if Self::overlay_heights_equal(end_delta, 0.0) {
                    breakpoints.push(end_t);
                }
                if start_delta.signum() == end_delta.signum() {
                    continue;
                }
                let denominator = start_delta - end_delta;
                if denominator == 0.0 {
                    continue;
                }
                let crossing_t = start_t + (end_t - start_t) * start_delta / denominator;
                if crossing_t > start_t && crossing_t < end_t {
                    breakpoints.push(crossing_t);
                }
            }
        }
    }

    pub(super) fn terrain_clip_interval_covers(
        interval: TerrainClipSourceInterval,
        start_t: f64,
        end_t: f64,
    ) -> bool {
        interval.start_t <= start_t && interval.end_t >= end_t
    }

    fn terrain_clip_top_envelope_heights(
        sources: &[TerrainClipPreparedSource],
        start_t: f64,
        end_t: f64,
    ) -> Option<(f64, f64)> {
        let mut heights: Option<(f64, f64)> = None;
        for source in sources {
            let interval = source.interval;
            if !Self::terrain_clip_interval_covers(interval, start_t, end_t) {
                continue;
            }
            let candidate_start = interval_height_at(interval, start_t);
            let candidate_end = interval_height_at(interval, end_t);
            heights = Some(heights.map_or(
                (candidate_start, candidate_end),
                |(current_start, current_end)| {
                    (
                        current_start.max(candidate_start),
                        current_end.max(candidate_end),
                    )
                },
            ));
        }
        heights
    }

    fn merge_terrain_clip_top_envelope_height(height: &mut Option<f64>, candidate: f64) {
        *height = Some(height.map_or(candidate, |current| current.max(candidate)));
    }

    pub(super) fn dedup_terrain_clip_top_envelope_points(points: &mut Vec<RoadVec3>) {
        if points.len() < 2 {
            return;
        }
        let mut write_index = 0;
        for read_index in 1..points.len() {
            let point = points[read_index];
            if Self::world_points_same_for_boundary(points[write_index], point) {
                if point.y > points[write_index].y {
                    points[write_index].y = point.y;
                }
                continue;
            }
            write_index += 1;
            points[write_index] = point;
        }
        points.truncate(write_index + 1);
    }

    pub(super) fn terrain_clip_unambiguous_overlay_point_height_from_source_edges(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<Option<f64>, String> {
        Self::terrain_clip_overlay_point_height_from_source_edges(point, source_edges, 0)
    }

    pub(super) fn terrain_clip_dust_overlay_point_height_from_source_edges(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<Option<f64>, String> {
        Self::terrain_clip_overlay_point_height_from_source_edges(
            point,
            source_edges,
            TERRAIN_CLIP_DUST_HEIGHT_TIE_TOLERANCE_MM,
        )
    }

    fn terrain_clip_overlay_point_height_from_source_edges(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
        quantization_tie_tolerance_mm: u64,
    ) -> Result<Option<f64>, String> {
        let mut height = None;
        let mut height_key: Option<SurfaceHeightMmKey> = None;
        for &source_edge in source_edges {
            let source_start = [source_edge.start.x, source_edge.start.z];
            let source_end = [source_edge.end.x, source_edge.end.z];
            let Some(t) = Self::overlay_segment_parameter(point, source_start, source_end) else {
                continue;
            };
            let candidate = interpolate_height_f64(source_edge.start.y, source_edge.end.y, t);
            let candidate_key = SurfaceHeightMmKey::from_m_f64(candidate);
            if let Some(current_key) = height_key {
                let current_mm = current_key.as_i64();
                let candidate_mm = candidate_key.as_i64();
                if current_mm != candidate_mm {
                    if current_mm.abs_diff(candidate_mm) <= quantization_tie_tolerance_mm {
                        if candidate_mm > current_mm {
                            height_key = Some(candidate_key);
                            height = Some(candidate_mm as f64 / SURFACE_MM_PER_M);
                        }
                        continue;
                    }
                    return Err(format!(
                        "conflicting_source_heights point=({:.6},{:.6}) current_mm={} candidate_mm={}",
                        point[0], point[1], current_mm, candidate_mm
                    ));
                }
            } else {
                height_key = Some(candidate_key);
                height = Some(candidate_key.as_i64() as f64 / SURFACE_MM_PER_M);
            }
        }
        Ok(height)
    }
}

pub(super) fn interval_height_at(interval: TerrainClipSourceInterval, t: f64) -> f64 {
    let span = interval.end_t - interval.start_t;
    if span == 0.0 {
        return interval.start_y;
    }
    interpolate_height_f64(
        interval.start_y,
        interval.end_y,
        (t - interval.start_t) / span,
    )
}
