//! Terrain-clip cutter height recovery and interval coverage.

use super::super::{NodeOverlayPoint, RoadSurfaceSystem, keys::SurfaceHeightMmKey};
use super::geometry::{interpolate_height_f64, interpolate_overlay_point};
use super::model::{
    TerrainClipSegmentPointRecovery, TerrainClipSourceEdge, TerrainClipSourceInterval,
};
use godot::prelude::Vector3;

impl RoadSurfaceSystem {
    pub(super) fn terrain_clip_top_envelope_points_from_interval_coverage<I>(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        intervals: I,
    ) -> TerrainClipSegmentPointRecovery
    where
        I: IntoIterator<Item = TerrainClipSourceInterval>,
    {
        let intervals = intervals.into_iter().collect::<Vec<_>>();
        if intervals.is_empty() {
            return TerrainClipSegmentPointRecovery::Missing;
        }

        let mut breakpoints = Self::terrain_clip_interval_breakpoints(&intervals);
        Self::append_terrain_clip_height_crossings(&intervals, &mut breakpoints);
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

            let covering = intervals
                .iter()
                .copied()
                .filter(|interval| Self::terrain_clip_interval_covers(*interval, start_t, end_t))
                .collect::<Vec<_>>();
            if covering.is_empty() {
                return TerrainClipSegmentPointRecovery::Partial;
            }
            covered_any = true;

            let Some(start_y) = Self::terrain_clip_top_envelope_height_at_t(&covering, start_t)
            else {
                return TerrainClipSegmentPointRecovery::Partial;
            };
            let Some(end_y) = Self::terrain_clip_top_envelope_height_at_t(&covering, end_t) else {
                return TerrainClipSegmentPointRecovery::Partial;
            };
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
            points.push(Vector3::new(point[0] as f32, height, point[1] as f32));
        }
        Self::dedup_terrain_clip_top_envelope_points(&mut points);
        if points.len() >= 2 {
            TerrainClipSegmentPointRecovery::Covered(points)
        } else {
            TerrainClipSegmentPointRecovery::Degenerate
        }
    }

    fn terrain_clip_interval_breakpoints(intervals: &[TerrainClipSourceInterval]) -> Vec<f64> {
        let mut breakpoints = Vec::with_capacity(intervals.len() * 2 + 2);
        breakpoints.push(0.0);
        breakpoints.push(1.0);
        for interval in intervals {
            breakpoints.push(interval.start_t.clamp(0.0, 1.0));
            breakpoints.push(interval.end_t.clamp(0.0, 1.0));
        }
        breakpoints
    }

    fn append_terrain_clip_height_crossings(
        intervals: &[TerrainClipSourceInterval],
        breakpoints: &mut Vec<f64>,
    ) {
        for first_index in 0..intervals.len() {
            for second_index in first_index + 1..intervals.len() {
                let first = intervals[first_index];
                let second = intervals[second_index];
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
                let denominator = f64::from(start_delta - end_delta);
                if denominator == 0.0 {
                    continue;
                }
                let crossing_t = start_t + (end_t - start_t) * f64::from(start_delta) / denominator;
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

    fn terrain_clip_top_envelope_height_at_t(
        intervals: &[TerrainClipSourceInterval],
        t: f64,
    ) -> Option<f32> {
        intervals
            .iter()
            .copied()
            .map(|interval| interval_height_at(interval, t))
            .max_by(|a, b| a.total_cmp(b))
    }

    fn merge_terrain_clip_top_envelope_height(height: &mut Option<f32>, candidate: f32) {
        *height = Some(height.map_or(candidate, |current| current.max(candidate)));
    }

    pub(super) fn dedup_terrain_clip_top_envelope_points(points: &mut Vec<Vector3>) {
        let mut deduped = Vec::with_capacity(points.len());
        for &point in points.iter() {
            if let Some(last) = deduped.last_mut() {
                if Self::world_points_same_for_boundary(*last, point) {
                    if point.y > last.y {
                        last.y = point.y;
                    }
                    continue;
                }
            }
            deduped.push(point);
        }
        *points = deduped;
    }

    pub(super) fn terrain_clip_unambiguous_overlay_point_height_from_source_edges(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<Option<f32>, String> {
        let mut height = None;
        let mut height_key: Option<SurfaceHeightMmKey> = None;
        for &source_edge in source_edges {
            let source_start = [
                f64::from(source_edge.start.x),
                f64::from(source_edge.start.z),
            ];
            let source_end = [f64::from(source_edge.end.x), f64::from(source_edge.end.z)];
            let Some(t) = Self::overlay_segment_parameter(point, source_start, source_end) else {
                continue;
            };
            let candidate = interpolate_height_f64(source_edge.start.y, source_edge.end.y, t);
            let candidate_key = SurfaceHeightMmKey::from_m_f32(candidate);
            if let Some(current_key) = height_key {
                if current_key != candidate_key {
                    return Err(format!(
                        "conflicting_source_heights point=({:.6},{:.6}) current_mm={} candidate_mm={}",
                        point[0],
                        point[1],
                        current_key.as_i64(),
                        candidate_key.as_i64()
                    ));
                }
            } else {
                height_key = Some(candidate_key);
                height = Some(candidate_key.as_i64() as f32 / 1000.0);
            }
        }
        Ok(height)
    }
}

pub(super) fn interval_height_at(interval: TerrainClipSourceInterval, t: f64) -> f32 {
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
