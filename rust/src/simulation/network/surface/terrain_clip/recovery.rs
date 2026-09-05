//! Terrain-clip segment and source-chain recovery.

use super::super::backend::RoadVec3;
use super::super::{
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayPoint, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceSystem, keys::SurfaceXzKey,
};
use super::geometry::{interpolate_height_f64, overlay_segment_length_m};
use super::model::*;
use std::collections::{BTreeMap, BTreeSet};

const TERRAIN_CLIP_SOURCE_CHAIN_MAX_DETOUR_RATIO: f64 = 1.75;

pub(super) enum TerrainClipSourceChainRecovery {
    Missing,
    Ambiguous(String),
    Covered(Vec<RoadVec3>),
}

#[derive(Clone, Copy)]
struct TerrainClipSourceLoopAnchor {
    edge_position: usize,
    t: f64,
    point: RoadVec3,
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

        let samples = Self::terrain_clip_prepared_sources_on_segment(start, end, source_edges);
        Self::terrain_clip_segment_points_from_prepared_sources(start, end, &samples)
    }

    pub(super) fn terrain_clip_prepared_sources_on_segment(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<TerrainClipPreparedSource> {
        let mut samples = Vec::with_capacity(source_edges.len());
        for &source_edge in source_edges {
            if let Some(interval) =
                Self::terrain_clip_source_interval_on_segment(start, end, source_edge)
            {
                samples.push(TerrainClipPreparedSource {
                    edge: source_edge,
                    interval,
                });
            }
        }
        samples
    }

    pub(super) fn terrain_clip_segment_points_from_prepared_sources(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        sources: &[TerrainClipPreparedSource],
    ) -> TerrainClipSegmentPointRecovery {
        Self::terrain_clip_top_envelope_points_from_prepared_sources(start, end, sources)
    }

    pub(super) fn terrain_clip_source_chain_points_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> TerrainClipSourceChainRecovery {
        if Self::terrain_clip_overlay_key(start) == Self::terrain_clip_overlay_key(end) {
            return TerrainClipSourceChainRecovery::Missing;
        }

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

            let start_anchors =
                Self::terrain_clip_source_loop_anchors_at_point(start, &source_chain_edges);
            let end_anchors =
                Self::terrain_clip_source_loop_anchors_at_point(end, &source_chain_edges);
            for start_anchor in start_anchors {
                for end_anchor in end_anchors.iter().copied() {
                    if start_anchor.edge_position == end_anchor.edge_position
                        && (start_anchor.t - end_anchor.t).abs() <= f64::EPSILON
                    {
                        continue;
                    }
                    let Some(mut points) = Self::terrain_clip_ordered_source_loop_point_path(
                        &source_chain_edges,
                        start_anchor,
                        end_anchor,
                    ) else {
                        continue;
                    };
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
            0 => Self::terrain_clip_endpoint_owner_connector_points_from_source_edges(
                start,
                end,
                source_edges,
            ),
            1 => TerrainClipSourceChainRecovery::Covered(
                candidates.into_values().next().unwrap_or_default(),
            ),
            _ => TerrainClipSourceChainRecovery::Ambiguous(format!(
                "ambiguous_source_chain candidates={}",
                candidates.len()
            )),
        }
    }

    fn terrain_clip_endpoint_owner_connector_points_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> TerrainClipSourceChainRecovery {
        let start_y = match Self::terrain_clip_unambiguous_overlay_point_height_from_source_edges(
            start,
            source_edges,
        ) {
            Ok(Some(height)) => height,
            Ok(None) => return TerrainClipSourceChainRecovery::Missing,
            Err(context) => {
                return TerrainClipSourceChainRecovery::Ambiguous(format!(
                    "endpoint_owner_connector_start_{context}"
                ));
            }
        };
        let end_y = match Self::terrain_clip_unambiguous_overlay_point_height_from_source_edges(
            end,
            source_edges,
        ) {
            Ok(Some(height)) => height,
            Ok(None) => return TerrainClipSourceChainRecovery::Missing,
            Err(context) => {
                return TerrainClipSourceChainRecovery::Ambiguous(format!(
                    "endpoint_owner_connector_end_{context}"
                ));
            }
        };
        let start_point = RoadVec3::new(start[0], start_y, start[1]);
        let end_point = RoadVec3::new(end[0], end_y, end[1]);
        let start_candidates =
            Self::explicit_node_boundary_endpoint_sources(start_point, source_edges);
        let end_candidates = Self::explicit_node_boundary_endpoint_sources(end_point, source_edges);
        if start_candidates.is_empty() || end_candidates.is_empty() {
            return TerrainClipSourceChainRecovery::Missing;
        }

        let Some(start_source) = Self::canonical_same_owner_dust_connector_output_source(
            &start_candidates,
            start_point,
            end_point,
        ) else {
            return TerrainClipSourceChainRecovery::Missing;
        };
        let Some(end_source) = Self::canonical_same_owner_dust_connector_output_source(
            &end_candidates,
            start_point,
            end_point,
        ) else {
            return TerrainClipSourceChainRecovery::Missing;
        };
        if !terrain_clip_source_edges_same_provenance(start_source, end_source) {
            return TerrainClipSourceChainRecovery::Missing;
        }

        let mut combined_candidates = start_candidates;
        combined_candidates.extend(end_candidates);
        let Some(combined_source) = Self::canonical_same_owner_dust_connector_output_source(
            &combined_candidates,
            start_point,
            end_point,
        ) else {
            return TerrainClipSourceChainRecovery::Missing;
        };
        if !terrain_clip_source_edges_same_provenance(start_source, combined_source) {
            return TerrainClipSourceChainRecovery::Missing;
        }

        TerrainClipSourceChainRecovery::Covered(vec![start_point, end_point])
    }

    fn explicit_node_boundary_endpoint_sources(
        point: RoadVec3,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<TerrainClipSourceEdge> {
        Self::terrain_clip_source_edges_at_world_xz_point(point, source_edges)
            .into_iter()
            .filter(|edge| {
                matches!(
                    edge.source,
                    RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                        boundary_source: Some(_),
                        ..
                    }
                )
            })
            .collect()
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

    fn terrain_clip_source_loop_anchors_at_point(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<TerrainClipSourceLoopAnchor> {
        let mut anchors = Vec::new();
        for (edge_position, source_edge) in source_edges.iter().copied().enumerate() {
            let source_start = [source_edge.start.x, source_edge.start.z];
            let source_end = [source_edge.end.x, source_edge.end.z];
            let Some(t) = Self::overlay_segment_parameter(point, source_start, source_end) else {
                continue;
            };
            let point = RoadVec3::new(
                point[0],
                interpolate_height_f64(source_edge.start.y, source_edge.end.y, t),
                point[1],
            );
            anchors.push(TerrainClipSourceLoopAnchor {
                edge_position,
                t,
                point,
            });
        }
        anchors
    }

    fn terrain_clip_ordered_source_loop_point_path(
        source_edges: &[TerrainClipSourceEdge],
        start_anchor: TerrainClipSourceLoopAnchor,
        end_anchor: TerrainClipSourceLoopAnchor,
    ) -> Option<Vec<RoadVec3>> {
        Self::terrain_clip_ordered_source_loop_point_path_in_direction(
            source_edges,
            start_anchor,
            end_anchor,
            1,
        )
        .filter(|points| {
            Self::terrain_clip_source_chain_path_is_local_to_segment(
                start_anchor.point,
                end_anchor.point,
                points,
            )
        })
        .or_else(|| {
            Self::terrain_clip_ordered_source_loop_point_path_in_direction(
                source_edges,
                start_anchor,
                end_anchor,
                -1,
            )
            .filter(|points| {
                Self::terrain_clip_source_chain_path_is_local_to_segment(
                    start_anchor.point,
                    end_anchor.point,
                    points,
                )
            })
        })
    }

    fn terrain_clip_ordered_source_loop_point_path_in_direction(
        source_edges: &[TerrainClipSourceEdge],
        start_anchor: TerrainClipSourceLoopAnchor,
        end_anchor: TerrainClipSourceLoopAnchor,
        direction: i8,
    ) -> Option<Vec<RoadVec3>> {
        if source_edges.is_empty() || start_anchor.edge_position >= source_edges.len() {
            return None;
        }
        let mut path = vec![start_anchor.point];
        let mut cursor = start_anchor.edge_position;
        let mut first_edge = true;
        for _ in 0..=source_edges.len() {
            if cursor == end_anchor.edge_position {
                let same_edge_progresses_toward_end = if direction >= 0 {
                    end_anchor.t > start_anchor.t
                } else {
                    end_anchor.t < start_anchor.t
                };
                if !first_edge || same_edge_progresses_toward_end {
                    path.push(end_anchor.point);
                    return (path.len() >= 2).then_some(path);
                }
                if (start_anchor.t - end_anchor.t).abs() <= f64::EPSILON {
                    return None;
                }
            }
            let edge = source_edges[cursor % source_edges.len()];
            if direction >= 0 {
                path.push(edge.end);
                if !Self::terrain_clip_source_loop_edge_connects_to_next(source_edges, cursor) {
                    return None;
                }
                cursor = (cursor + 1) % source_edges.len();
            } else {
                path.push(edge.start);
                if !Self::terrain_clip_source_loop_edge_connects_to_previous(source_edges, cursor) {
                    return None;
                }
                cursor = (cursor + source_edges.len() - 1) % source_edges.len();
            }
            first_edge = false;
        }
        None
    }

    fn terrain_clip_source_chain_path_is_local_to_segment(
        start: RoadVec3,
        end: RoadVec3,
        path: &[RoadVec3],
    ) -> bool {
        let chord_length_m = overlay_segment_length_m([start.x, start.z], [end.x, end.z]);
        if chord_length_m <= f64::EPSILON {
            return false;
        }
        let path_length_m = path
            .windows(2)
            .map(|segment| {
                let dx = segment[1].x - segment[0].x;
                let dz = segment[1].z - segment[0].z;
                (dx * dx + dz * dz).sqrt()
            })
            .sum::<f64>();
        path_length_m
            <= chord_length_m * TERRAIN_CLIP_SOURCE_CHAIN_MAX_DETOUR_RATIO
                + f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
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

    fn terrain_clip_source_loop_edge_connects_to_previous(
        source_edges: &[TerrainClipSourceEdge],
        position: usize,
    ) -> bool {
        let Some(edge) = source_edges.get(position % source_edges.len()) else {
            return false;
        };
        let previous_position = (position + source_edges.len() - 1) % source_edges.len();
        let Some(previous_edge) = source_edges.get(previous_position) else {
            return false;
        };
        Self::terrain_clip_world_key(edge.start) == Self::terrain_clip_world_key(previous_edge.end)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::{
        RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind,
    };

    #[test]
    fn source_chain_recovery_walks_reverse_when_forward_chain_is_open() {
        let y = 7.0;
        let p0 = RoadVec3::new(0.0, y, 0.0);
        let p1 = RoadVec3::new(0.35, y, 0.0);
        let p2 = RoadVec3::new(0.55, y, 0.18);
        let p3 = RoadVec3::new(1.0, y, 0.0);
        let source_edges = [p0, p1, p2]
            .into_iter()
            .zip([p1, p2, p3])
            .enumerate()
            .map(|(edge_index, (start, end))| source_edge(start, end, edge_index))
            .collect::<Vec<_>>();

        let recovered = RoadSurfaceSystem::terrain_clip_source_chain_points_from_source_edges(
            [p3.x, p3.z],
            [p0.x, p0.z],
            &source_edges,
        );

        let TerrainClipSourceChainRecovery::Covered(points) = recovered else {
            panic!("reverse source chain should recover through connected source edges");
        };
        assert_eq!(
            points
                .iter()
                .map(|point| RoadSurfaceSystem::terrain_clip_world_key(*point))
                .collect::<Vec<_>>(),
            [p3, p2, p1, p0]
                .into_iter()
                .map(RoadSurfaceSystem::terrain_clip_world_key)
                .collect::<Vec<_>>()
        );
    }

    fn source_edge(start: RoadVec3, end: RoadVec3, edge_index: usize) -> TerrainClipSourceEdge {
        TerrainClipSourceEdge {
            start,
            end,
            kind: RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
            source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id: 0,
                kind: RoadSurfaceVisualNodePieceKind::Terminal,
                owner_kind: RoadSurfaceBandKind::Sidewalk,
                owner_index: 0,
                boundary_source: None,
            },
            source_index: 0,
            edge_index,
        }
    }
}
