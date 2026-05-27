//! Source-edge extraction and diagnostics for terrain-clip union output.

use super::super::backend::RoadVec3;
use super::super::{
    NodeOverlayPoint, RoadSurfaceSystem,
    keys::{SurfaceHeightMmKey, SurfaceXzKey, SurfaceXzSegmentKey},
};
use super::geometry::{interpolate_height_f64, interpolate_overlay_point};
use super::model::*;
use std::collections::BTreeMap;

impl RoadSurfaceSystem {
    pub(super) fn terrain_clip_source_edges_from_boundary_loops(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Vec<TerrainClipSourceEdge> {
        let mut edges = Vec::new();
        for (source_index, boundary_loop) in boundary_loops.iter().enumerate() {
            for (edge_index, source_edge) in boundary_loop.source_edges.iter().copied().enumerate()
            {
                let start = Self::terrain_clip_canonical_loop_point(
                    source_edge.start,
                    &boundary_loop.points_world,
                );
                let end = Self::terrain_clip_canonical_loop_point(
                    source_edge.end,
                    &boundary_loop.points_world,
                );
                if Self::terrain_clip_world_key(start) == Self::terrain_clip_world_key(end) {
                    continue;
                }
                edges.push(TerrainClipSourceEdge {
                    start,
                    end,
                    kind: source_edge.kind,
                    source: source_edge.source,
                    source_index,
                    edge_index,
                });
            }
        }
        // Keep source endpoints on one canonical coordinate only when they are already the same
        // solved-height seam endpoint after boundary-loop representation cleanup.
        Self::canonicalize_terrain_clip_source_endpoint_groups(&mut edges);

        edges.sort_by_key(|edge| {
            let start_key = Self::terrain_clip_world_key(edge.start);
            let end_key = Self::terrain_clip_world_key(edge.end);
            let edge_key = SurfaceXzSegmentKey::new(start_key, end_key);
            (
                edge_key.start(),
                edge_key.end(),
                terrain_clip_edge_kind_priority(edge.kind),
                Self::overlay_height_key(edge.start.y),
                Self::overlay_height_key(edge.end.y),
                edge.source_index,
                edge.edge_index,
            )
        });
        edges
    }

    fn canonicalize_terrain_clip_source_endpoint_groups(edges: &mut [TerrainClipSourceEdge]) {
        let mut groups: Vec<Vec<TerrainClipSourceEndpointKey>> = Vec::new();
        for point in edges.iter().flat_map(|edge| [edge.start, edge.end]) {
            let point_key = Self::terrain_clip_source_endpoint_key(point);
            if let Some(group) = groups.iter_mut().find(|group| {
                group.iter().any(|candidate| {
                    Self::terrain_clip_source_endpoint_keys_match(*candidate, point_key)
                })
            }) {
                group.push(point_key);
            } else {
                groups.push(vec![point_key]);
            }
        }

        let mut replacements = BTreeMap::new();
        for group in groups {
            if group.len() < 2 {
                continue;
            }
            for key in group {
                replacements.insert(key, terrain_clip_point_from_source_endpoint_key(key));
            }
        }

        for edge in edges {
            if let Some(point) =
                replacements.get(&Self::terrain_clip_source_endpoint_key(edge.start))
            {
                edge.start = *point;
            }
            if let Some(point) = replacements.get(&Self::terrain_clip_source_endpoint_key(edge.end))
            {
                edge.end = *point;
            }
        }
    }

    fn terrain_clip_source_endpoint_keys_match(
        a: TerrainClipSourceEndpointKey,
        b: TerrainClipSourceEndpointKey,
    ) -> bool {
        a == b
    }

    fn terrain_clip_source_endpoint_key(point: RoadVec3) -> TerrainClipSourceEndpointKey {
        let key = Self::terrain_clip_world_key(point);
        TerrainClipSourceEndpointKey {
            x_key: key.x_key(),
            z_key: key.z_key(),
            height_mm: SurfaceHeightMmKey::from_m_f64(point.y).as_i64(),
        }
    }

    fn terrain_clip_canonical_loop_point(point: RoadVec3, loop_points: &[RoadVec3]) -> RoadVec3 {
        loop_points
            .iter()
            .copied()
            .find(|candidate| {
                Self::terrain_clip_world_key(*candidate) == Self::terrain_clip_world_key(point)
                    && Self::overlay_height_key(candidate.y) == Self::overlay_height_key(point.y)
            })
            .unwrap_or(point)
    }

    fn terrain_clip_endpoint_samples(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<TerrainClipEndpointSample> {
        let point_key = Self::terrain_clip_overlay_key(point);
        let mut samples = Vec::new();
        for &source_edge in source_edges {
            if Self::terrain_clip_world_key(source_edge.start) == point_key {
                samples.push(TerrainClipEndpointSample {
                    kind: source_edge.kind,
                    source_index: source_edge.source_index,
                    edge_index: source_edge.edge_index,
                    y: source_edge.start.y,
                });
            }
            if Self::terrain_clip_world_key(source_edge.end) == point_key {
                samples.push(TerrainClipEndpointSample {
                    kind: source_edge.kind,
                    source_index: source_edge.source_index,
                    edge_index: source_edge.edge_index,
                    y: source_edge.end.y,
                });
            }
        }
        samples
    }

    pub(super) fn terrain_clip_missing_source_context_label(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> String {
        format!(
            "start_sources={} end_sources={}",
            Self::terrain_clip_endpoint_context_label(start, source_edges),
            Self::terrain_clip_endpoint_context_label(end, source_edges)
        )
    }

    fn terrain_clip_endpoint_context_label(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> String {
        let mut samples = Self::terrain_clip_endpoint_samples(point, source_edges)
            .into_iter()
            .map(|sample| {
                format!(
                    "{:?}:{}:{}@{:.3}",
                    sample.kind, sample.source_index, sample.edge_index, sample.y
                )
            })
            .collect::<Vec<_>>();
        samples.sort();
        samples.dedup();
        if samples.is_empty() {
            "none".to_string()
        } else {
            samples.into_iter().take(6).collect::<Vec<_>>().join("|")
        }
    }

    pub(super) fn terrain_clip_source_interval_on_segment(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edge: TerrainClipSourceEdge,
    ) -> Option<TerrainClipSourceInterval> {
        let source_start = [source_edge.start.x, source_edge.start.z];
        let source_end = [source_edge.end.x, source_edge.end.z];
        let source_start_t = Self::overlay_line_parameter(source_start, start, end)?;
        let source_end_t = Self::overlay_line_parameter(source_end, start, end)?;
        let mut overlap_start_t = source_start_t.min(source_end_t).max(0.0);
        let mut overlap_end_t = source_start_t.max(source_end_t).min(1.0);
        let endpoint_dust_t = Self::overlay_endpoint_dust_parameter(start, end)?;
        if overlap_start_t <= endpoint_dust_t {
            overlap_start_t = 0.0;
        }
        if overlap_end_t >= 1.0 - endpoint_dust_t {
            overlap_end_t = 1.0;
        }
        if overlap_end_t <= overlap_start_t {
            return None;
        }

        let overlap_start = interpolate_overlay_point(start, end, overlap_start_t);
        let overlap_end = interpolate_overlay_point(start, end, overlap_end_t);
        let edge_overlap_start_t =
            Self::overlay_segment_parameter(overlap_start, source_start, source_end)?;
        let edge_overlap_end_t =
            Self::overlay_segment_parameter(overlap_end, source_start, source_end)?;
        Some(TerrainClipSourceInterval {
            start_t: overlap_start_t,
            end_t: overlap_end_t,
            start_y: interpolate_height_f64(
                source_edge.start.y,
                source_edge.end.y,
                edge_overlap_start_t,
            ),
            end_y: interpolate_height_f64(
                source_edge.start.y,
                source_edge.end.y,
                edge_overlap_end_t,
            ),
        })
    }

    pub(super) fn terrain_clip_top_envelope_source_point_for_vertex_key(
        key: SurfaceXzKey,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<RoadVec3> {
        // This top-envelope pick is scoped to terrain-removal cutter recovery after
        // final footprint ownership exists. Output provenance and dust connectors
        // must use their ambiguity-checking paths instead.
        source_edges
            .iter()
            .flat_map(|edge| [edge.start, edge.end])
            .filter(|point| Self::terrain_clip_world_key(*point) == key)
            .max_by(|a, b| a.y.total_cmp(&b.y))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TerrainClipSourceEndpointKey {
    x_key: i64,
    z_key: i64,
    height_mm: i64,
}

fn terrain_clip_point_from_source_endpoint_key(key: TerrainClipSourceEndpointKey) -> RoadVec3 {
    RoadVec3::new(
        key.x_key as f64 / super::super::keys::SURFACE_XZ_KEY_SCALE,
        key.height_mm as f64 / 1000.0,
        key.z_key as f64 / super::super::keys::SURFACE_XZ_KEY_SCALE,
    )
}
