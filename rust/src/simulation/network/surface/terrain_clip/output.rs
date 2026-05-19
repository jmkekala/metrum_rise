//! Terrain-clip output edge sourcing.

use super::super::{NodeOverlayPoint, RoadSurfaceSystem};
use super::model::*;
use super::union::interval_height_at;
use godot::prelude::Vector3;

impl RoadSurfaceSystem {
    pub(super) fn append_terrain_clip_sourced_segment_points(
        out: &mut Vec<RoadSurfaceTerrainClipSourceEdge>,
        mut points: Vec<Vector3>,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<(), (Vector3, Vector3)> {
        Self::dedup_terrain_clip_segment_points(&mut points);
        for segment in points.windows(2) {
            let start = segment[0];
            let end = segment[1];
            if Self::world_points_same_for_boundary(start, end) {
                continue;
            }
            let start_overlay = [f64::from(start.x), f64::from(start.z)];
            let end_overlay = [f64::from(end.x), f64::from(end.z)];
            let Some(source) = Self::terrain_clip_output_source_for_segment(
                start_overlay,
                end_overlay,
                source_edges,
            )
            .or_else(|| {
                Self::terrain_clip_output_source_for_endpoint_segment(
                    start_overlay,
                    end_overlay,
                    source_edges,
                )
            })
            .or_else(|| {
                Self::terrain_clip_output_dust_connector_source(
                    start_overlay,
                    end_overlay,
                    source_edges,
                )
            }) else {
                return Err((start, end));
            };
            Self::append_terrain_clip_source_edge(
                out,
                RoadSurfaceTerrainClipSourceEdge {
                    start,
                    end,
                    kind: source.kind,
                    source: source.source,
                },
            );
        }
        Ok(())
    }

    pub(super) fn close_terrain_clip_source_edges(edges: &mut [RoadSurfaceTerrainClipSourceEdge]) {
        if edges.len() < 2 {
            return;
        }
        let first_start = edges[0].start;
        let last_index = edges.len() - 1;
        let last_end = edges[last_index].end;
        if Self::world_points_same_for_boundary(first_start, last_end) {
            let shared = if last_end.y > first_start.y {
                last_end
            } else {
                first_start
            };
            edges[0].start = shared;
            edges[last_index].end = shared;
        }
    }

    fn append_terrain_clip_source_edge(
        out: &mut Vec<RoadSurfaceTerrainClipSourceEdge>,
        mut edge: RoadSurfaceTerrainClipSourceEdge,
    ) {
        if Self::world_points_same_for_boundary(edge.start, edge.end) {
            return;
        }
        if let Some(last) = out.last_mut()
            && Self::world_points_same_for_boundary(last.end, edge.start)
        {
            let shared = if edge.start.y > last.end.y {
                edge.start
            } else {
                last.end
            };
            last.end = shared;
            edge.start = shared;
        }
        out.push(edge);
    }

    fn terrain_clip_output_source_for_segment(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSourceEdge> {
        let mut best = None;
        for &source_edge in source_edges {
            let interval = Self::terrain_clip_source_interval_on_segment(start, end, source_edge)?;
            if !Self::terrain_clip_interval_covers(interval, 0.0, 1.0) {
                continue;
            }
            let height = interval_height_at(interval, 0.5);
            if best.is_none_or(|(best_height, best_edge): (f32, TerrainClipSourceEdge)| {
                height > best_height
                    || (Self::overlay_heights_equal(height, best_height)
                        && terrain_clip_source_edge_ordering(source_edge, best_edge).is_lt())
            }) {
                best = Some((height, source_edge));
            }
        }
        best.map(|(_, source_edge)| source_edge)
    }

    fn terrain_clip_output_source_for_endpoint_segment(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSourceEdge> {
        let start_key = Self::terrain_clip_overlay_key(start);
        let end_key = Self::terrain_clip_overlay_key(end);
        let mut candidates = source_edges
            .iter()
            .copied()
            .filter(|source_edge| {
                let source_start_key = Self::terrain_clip_world_key(source_edge.start);
                let source_end_key = Self::terrain_clip_world_key(source_edge.end);
                (source_start_key == start_key && source_end_key == end_key)
                    || (source_start_key == end_key && source_end_key == start_key)
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| terrain_clip_source_edge_ordering(*a, *b));
        candidates.into_iter().next()
    }

    fn terrain_clip_output_dust_connector_source(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSourceEdge> {
        let mut endpoint_edges =
            Self::terrain_clip_source_edges_at_overlay_point(start, source_edges);
        endpoint_edges.extend(Self::terrain_clip_source_edges_at_overlay_point(
            end,
            source_edges,
        ));
        endpoint_edges.sort_by(|a, b| terrain_clip_source_edge_ordering(*a, *b));
        endpoint_edges.dedup_by(|a, b| terrain_clip_source_edge_ordering(*a, *b).is_eq());
        endpoint_edges.into_iter().next()
    }

    fn terrain_clip_source_edges_at_overlay_point(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<TerrainClipSourceEdge> {
        source_edges
            .iter()
            .copied()
            .filter(|source_edge| {
                let source_start = [
                    f64::from(source_edge.start.x),
                    f64::from(source_edge.start.z),
                ];
                let source_end = [f64::from(source_edge.end.x), f64::from(source_edge.end.z)];
                Self::overlay_segment_parameter(point, source_start, source_end).is_some()
            })
            .collect()
    }
}
