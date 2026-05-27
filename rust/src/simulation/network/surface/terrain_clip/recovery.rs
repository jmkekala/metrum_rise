//! Terrain-clip segment and source-chain recovery.

use super::super::backend::RoadVec3;
use super::super::{NodeOverlayPoint, RoadSurfaceSystem, keys::SurfaceXzKey};
use super::geometry::interpolate_height_f64;
use super::model::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) enum TerrainClipSourceChainRecovery {
    Missing,
    Ambiguous(String),
    Covered(Vec<RoadVec3>),
}

impl RoadSurfaceSystem {
    pub(super) fn terrain_clip_segment_heights_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSegmentHeights> {
        let TerrainClipSegmentPointRecovery::Covered(points) =
            Self::terrain_clip_segment_points_from_source_edges(start, end, source_edges)
        else {
            return None;
        };
        Some(TerrainClipSegmentHeights {
            start_y: points.first()?.y,
            end_y: points.last()?.y,
        })
    }

    pub(super) fn terrain_clip_segment_points_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> TerrainClipSegmentPointRecovery {
        if Self::terrain_clip_overlay_key(start) == Self::terrain_clip_overlay_key(end) {
            return TerrainClipSegmentPointRecovery::Degenerate;
        }

        let mut samples = Vec::new();
        for &source_edge in source_edges {
            if let Some(interval) =
                Self::terrain_clip_source_interval_on_segment(start, end, source_edge)
            {
                samples.push(interval);
            }
        }
        Self::terrain_clip_top_envelope_points_from_interval_coverage(start, end, samples)
    }

    pub(super) fn terrain_clip_source_chain_points_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> TerrainClipSourceChainRecovery {
        if Self::terrain_clip_overlay_key(start) == Self::terrain_clip_overlay_key(end) {
            return TerrainClipSourceChainRecovery::Missing;
        }

        let start_key = Self::terrain_clip_overlay_key(start);
        let end_key = Self::terrain_clip_overlay_key(end);
        let source_indices = source_edges
            .iter()
            .map(|edge| edge.source_index)
            .collect::<BTreeSet<_>>();
        let mut candidates = BTreeMap::<Vec<(i64, i64, i64)>, Vec<RoadVec3>>::new();
        for source_index in source_indices.iter().copied() {
            let mut source_chain_edges = source_edges
                .iter()
                .copied()
                .filter(|edge| edge.source_index == source_index)
                .collect::<Vec<_>>();
            source_chain_edges.sort_by_key(|edge| edge.edge_index);
            if source_chain_edges.len() < 2 {
                continue;
            }

            let start_positions =
                Self::terrain_clip_source_loop_positions_at_key(start_key, &source_chain_edges);
            let end_positions =
                Self::terrain_clip_source_loop_positions_at_key(end_key, &source_chain_edges);
            for start_position in start_positions {
                for end_position in end_positions.iter().copied() {
                    if start_position == end_position {
                        continue;
                    }
                    let Some(path_keys) = Self::terrain_clip_ordered_source_loop_key_path(
                        &source_chain_edges,
                        start_position,
                        end_position,
                    ) else {
                        continue;
                    };
                    let mut points = path_keys
                        .into_iter()
                        .filter_map(|key| {
                            Self::terrain_clip_top_envelope_source_point_for_vertex_key(
                                key,
                                source_edges,
                            )
                        })
                        .collect::<Vec<_>>();
                    Self::apply_terrain_clip_source_chain_top_envelope_heights(
                        &mut points,
                        source_edges,
                    );
                    Self::dedup_terrain_clip_top_envelope_points(&mut points);
                    if points.len() < 2 {
                        continue;
                    }
                    candidates
                        .entry(Self::terrain_clip_source_chain_point_identity(&points))
                        .or_insert(points);
                }
            }
        }

        match candidates.len() {
            0 => TerrainClipSourceChainRecovery::Missing,
            1 => TerrainClipSourceChainRecovery::Covered(
                candidates.into_values().next().unwrap_or_default(),
            ),
            _ => TerrainClipSourceChainRecovery::Ambiguous(format!(
                "ambiguous_source_chain candidates={}",
                candidates.len()
            )),
        }
    }

    fn terrain_clip_source_chain_point_identity(points: &[RoadVec3]) -> Vec<(i64, i64, i64)> {
        points
            .iter()
            .map(|point| {
                let key = Self::terrain_clip_world_key(*point);
                (key.x_key(), key.z_key(), Self::overlay_height_key(point.y))
            })
            .collect()
    }

    fn terrain_clip_source_loop_positions_at_key(
        key: SurfaceXzKey,
        source_edges: &[TerrainClipSourceEdge],
    ) -> BTreeSet<usize> {
        let mut positions = BTreeSet::new();
        if source_edges.is_empty() {
            return positions;
        }
        for (position, source_edge) in source_edges.iter().copied().enumerate() {
            let start_key = Self::terrain_clip_world_key(source_edge.start);
            if start_key == key {
                positions.insert(position);
            }
            let end_key = Self::terrain_clip_world_key(source_edge.end);
            if end_key == key {
                positions.insert((position + 1) % source_edges.len());
            }
        }
        positions
    }

    fn terrain_clip_ordered_source_loop_key_path(
        source_edges: &[TerrainClipSourceEdge],
        start_position: usize,
        end_position: usize,
    ) -> Option<Vec<SurfaceXzKey>> {
        if source_edges.is_empty() || start_position >= source_edges.len() {
            return None;
        }
        let mut path = vec![Self::terrain_clip_source_loop_vertex_key(
            source_edges,
            start_position,
        )?];
        let mut cursor = start_position;
        for _ in 0..source_edges.len() {
            if cursor == end_position {
                return (path.len() >= 2).then_some(path);
            }
            if !Self::terrain_clip_source_loop_edge_connects_to_next(source_edges, cursor) {
                return None;
            }
            cursor = (cursor + 1) % source_edges.len();
            path.push(Self::terrain_clip_source_loop_vertex_key(
                source_edges,
                cursor,
            )?);
        }
        None
    }

    fn terrain_clip_source_loop_vertex_key(
        source_edges: &[TerrainClipSourceEdge],
        position: usize,
    ) -> Option<SurfaceXzKey> {
        let source_edge = source_edges.get(position % source_edges.len())?;
        Some(Self::terrain_clip_world_key(source_edge.start))
    }

    fn terrain_clip_source_loop_edge_connects_to_next(
        source_edges: &[TerrainClipSourceEdge],
        position: usize,
    ) -> bool {
        let Some(edge) = source_edges.get(position % source_edges.len()) else {
            return false;
        };
        let Some(next_vertex_key) =
            Self::terrain_clip_source_loop_vertex_key(source_edges, position + 1)
        else {
            return false;
        };
        Self::terrain_clip_world_key(edge.end) == next_vertex_key
    }

    fn apply_terrain_clip_source_chain_top_envelope_heights(
        points: &mut [RoadVec3],
        source_edges: &[TerrainClipSourceEdge],
    ) {
        for point in points {
            let overlay_point = [point.x, point.z];
            if let Some(height) =
                Self::terrain_clip_source_chain_top_envelope_height_from_source_edges(
                    overlay_point,
                    source_edges,
                )
                && height > point.y
            {
                point.y = height;
            }
        }
    }

    fn terrain_clip_source_chain_top_envelope_height_from_source_edges(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<f64> {
        let mut top_height = None;
        for &source_edge in source_edges {
            let source_start = [source_edge.start.x, source_edge.start.z];
            let source_end = [source_edge.end.x, source_edge.end.z];
            let Some(t) = Self::overlay_segment_parameter(point, source_start, source_end) else {
                continue;
            };
            let candidate = interpolate_height_f64(source_edge.start.y, source_edge.end.y, t);
            if top_height.is_none_or(|height| candidate > height) {
                top_height = Some(candidate);
            }
        }
        top_height
    }
}
